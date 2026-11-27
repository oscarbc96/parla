//! Install, disable and uninstall. No UI: the install dialog shows a
//! [`ConsentRequest`] and hands its grant back to [`Installer::commit`].
//!
//! - Staging: [`Installer::stage_from_file`] (source `file`, unverified)
//!   and [`Installer::stage_from_bytes`] (source `registry`, SHA-256 from
//!   the index) unpack into a hidden `.staging-*` directory inside the
//!   plugins folder, so the commit is one rename on the same file system.
//!   Dropping a [`Staged`] removes it.
//! - Commit: a `verified` row is written first, then the folder moves to
//!   `plugins/<id>/<version>/`, the grant goes to `app.db`,
//!   `plugins.<id>.enabled = true` to `settings.json`, and the row turns
//!   `installed`; then the contributions are registered and a background
//!   precompile is queued. Nothing is instantiated. A failure undoes the
//!   earlier steps; a crash leaves a `verified` row, which the next
//!   [`Installer::load`] rolls back (folder and row).
//! - The grant is what the user accepted, read from `app.db`; the files on
//!   disk never widen it. The digest of the installed files
//!   ([`files_digest`]) is kept on the row and checked by every load; a
//!   changed package refuses to load. No source skips the check today
//!   ([`DIGEST_EXEMPT_SOURCES`] is the allow-list dev plugins will join).
//! - Uninstall moves the folder aside, removes the rows (store first), then
//!   the keychain secrets ([`PluginSecrets`], C2). "Keep its data" leaves a
//!   [`KEPT_DATA_ROW`] row holding only the storage; the next install of
//!   the same id from the same source and author takes it over, any other
//!   install drops it and the old settings entry first.
//! - A row that fails to load (changed on disk, unreadable, folder gone) can
//!   be uninstalled, or installed over, from the row alone.
//! - The lifecycle (B5) runs through [`Installer::apply`]: each transition
//!   is persisted before it takes effect in memory, and its host effects
//!   (close views, unload, banners) are returned to the caller.
//!
//! Filesystem work is blocking: call these off the UI thread.
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use semver::Version;
use sha2::{Digest, Sha256};
use store::{AppStore, GrantRow, PluginRow, StoreError, WriteAllow};

use crate::caps::{CardRow, ExecGrant, Grant, card_rows};
use crate::lifecycle::{
    BlockEntry, Discovered, Effect, Event, KEPT_DATA_ROW, Lifecycle, PluginState, discover,
    is_kept_data, transition,
};
use crate::manifest::{
    self, DEFAULT_MEMORY_MB, LocalisedText, Manifest, ManifestSource, NetworkHost, PluginId,
    ReadArea, WriteArea,
};
use crate::package::{self, Package, PackageError, PackageSource};
use crate::registry::InstalledSet;

/// Sources whose files are not digest-checked on load. Empty: every
/// source is checked (dev plugins, which are edited in place, join later).
pub const DIGEST_EXEMPT_SOURCES: &[&str] = &[];
/// `plugins.state` while a commit is in flight (the §8.1 Verified state).
pub const INSTALLING_ROW: &str = "verified";
/// Prefix of a folder moved aside by an uninstall or a reinstall.
const REMOVING_PREFIX: &str = ".removing-";
/// Prefix of a staging folder ([`Package::open`]).
const STAGING_PREFIX: &str = ".staging-";

/// Where an installed plugin came from (`plugins.source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallSource {
    /// The plugin index; checksum verified.
    Registry,
    /// "Install from file…" (§13); unverified, never auto-updated.
    File,
}

impl InstallSource {
    pub fn as_row(self) -> &'static str {
        match self {
            Self::Registry => "registry",
            Self::File => "file",
        }
    }
}

/// Writes the user's `plugins.<id>` preferences.
pub trait SettingsWriter: Send + Sync {
    /// Sets `plugins.<id>.enabled`.
    fn set_enabled(&self, id: &str, enabled: bool) -> io::Result<()>;
    /// Replaces the whole `plugins.<id>` entry with `{ "enabled": true }`,
    /// dropping another plugin's config under the same id.
    fn reset(&self, id: &str) -> io::Result<()>;
}

/// [`SettingsWriter`] over a `settings.json` file (comment-preserving,
/// atomic rename).
pub struct SettingsFile(pub PathBuf);

impl SettingsFile {
    fn edit(&self, path: &[&str], value: &str) -> io::Result<()> {
        if settings::edit_file(&self.0, path, value)? {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "settings.json is not a JSON object",
            ))
        }
    }
}

impl SettingsWriter for SettingsFile {
    fn set_enabled(&self, id: &str, enabled: bool) -> io::Result<()> {
        let value = if enabled { "true" } else { "false" };
        self.edit(&["plugins", id, "enabled"], value)
    }
    fn reset(&self, id: &str) -> io::Result<()> {
        self.edit(&["plugins", id], r#"{ "enabled": true }"#)
    }
}

/// Removes a plugin's keychain secrets on uninstall. The app implements it
/// with the secrets host.
pub trait PluginSecrets: Send + Sync {
    fn remove_all(&self, id: &PluginId) -> Result<(), String>;
}

/// Background `.cwasm` work. The app runs [`crate::cache::PrecompileCache`]
/// at low priority; nothing here waits for it.
pub trait PrecompileQueue: Send + Sync {
    /// Precompile `wasm` for `id` at `version`, later.
    fn queue(&self, id: &PluginId, version: &str, wasm: PathBuf);
    /// Drop the `.cwasm` files and hashes of `id` at `version`.
    fn forget(&self, id: &PluginId, version: &str);
}

/// The registry blocklist (§8.7), from the last good index.
pub trait Blocklist: Send + Sync {
    /// The entry naming this plugin (id, version or package hash), if any.
    fn entry(&self, row: &PluginRow) -> Option<BlockEntry>;
}

/// A [`Blocklist`] that names nothing (no index yet).
pub struct NoBlocklist;

impl Blocklist for NoBlocklist {
    fn entry(&self, _row: &PluginRow) -> Option<BlockEntry> {
        None
    }
}

/// Resolves an `exec` program (§7.4) at consent and again at commit.
///
/// Contract: returns the absolute, canonical path (symlinks resolved) of an
/// executable regular file, or `None`. A bare name is looked up on the
/// login `PATH`, skipping empty, `.` and other relative entries; a name with
/// a separator must already be absolute. [`resolve_on_path`] implements it;
/// the app passes the login shell's `PATH`. Commit refuses a relative
/// answer.
pub type ExecResolver = Box<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

/// [`ExecResolver`]'s contract over the `PATH` value `path`.
pub fn resolve_on_path(program: &str, path: &OsStr) -> Option<PathBuf> {
    fn executable(p: &Path) -> Option<PathBuf> {
        let c = fs::canonicalize(p).ok()?;
        let md = fs::metadata(&c).ok()?;
        if !md.is_file() {
            return None;
        }
        #[cfg(unix)]
        if std::os::unix::fs::PermissionsExt::mode(&md.permissions()) & 0o111 == 0 {
            return None;
        }
        Some(c)
    }
    let p = Path::new(program);
    if program.is_empty() {
        return None;
    }
    if p.components().count() > 1 || p.is_absolute() {
        return if p.is_absolute() { executable(p) } else { None };
    }
    std::env::split_paths(path)
        .filter(|d| d.is_absolute())
        .find_map(|d| executable(&d.join(program)))
}

/// What an [`Installer`] works with.
pub struct InstallerConfig {
    /// `$CONFIG/parla/plugins`.
    pub plugins_dir: PathBuf,
    /// Checked against a manifest's `min_app`.
    pub app_version: Version,
    pub store: Arc<dyn AppStore>,
    pub settings: Arc<dyn SettingsWriter>,
    pub secrets: Arc<dyn PluginSecrets>,
    pub precompile: Arc<dyn PrecompileQueue>,
    pub blocklist: Arc<dyn Blocklist>,
    pub resolve_exec: ExecResolver,
}

/// Why an install step failed. Nothing was installed or removed.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error(transparent)]
    Package(#[from] PackageError),
    #[error("needs Parla {min} or later")]
    NeedsNewerApp { min: Version },
    #[error("the accepted permissions do not match the plugin")]
    GrantMismatch,
    #[error("`{0}` was not found on the PATH")]
    ExecNotFound(String),
    #[error("the plugin index blocks this plugin: {}", .0.reason)]
    Blocked(BlockEntry),
    #[error("{id} is already installed")]
    AlreadyInstalled { id: PluginId },
    #[error("the plugin is not installed")]
    NotInstalled,
    #[error("the plugins folder already has `{0}`")]
    FolderExists(PathBuf),
    #[error("could not write plugin files: {0}")]
    Io(#[from] io::Error),
    #[error("could not save settings: {0}")]
    Settings(io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Staging folders owned by a live [`Staged`] in this process; a load never
/// sweeps them. Bounded by the live [`Staged`] values.
type LiveStaging = Arc<Mutex<HashSet<PathBuf>>>;

/// Unregisters a staging folder when its [`Staged`] drops.
#[derive(Debug)]
struct LiveGuard {
    set: LiveStaging,
    path: PathBuf,
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        lock(&self.set).remove(&self.path);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A verified package in its staging folder, waiting for consent.
/// Dropping it removes the folder.
#[derive(Debug)]
pub struct Staged {
    package: package::Staged,
    source: InstallSource,
    toml: String,
    _live: LiveGuard,
}

impl Staged {
    pub fn manifest(&self) -> &Manifest {
        &self.package.manifest
    }
    pub fn source(&self) -> InstallSource {
        self.source
    }
    pub fn sha256(&self) -> [u8; 32] {
        self.package.sha256
    }
    fn authors(&self) -> String {
        self.package.manifest.authors.join("\n")
    }
}

/// What the install dialog shows (§8.2 step 4, §13).
#[derive(Clone, Debug, PartialEq)]
pub struct ConsentRequest {
    pub summary: Summary,
    /// Empty for a data plugin ("No code runs").
    pub rows: Vec<CardRow>,
    pub source: SourceChip,
    pub memory: MemoryNote,
    /// The read-only `plugin.toml` beside the card.
    pub toml: String,
    /// Data kept from an earlier install of this id, same source and author,
    /// is reused ("Its saved data from before is kept").
    pub reuses_data: bool,
    /// What Install accepts: built from the manifest, `exec` paths
    /// resolved now. Pass it to [`Installer::commit`].
    pub grant: Grant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub id: PluginId,
    pub name: LocalisedText,
    pub version: Version,
    pub authors: Vec<String>,
    pub repository: Option<String>,
    /// Themes or icons only.
    pub data: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceChip {
    /// "Checksum verified" (registry).
    ChecksumVerified,
    /// "Unverified file", warning colour (§13).
    UnverifiedFile,
}

/// The sandbox note's memory cap; `above_default` adds the Low-risk row
/// "Uses up to N MB of memory".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryNote {
    pub cap_mb: u32,
    pub above_default: bool,
}

/// What [`Installer::load`] did with one plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// In the installed set in this state.
    Loaded(PluginId, PluginState),
    /// Its files differ from what was installed: not loaded.
    ChangedOnDisk(PluginId),
    /// Its manifest cannot be read or no longer parses: not loaded.
    Unreadable(PluginId, String),
    /// A folder without a row (§14 "Found in the plugins folder").
    FoundNotInstalled(String),
    /// A row whose folder is gone.
    MissingFolder(String),
}

/// What [`Installer::uninstall`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uninstalled {
    /// For the host to run (close views, unload).
    pub effects: Vec<Effect>,
    /// The plugin is gone but its keychain secrets could not be removed.
    pub secrets_error: Option<String>,
}

struct Plugin {
    row: PluginRow,
    lifecycle: Lifecycle,
    grant: Grant,
}

/// An earlier row for the id being installed.
enum Previous {
    None,
    /// Uninstalled with "Keep its data", or a row that failed to load.
    Replaceable {
        row: PluginRow,
        broken: bool,
    },
}

/// The install pipeline and the installed plugins' state. Bounded by the
/// installed plugins.
pub struct Installer {
    cfg: InstallerConfig,
    set: InstalledSet,
    plugins: BTreeMap<PluginId, Plugin>,
    /// Rows the last load could not load; they can be uninstalled or
    /// installed over.
    broken: BTreeSet<PluginId>,
    live_staging: LiveStaging,
}

impl Installer {
    pub fn new(cfg: InstallerConfig) -> Self {
        Self {
            cfg,
            set: InstalledSet::default(),
            plugins: BTreeMap::new(),
            broken: BTreeSet::new(),
            live_staging: LiveStaging::default(),
        }
    }

    /// Installed plugins and their registered contributions.
    pub fn installed(&self) -> &InstalledSet {
        &self.set
    }
    pub fn lifecycle(&self, id: &PluginId) -> Option<&Lifecycle> {
        self.plugins.get(id).map(|p| &p.lifecycle)
    }
    /// The accepted grant of a loaded plugin.
    pub fn grant(&self, id: &PluginId) -> Option<&Grant> {
        self.plugins.get(id).map(|p| &p.grant)
    }
    /// The grant as stored in `app.db`.
    pub async fn stored_grant(&self, id: &PluginId) -> Result<Grant, InstallError> {
        Ok(grant_from_rows(&self.cfg.store.grants(id.as_str()).await?))
    }

    /// Stages a `.parlaplugin` chosen with "Install from file…".
    pub fn stage_from_file(&self, path: &Path) -> Result<Staged, InstallError> {
        let pkg = Package::open(path, PackageSource::File, &self.cfg.plugins_dir)?;
        self.staged(pkg, InstallSource::File)
    }

    /// Stages a downloaded registry package; `expected_sha256` comes from
    /// the index.
    pub fn stage_from_bytes(
        &self,
        bytes: &[u8],
        expected_sha256: [u8; 32],
    ) -> Result<Staged, InstallError> {
        let pkg = Package::open_bytes(
            bytes,
            PackageSource::Registry {
                sha256: expected_sha256,
            },
            &self.cfg.plugins_dir,
        )?;
        self.staged(pkg, InstallSource::Registry)
    }

    fn staged(&self, pkg: package::Staged, source: InstallSource) -> Result<Staged, InstallError> {
        let path = pkg.dir.path().to_path_buf();
        lock(&self.live_staging).insert(path.clone());
        let live = LiveGuard {
            set: self.live_staging.clone(),
            path,
        };
        if let Some(min) = &pkg.manifest.min_app
            && *min > self.cfg.app_version
        {
            return Err(InstallError::NeedsNewerApp { min: min.clone() });
        }
        let toml = fs::read_to_string(pkg.dir.path().join("plugin.toml"))?;
        Ok(Staged {
            package: pkg,
            source,
            toml,
            _live: live,
        })
    }

    /// What the install dialog shows for `staged`.
    pub async fn consent_request(&self, staged: &Staged) -> Result<ConsentRequest, InstallError> {
        let m = staged.manifest();
        let mut grant = Grant::from_manifest(m);
        for e in &mut grant.exec {
            e.path = (self.cfg.resolve_exec)(&e.program).filter(|p| p.is_absolute());
        }
        let reuses_data = match self.previous(&m.id).await? {
            Previous::Replaceable { row, .. } => same_owner(&row, staged),
            Previous::None => false,
        };
        let cap_mb = m.limits.memory_mb;
        Ok(ConsentRequest {
            summary: Summary {
                id: m.id.clone(),
                name: m.name.clone(),
                version: m.version.clone(),
                authors: m.authors.clone(),
                repository: m.repository.clone(),
                data: m.is_data(),
            },
            rows: if m.is_data() {
                Vec::new()
            } else {
                card_rows(m)
            },
            source: match staged.source {
                InstallSource::Registry => SourceChip::ChecksumVerified,
                InstallSource::File => SourceChip::UnverifiedFile,
            },
            memory: MemoryNote {
                cap_mb,
                above_default: cap_mb > DEFAULT_MEMORY_MB,
            },
            toml: staged.toml.clone(),
            reuses_data,
            grant,
        })
    }

    /// The earlier row for `id`; an installed one is an error.
    async fn previous(&self, id: &PluginId) -> Result<Previous, InstallError> {
        let row = self
            .cfg
            .store
            .plugins()
            .await?
            .into_iter()
            .find(|r| &*r.id == id.as_str());
        let broken = self.broken.contains(id);
        match row {
            None if self.plugins.contains_key(id) => {
                Err(InstallError::AlreadyInstalled { id: id.clone() })
            }
            None => Ok(Previous::None),
            Some(row) if !self.plugins.contains_key(id) && (broken || is_kept_data(&row)) => {
                Ok(Previous::Replaceable { row, broken })
            }
            Some(_) => Err(InstallError::AlreadyInstalled { id: id.clone() }),
        }
    }

    /// Checks each `exec` path against a fresh resolution.
    fn check_exec(&self, grant: &Grant) -> Result<(), InstallError> {
        for e in &grant.exec {
            let now = (self.cfg.resolve_exec)(&e.program)
                .filter(|p| p.is_absolute())
                .ok_or_else(|| InstallError::ExecNotFound(e.program.clone()))?;
            if e.path.as_ref() != Some(&now) {
                return Err(InstallError::GrantMismatch);
            }
        }
        Ok(())
    }

    /// Installs `staged` with the `grant` the user accepted (§8.2 step 5).
    /// Atomic: on failure no folder, row or staging is left, and an earlier
    /// kept or broken row is as it was (unless it was another owner's,
    /// whose data is dropped first).
    pub async fn commit(&mut self, staged: Staged, grant: Grant) -> Result<(), InstallError> {
        let m = Arc::new(staged.package.manifest.clone());
        let id = m.id.clone();
        if without_paths(&grant) != without_paths(&Grant::from_manifest(&m)) {
            return Err(InstallError::GrantMismatch);
        }
        self.check_exec(&grant)?;
        let version = m.version.to_string();
        let now = now_ms();
        let digest = hex(&files_digest(staged.package.dir.path())?);
        let mut row = PluginRow {
            id: id.as_str().into(),
            version: version.as_str().into(),
            source: staged.source.as_row().into(),
            sha256: hex(&staged.sha256()).into(),
            installed_at_ms: now,
            state: INSTALLING_ROW.into(),
            previous_version: None,
            files_digest: Some(digest.into()),
            authors: Some(staged.authors().into()),
        };
        if let Some(entry) = self.cfg.blocklist.entry(&row) {
            return Err(InstallError::Blocked(entry));
        }
        let store = self.cfg.store.clone();
        let (restore, broken, reuse) = match self.previous(&id).await? {
            Previous::None => (None, false, false),
            Previous::Replaceable { row: prev, broken } => {
                if same_owner(&prev, &staged) {
                    let grants = store.grants(&prev.id).await?;
                    (Some((prev, grants)), broken, true)
                } else {
                    // Another owner's data never passes to this plugin.
                    store.delete_plugin(&prev.id).await?;
                    if broken {
                        let _ = self.cfg.secrets.remove_all(&id);
                    }
                    (None, broken, false)
                }
            }
        };

        let id_dir = self.cfg.plugins_dir.join(id.as_str());
        let dest = id_dir.join(&version);
        // A broken row's folder goes aside, back on failure.
        let aside = if broken && id_dir.exists() {
            Some(self.move_aside(&id)?)
        } else {
            None
        };
        let put_aside_back = |aside: &Option<PathBuf>| {
            if let Some(a) = aside
                && fs::rename(a.join("dir"), &id_dir).is_ok()
            {
                let _ = fs::remove_dir_all(a);
            }
        };
        if dest.exists() {
            put_aside_back(&aside);
            return Err(InstallError::FolderExists(dest));
        }

        // The `verified` row first: a crash from here on leaves a row the
        // next load rolls back.
        if let Err(e) = store.upsert_plugin(row.clone()).await {
            put_aside_back(&aside);
            return Err(e.into());
        }
        let created_id_dir = !id_dir.exists();
        let mut moved = false;
        let result = async {
            fs::create_dir_all(&id_dir)?;
            fs::rename(staged.package.dir.path(), &dest)?;
            moved = true;
            store
                .set_grants(id.as_str(), grant_rows(&grant, now))
                .await?;
            let s = &self.cfg.settings;
            if reuse {
                s.set_enabled(id.as_str(), true)
            } else {
                s.reset(id.as_str())
            }
            .map_err(InstallError::Settings)?;
            row.state = PluginState::Installed.as_row().into();
            store.upsert_plugin(row.clone()).await?;
            Ok::<(), InstallError>(())
        }
        .await;
        if let Err(e) = result {
            match &restore {
                Some((prev, grants)) => {
                    let _ = store.upsert_plugin(prev.clone()).await;
                    let _ = store.set_grants(&prev.id, grants.clone()).await;
                }
                None => {
                    let _ = store.delete_plugin(id.as_str()).await;
                }
            }
            if moved {
                let _ = fs::remove_dir_all(&dest);
            }
            if created_id_dir {
                let _ = fs::remove_dir(&id_dir);
            }
            put_aside_back(&aside);
            return Err(e);
        }
        drop(staged);
        if let Some(a) = aside {
            let _ = fs::remove_dir_all(a);
        }

        let (lifecycle, _) = transition(
            &Lifecycle::new(PluginState::Verified, true),
            Event::Installed,
            Instant::now(),
        );
        self.broken.remove(&id);
        self.set.insert(m.clone());
        self.set.register(&id);
        if !m.is_data() {
            self.cfg
                .precompile
                .queue(&id, &version, dest.join("plugin.wasm"));
        }
        self.plugins.insert(
            id,
            Plugin {
                row,
                lifecycle,
                grant,
            },
        );
        Ok(())
    }

    /// Moves `plugins/<id>` to `.removing-<id>-*/dir`; returns the
    /// `.removing-*` folder.
    fn move_aside(&self, id: &PluginId) -> io::Result<PathBuf> {
        let aside = tempfile::Builder::new()
            .prefix(&format!("{REMOVING_PREFIX}{id}-"))
            .tempdir_in(&self.cfg.plugins_dir)?
            .keep();
        if let Err(e) = fs::rename(self.cfg.plugins_dir.join(id.as_str()), aside.join("dir")) {
            let _ = fs::remove_dir(&aside);
            return Err(e);
        }
        Ok(aside)
    }

    /// Turns `plugins.<id>.enabled` on or off (§8.5) and applies it.
    /// Returns the host's effects (close views, unload).
    pub async fn set_enabled(
        &mut self,
        id: &PluginId,
        enabled: bool,
        now: Instant,
    ) -> Result<Vec<Effect>, InstallError> {
        let was = self
            .plugins
            .get(id)
            .ok_or(InstallError::NotInstalled)?
            .lifecycle
            .enabled;
        self.cfg
            .settings
            .set_enabled(id.as_str(), enabled)
            .map_err(InstallError::Settings)?;
        let event = if enabled {
            Event::Enabled
        } else {
            Event::Disabled
        };
        match self.apply(id, event, now).await {
            Ok(effects) => Ok(effects),
            Err(e) => {
                let _ = self.cfg.settings.set_enabled(id.as_str(), was);
                Err(e)
            }
        }
    }

    /// Runs one lifecycle event (B5): persists the new state, then updates
    /// the installed set. Returns the effects left for the host (close
    /// views, unload, stopped card, blocked banner).
    pub async fn apply(
        &mut self,
        id: &PluginId,
        event: Event,
        now: Instant,
    ) -> Result<Vec<Effect>, InstallError> {
        let p = self.plugins.get(id).ok_or(InstallError::NotInstalled)?;
        let (next, effects) = transition(&p.lifecycle, event, now);
        let mut row = p.row.clone();
        if *row.state != *next.row_text() {
            row.state = next.row_text().into();
            self.cfg.store.upsert_plugin(row.clone()).await?;
        }
        self.set.apply(id, &effects);
        if let Some(p) = self.plugins.get_mut(id) {
            p.row = row;
            p.lifecycle = next;
        }
        Ok(host_effects(effects))
    }

    /// Removes a plugin (§8.5), loaded or a row that failed to load: its
    /// folder, row and grant and, unless `keep_data`, its storage, then
    /// its keychain secrets. Returns the host's effects (close views,
    /// unload) to run first.
    pub async fn uninstall(
        &mut self,
        id: &PluginId,
        keep_data: bool,
    ) -> Result<Uninstalled, InstallError> {
        let mut effects = Vec::new();
        let row = match self.plugins.get(id) {
            Some(p) => {
                if p.lifecycle.state.contributes() {
                    effects.push(Effect::CloseViews);
                }
                if p.lifecycle.state == PluginState::Active {
                    effects.push(Effect::Unload);
                }
                p.row.clone()
            }
            None if self.broken.contains(id) => self
                .cfg
                .store
                .plugins()
                .await?
                .into_iter()
                .find(|r| &*r.id == id.as_str() && !is_kept_data(r))
                .ok_or(InstallError::NotInstalled)?,
            None => return Err(InstallError::NotInstalled),
        };

        // The folder goes aside first; while the row is live a load puts
        // it back instead of sweeping it.
        let aside = if self.cfg.plugins_dir.join(id.as_str()).exists() {
            Some(self.move_aside(id)?)
        } else {
            None
        };
        let store = self.cfg.store.clone();
        let removed = if keep_data {
            keep_only_data(&*store, &row).await
        } else {
            store.delete_plugin(id.as_str()).await
        };
        if let Err(e) = removed {
            if let Some(a) = &aside
                && fs::rename(a.join("dir"), self.cfg.plugins_dir.join(id.as_str())).is_ok()
            {
                let _ = fs::remove_dir_all(a);
            }
            return Err(e.into());
        }
        let secrets_error = self.cfg.secrets.remove_all(id).err();
        if let Some(a) = aside {
            let _ = fs::remove_dir_all(a);
        }

        self.cfg.precompile.forget(id, &row.version);
        self.set.remove(id);
        self.plugins.remove(id);
        self.broken.remove(id);
        Ok(Uninstalled {
            effects,
            secrets_error,
        })
    }

    /// Loads the installed plugins at startup: rolls back interrupted
    /// installs, puts back or sweeps folders moved aside, sweeps staging no
    /// live [`Staged`] owns, matches folders with rows (§14), checks each
    /// plugin's files against its digest, rebuilds a blocked row's entry
    /// from the blocklist, applies the user's `enabled` setting and
    /// registers what may contribute.
    pub async fn load(
        &mut self,
        prefs: &settings::PluginsSettings,
        now: Instant,
    ) -> Result<Vec<LoadOutcome>, InstallError> {
        self.broken.clear();
        let mut rows = self.cfg.store.plugins().await?;
        for r in rows.iter().filter(|r| &*r.state == INSTALLING_ROW) {
            self.roll_back(r).await?;
        }
        rows.retain(|r| &*r.state != INSTALLING_ROW);
        let live: HashSet<&str> = rows
            .iter()
            .filter(|r| !is_kept_data(r))
            .map(|r| &*r.id)
            .collect();
        let folders = self.sweep_and_list(&live)?;
        let mut out = Vec::new();
        for d in discover(folders.iter().map(String::as_str), &rows) {
            match d {
                Discovered::FoundNotInstalled(f) => out.push(LoadOutcome::FoundNotInstalled(f)),
                Discovered::MissingFolder(f) => {
                    self.broken.extend(PluginId::parse(&f));
                    out.push(LoadOutcome::MissingFolder(f));
                }
                Discovered::Installed { id, state } => {
                    let Some(row) = rows.iter().find(|r| *r.id == *id).cloned() else {
                        continue;
                    };
                    let o = self.load_one(row, state, prefs, now).await?;
                    if let LoadOutcome::ChangedOnDisk(id) | LoadOutcome::Unreadable(id, _) = &o {
                        self.broken.insert(id.clone());
                    }
                    out.push(o);
                }
            }
        }
        Ok(out)
    }

    /// Undoes a commit that died: its version folder goes, and the row is
    /// deleted, or kept as [`KEPT_DATA_ROW`] when it holds storage (a
    /// takeover of kept data).
    async fn roll_back(&self, row: &PluginRow) -> Result<(), InstallError> {
        let id_dir = self.cfg.plugins_dir.join(&*row.id);
        // A version is one plain path component (semver text).
        let version = Path::new(&*row.version);
        if version.components().count() == 1 && !row.version.starts_with('.') {
            let _ = fs::remove_dir_all(id_dir.join(version));
        }
        let _ = fs::remove_dir(&id_dir);
        let store = &self.cfg.store;
        if store.kv_usage(&row.id).await?.keys > 0 {
            keep_only_data(&**store, row).await?;
        } else {
            store.delete_plugin(&row.id).await?;
        }
        Ok(())
    }

    async fn load_one(
        &mut self,
        row: PluginRow,
        state: PluginState,
        prefs: &settings::PluginsSettings,
        now: Instant,
    ) -> Result<LoadOutcome, InstallError> {
        let dir = self.cfg.plugins_dir.join(&*row.id).join(&*row.version);
        let src = match fs::read_to_string(dir.join("plugin.toml")) {
            Ok(s) => s,
            Err(e) => return Ok(unreadable(&row, e.to_string())),
        };
        let m = match manifest::parse(&src, ManifestSource::Package) {
            Ok(m) => m,
            Err(e) => return Ok(unreadable(&row, e.to_string())),
        };
        let id = m.id.clone();
        if id.as_str() != &*row.id {
            return Ok(unreadable(&row, "plugin.toml names another id".into()));
        }
        // Fail closed: only an explicit allow-list skips the check.
        if !DIGEST_EXEMPT_SOURCES.contains(&&*row.source) {
            let have = files_digest(&dir).ok().map(|d| hex(&d));
            if row.files_digest.is_none() || row.files_digest.as_deref() != have.as_deref() {
                return Ok(LoadOutcome::ChangedOnDisk(id));
            }
        }
        let grant_rows = self.cfg.store.grants(&row.id).await?;

        let enabled = prefs.entry(&row.id).is_none_or(|e| e.enabled);
        let mut lc = Lifecycle::new(state, enabled);
        lc.quarantined_under_block = &*row.state == crate::lifecycle::BLOCKED_QUARANTINED_ROW;
        let listed = self.cfg.blocklist.entry(&row);
        if state == PluginState::Blocked {
            lc.block = listed.clone();
            // The row says blocked but the list no longer does.
            if listed.is_none() {
                lc = transition(&lc, Event::Unblocked, now).0;
            }
        } else if let Some(entry) = listed {
            lc = transition(&lc, Event::Blocked(entry), now).0;
        }
        // `settings.json` may have changed while Parla was closed.
        if lc.state == PluginState::Installed && !enabled {
            lc = transition(&lc, Event::Disabled, now).0;
        } else if lc.state == PluginState::Disabled && enabled {
            lc = transition(&lc, Event::Enabled, now).0;
        }

        let mut row = row;
        if *row.state != *lc.row_text() {
            row.state = lc.row_text().into();
            self.cfg.store.upsert_plugin(row.clone()).await?;
        }
        self.set.insert(Arc::new(m));
        if lc.state.contributes() {
            self.set.register(&id);
        }
        let state = lc.state;
        self.plugins.insert(
            id.clone(),
            Plugin {
                row,
                lifecycle: lc,
                grant: grant_from_rows(&grant_rows),
            },
        );
        Ok(LoadOutcome::Loaded(id, state))
    }

    /// Handles hidden leftovers and returns the plugin folders. A
    /// `.staging-*` folder no live [`Staged`] owns is removed. A
    /// `.removing-<id>-*` folder is put back when `<id>` still has a live
    /// row and no folder (an uninstall that died before its store write),
    /// else removed; if putting it back fails it is left for the next load.
    fn sweep_and_list(&self, live: &HashSet<&str>) -> io::Result<Vec<String>> {
        let mut folders = Vec::new();
        let entries = match fs::read_dir(&self.cfg.plugins_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(folders),
            Err(e) => return Err(e),
        };
        let mut aside = Vec::new();
        for e in entries {
            let e = e?;
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if !e.file_type()?.is_dir() {
                continue;
            }
            if name.starts_with(STAGING_PREFIX) {
                if !lock(&self.live_staging).contains(&e.path()) {
                    let _ = fs::remove_dir_all(e.path());
                }
            } else if let Some(rest) = name.strip_prefix(REMOVING_PREFIX) {
                aside.push((rest.rsplit_once('-').map(|(id, _)| id.to_owned()), e.path()));
            } else if !name.starts_with('.') {
                folders.push(name);
            }
        }
        for (id, path) in aside {
            if let Some(id) = id
                && live.contains(id.as_str())
                && !folders.contains(&id)
                && path.join("dir").is_dir()
            {
                if fs::rename(path.join("dir"), self.cfg.plugins_dir.join(&id)).is_ok() {
                    let _ = fs::remove_dir_all(&path);
                    folders.push(id);
                }
                continue;
            }
            let _ = fs::remove_dir_all(&path);
        }
        Ok(folders)
    }
}

/// Whether a kept or broken row belongs to the same publisher as `staged`:
/// the same source, and for a file the same authors (the registry vouches
/// for its ids).
fn same_owner(prev: &PluginRow, staged: &Staged) -> bool {
    *prev.source == *staged.source.as_row()
        && (staged.source == InstallSource::Registry
            || prev.authors.as_deref() == Some(staged.authors().as_str()))
}

/// "Keep its data": the row stays as [`KEPT_DATA_ROW`] holding only the
/// storage; grant, write allowances and digest go.
async fn keep_only_data(store: &dyn AppStore, row: &PluginRow) -> Result<(), StoreError> {
    let allows: Vec<WriteAllow> = store.write_allow(&row.id).await?;
    for a in &allows {
        store.revoke_write_allow(&row.id, a).await?;
    }
    store.set_grants(&row.id, Vec::new()).await?;
    let mut kept = row.clone();
    kept.state = KEPT_DATA_ROW.into();
    kept.files_digest = None;
    store.upsert_plugin(kept).await
}

fn unreadable(row: &PluginRow, why: String) -> LoadOutcome {
    match PluginId::parse(&row.id) {
        Some(id) => LoadOutcome::Unreadable(id, why),
        None => LoadOutcome::FoundNotInstalled(row.id.to_string()),
    }
}

/// The installed set's effects are done; the rest are the host's.
fn host_effects(effects: Vec<Effect>) -> Vec<Effect> {
    effects
        .into_iter()
        .filter(|e| {
            !matches!(
                e,
                Effect::RegisterContributions | Effect::RemoveContributions | Effect::Persist(_)
            )
        })
        .collect()
}

/// SHA-256 over every file under `dir` in path order: path, a zero byte,
/// the length as 8 little-endian bytes, the content. A link or special
/// file is an error.
pub fn files_digest(dir: &Path) -> io::Result<[u8; 32]> {
    fn walk(root: &Path, rel: &str, out: &mut Vec<String>) -> io::Result<()> {
        for e in fs::read_dir(root.join(rel))? {
            let e = e?;
            let name = e
                .file_name()
                .into_string()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non UTF-8 name"))?;
            let path = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            let ft = fs::symlink_metadata(e.path())?.file_type();
            if ft.is_dir() {
                walk(root, &path, out)?;
            } else if ft.is_file() {
                out.push(path);
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "not a regular file",
                ));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    files.sort();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    for f in &files {
        let mut file = fs::File::open(dir.join(f))?;
        h.update(f.as_bytes());
        h.update([0]);
        h.update(file.metadata()?.len().to_le_bytes());
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
    }
    Ok(h.finalize().into())
}

/// The grant as `plugin_grants` rows: one per read area, write area,
/// network host and `exec` program, one per granted switch, one for the
/// implied access. Values are JSON.
pub fn grant_rows(g: &Grant, granted_at_ms: i64) -> Vec<GrantRow> {
    let mut out = Vec::new();
    let mut push = |cap: &str, value: String| {
        out.push(GrantRow {
            capability: cap.into(),
            value: value.into(),
            granted_at_ms,
        });
    };
    for a in &g.read {
        push("read", json(a));
    }
    for a in &g.write {
        push("write", json(a));
    }
    for h in &g.network {
        push("network", json(h));
    }
    for (cap, on) in [
        ("storage", g.storage),
        ("windows", g.windows),
        ("panel", g.panel),
        ("notifications", g.notifications),
        ("clipboard_write", g.clipboard_write),
    ] {
        if on {
            push(cap, String::new());
        }
    }
    // Indexed so the manifest order survives the store's sorting.
    for (i, e) in g.exec.iter().enumerate() {
        push("exec", json(&(i, e)));
    }
    if g.implied != Default::default() {
        push("implied", json(&g.implied));
    }
    out
}

/// Reads [`grant_rows`] back. A row that is unknown or does not parse is
/// skipped, so stored rows can only narrow a grant.
pub fn grant_from_rows(rows: &[GrantRow]) -> Grant {
    fn parse<T: serde::de::DeserializeOwned>(v: &str) -> Option<T> {
        serde_json::from_str(v).ok()
    }
    let mut g = Grant::default();
    let mut exec = Vec::new();
    for r in rows {
        let v = &*r.value;
        match &*r.capability {
            "read" => g.read.extend(parse::<ReadArea>(v)),
            "write" => g.write.extend(parse::<WriteArea>(v)),
            "network" => g.network.extend(parse::<NetworkHost>(v)),
            "storage" => g.storage = true,
            "windows" => g.windows = true,
            "panel" => g.panel = true,
            "notifications" => g.notifications = true,
            "clipboard_write" => g.clipboard_write = true,
            "exec" => exec.extend(parse::<(usize, ExecGrant)>(v)),
            "implied" => {
                if let Some(i) = parse(v) {
                    g.implied = i;
                }
            }
            _ => {}
        }
    }
    exec.sort_by_key(|(i, _)| *i);
    g.exec = exec.into_iter().map(|(_, e)| e).collect();
    g
}

/// JSON text of a grant element.
fn json<T: serde::Serialize + ?Sized>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn without_paths(g: &Grant) -> Grant {
    let mut g = g.clone();
    for e in &mut g.exec {
        e.path = None;
    }
    g
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}
