//! Install, disable and uninstall pipeline. In-memory `AppStore`, temp directories and fakes
//! for settings, secrets and the precompile queue.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures::future::BoxFuture;

use plugin_host::caps::Grant;
use plugin_host::install::{
    Blocklist, InstallError, InstallSource, Installer, InstallerConfig, LoadOutcome, PluginSecrets,
    PrecompileQueue, SettingsWriter, SourceChip, grant_from_rows, grant_rows, resolve_on_path,
};
use plugin_host::lifecycle::{BlockEntry, Event, FailureReason, PluginState};
use plugin_host::manifest::{ManifestSource, PluginId, parse};
use plugin_host::package::{PackageError, pack};
use plugin_host::registry::ContributionKind;
use semver::Version;
use sha2::{Digest, Sha256};
use store::{
    AppStore, AuditEntry, AuditQuery, AuditRow, CwasmKey, GrantRow, KvScope, KvUsage,
    MemoryAppStore, PluginRow, StoreError, WriteAllow,
};

const MANIFEST: &str = r#"id = "hello-plugin"
name = "Hello"
version = "0.1.0"
api = "0.1"
license = "MIT"
authors = ["Ada"]
activation = ["command:hello_plugin::greet"]

[capabilities]
network = ["api.example.com"]
storage = true

[[capabilities.exec]]
program = "gh"
allow = [["pr", "view", "*"]]

[[commands]]
id = "greet"
title = "Say hello"
"#;

const WASM: &[u8] = b"\0asm\x0d\0\x01\0";

fn id() -> PluginId {
    parse(MANIFEST, ManifestSource::Package).unwrap().id
}

/// A packed `.parlaplugin` of `manifest` plus a dummy component.
fn package(manifest: &str) -> Vec<u8> {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("plugin.toml"), manifest).unwrap();
    fs::write(d.path().join("plugin.wasm"), WASM).unwrap();
    fs::write(d.path().join("LICENSE"), "MIT").unwrap();
    fs::write(d.path().join("icon.svg"), "<svg/>").unwrap();
    pack(d.path()).unwrap()
}

#[derive(Default)]
struct FakeSettings {
    fail: bool,
    writes: Mutex<Vec<(String, bool)>>,
    resets: Mutex<Vec<String>>,
}

impl SettingsWriter for FakeSettings {
    fn set_enabled(&self, id: &str, enabled: bool) -> std::io::Result<()> {
        if self.fail {
            return Err(std::io::Error::other("settings.json is read-only"));
        }
        self.writes.lock().unwrap().push((id.to_owned(), enabled));
        Ok(())
    }
    fn reset(&self, id: &str) -> std::io::Result<()> {
        if self.fail {
            return Err(std::io::Error::other("settings.json is read-only"));
        }
        self.resets.lock().unwrap().push(id.to_owned());
        self.writes.lock().unwrap().push((id.to_owned(), true));
        Ok(())
    }
}

#[derive(Default)]
struct FakeSecrets {
    removed: Mutex<Vec<String>>,
    fail: AtomicBool,
}

impl PluginSecrets for FakeSecrets {
    fn remove_all(&self, id: &PluginId) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("keychain locked".into());
        }
        self.removed.lock().unwrap().push(id.to_string());
        Ok(())
    }
}

#[derive(Default)]
struct FakeBlocklist(Mutex<Option<BlockEntry>>);

impl Blocklist for FakeBlocklist {
    fn entry(&self, _row: &PluginRow) -> Option<BlockEntry> {
        self.0.lock().unwrap().clone()
    }
}

/// What [`FlakyStore`] does on its trigger method.
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    Fail,
    /// A crash: the task dies mid-step and no undo runs.
    Panic,
}

/// [`MemoryAppStore`] that fails or panics on one method.
#[derive(Default)]
struct FlakyStore {
    inner: MemoryAppStore,
    on: Mutex<Option<(&'static str, Fault)>>,
}

impl FlakyStore {
    fn set(&self, method: &'static str, fault: Fault) {
        *self.on.lock().unwrap() = Some((method, fault));
    }
    fn clear(&self) {
        *self.on.lock().unwrap() = None;
    }
    fn hit(&self, method: &str) -> Result<(), StoreError> {
        let on = *self.on.lock().unwrap();
        match on {
            Some((m, Fault::Fail)) if m == method => Err(StoreError::Other("disk full".into())),
            Some((m, Fault::Panic)) if m == method => panic!("crash in {method}"),
            _ => Ok(()),
        }
    }
}

macro_rules! guard {
    ($self:ident, $m:literal, $call:expr) => {{
        if let Err(e) = $self.hit($m) {
            return Box::pin(async move { Err(e) });
        }
        $call
    }};
}

impl AppStore for FlakyStore {
    fn audit_append(&self, e: AuditEntry) -> BoxFuture<'_, Result<i64, StoreError>> {
        self.inner.audit_append(e)
    }
    fn audit_page(&self, q: AuditQuery) -> BoxFuture<'_, Result<Vec<AuditRow>, StoreError>> {
        self.inner.audit_page(q)
    }
    fn audit_prune(&self, t: i64) -> BoxFuture<'_, Result<u64, StoreError>> {
        self.inner.audit_prune(t)
    }
    fn audit_forget_workspace(
        &self,
        ws: &settings::WorkspaceKey,
    ) -> BoxFuture<'_, Result<u64, StoreError>> {
        self.inner.audit_forget_workspace(ws)
    }
    fn audit_clear(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.audit_clear()
    }
    fn plugins(&self) -> BoxFuture<'_, Result<Vec<PluginRow>, StoreError>> {
        guard!(self, "plugins", self.inner.plugins())
    }
    fn upsert_plugin(&self, row: PluginRow) -> BoxFuture<'_, Result<(), StoreError>> {
        guard!(self, "upsert_plugin", self.inner.upsert_plugin(row))
    }
    fn delete_plugin(&self, id: &str) -> BoxFuture<'_, Result<(), StoreError>> {
        guard!(self, "delete_plugin", self.inner.delete_plugin(id))
    }
    fn grants(&self, id: &str) -> BoxFuture<'_, Result<Vec<GrantRow>, StoreError>> {
        self.inner.grants(id)
    }
    fn set_grants(&self, id: &str, g: Vec<GrantRow>) -> BoxFuture<'_, Result<(), StoreError>> {
        guard!(self, "set_grants", self.inner.set_grants(id, g))
    }
    fn write_allow(&self, id: &str) -> BoxFuture<'_, Result<Vec<WriteAllow>, StoreError>> {
        self.inner.write_allow(id)
    }
    fn add_write_allow(&self, id: &str, a: WriteAllow) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.add_write_allow(id, a)
    }
    fn revoke_write_allow(
        &self,
        id: &str,
        a: &WriteAllow,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.revoke_write_allow(id, a)
    }
    fn kv_get(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<Vec<u8>>, StoreError>> {
        self.inner.kv_get(id, scope, key)
    }
    fn kv_set(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
        value: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.kv_set(id, scope, key, value)
    }
    fn kv_delete(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.kv_delete(id, scope, key)
    }
    fn kv_keys(
        &self,
        id: &str,
        scope: &KvScope,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Arc<str>>, StoreError>> {
        self.inner.kv_keys(id, scope, prefix, after, limit)
    }
    fn kv_clear(&self, id: &str, scope: &KvScope) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.kv_clear(id, scope)
    }
    fn kv_usage(&self, id: &str) -> BoxFuture<'_, Result<KvUsage, StoreError>> {
        self.inner.kv_usage(id)
    }
    fn cwasm_hash(&self, key: &CwasmKey) -> BoxFuture<'_, Result<Option<[u8; 32]>, StoreError>> {
        self.inner.cwasm_hash(key)
    }
    fn set_cwasm_hash(
        &self,
        key: &CwasmKey,
        sha256: Option<[u8; 32]>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.set_cwasm_hash(key, sha256)
    }
}

#[derive(Default)]
struct FakeQueue {
    queued: Mutex<Vec<(String, String, PathBuf)>>,
    forgotten: Mutex<Vec<(String, String)>>,
}

impl PrecompileQueue for FakeQueue {
    fn queue(&self, id: &PluginId, version: &str, wasm: PathBuf) {
        self.queued
            .lock()
            .unwrap()
            .push((id.to_string(), version.to_owned(), wasm));
    }
    fn forget(&self, id: &PluginId, version: &str) {
        self.forgotten
            .lock()
            .unwrap()
            .push((id.to_string(), version.to_owned()));
    }
}

struct Rig {
    _root: tempfile::TempDir,
    plugins: PathBuf,
    downloads: PathBuf,
    store: Arc<FlakyStore>,
    settings: Arc<FakeSettings>,
    secrets: Arc<FakeSecrets>,
    queue: Arc<FakeQueue>,
    blocked: Arc<FakeBlocklist>,
    /// What the fake `resolve_exec` returns for `gh`.
    gh: Arc<Mutex<Option<PathBuf>>>,
}

impl Rig {
    fn new() -> Self {
        Self::with_settings(FakeSettings::default())
    }
    fn with_settings(settings: FakeSettings) -> Self {
        let root = tempfile::tempdir().unwrap();
        let plugins = root.path().join("plugins");
        let downloads = root.path().join("downloads");
        fs::create_dir_all(&plugins).unwrap();
        fs::create_dir_all(&downloads).unwrap();
        Self {
            _root: root,
            plugins,
            downloads,
            store: Arc::new(FlakyStore::default()),
            settings: Arc::new(settings),
            secrets: Arc::new(FakeSecrets::default()),
            queue: Arc::new(FakeQueue::default()),
            blocked: Arc::new(FakeBlocklist::default()),
            gh: Arc::new(Mutex::new(Some(PathBuf::from("/opt/bin/gh")))),
        }
    }
    fn installer(&self) -> Installer {
        Installer::new(InstallerConfig {
            plugins_dir: self.plugins.clone(),
            app_version: Version::new(0, 9, 0),
            store: self.store.clone(),
            settings: self.settings.clone(),
            secrets: self.secrets.clone(),
            precompile: self.queue.clone(),
            blocklist: self.blocked.clone(),
            resolve_exec: {
                let gh = self.gh.clone();
                Box::new(move |p| {
                    if p == "gh" {
                        gh.lock().unwrap().clone()
                    } else {
                        None
                    }
                })
            },
        })
    }
    fn file(&self, bytes: &[u8]) -> PathBuf {
        let p = self.downloads.join("hello-plugin-0.1.0.parlaplugin");
        fs::write(&p, bytes).unwrap();
        p
    }
    /// Installs [`MANIFEST`] from a file.
    async fn install(&self, inst: &mut Installer) {
        self.install_manifest(inst, MANIFEST).await.unwrap();
    }
    async fn install_manifest(&self, inst: &mut Installer, m: &str) -> Result<(), InstallError> {
        let staged = inst.stage_from_file(&self.file(&package(m)))?;
        let consent = inst.consent_request(&staged).await?;
        inst.commit(staged, consent.grant).await
    }
    async fn kv(&self) -> Option<Vec<u8>> {
        self.store
            .kv_get("hello-plugin", &KvScope::Global, "k")
            .await
            .unwrap()
    }
    async fn set_kv(&self) {
        self.store
            .kv_set("hello-plugin", &KvScope::Global, "k", b"v".to_vec())
            .await
            .unwrap();
    }
    fn version_dir(&self) -> PathBuf {
        self.plugins.join("hello-plugin").join("0.1.0")
    }
}

/// Entries of `dir`, names only.
fn entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn install_from_file_registers_contributions() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    let consent = inst.consent_request(&staged).await.unwrap();
    assert_eq!(consent.source, SourceChip::UnverifiedFile);
    assert_eq!(consent.summary.id, id());
    assert!(!consent.rows.is_empty());
    assert_eq!(consent.memory.cap_mb, 16);
    assert!(!consent.memory.above_default);
    assert_eq!(consent.toml, MANIFEST);
    // The exec path is resolved at consent.
    assert_eq!(
        consent.grant.exec[0].path.as_deref(),
        Some(Path::new("/opt/bin/gh"))
    );
    inst.commit(staged, consent.grant.clone()).await.unwrap();

    let cmds: Vec<String> = inst
        .installed()
        .contributions(ContributionKind::Command)
        .map(|c| c.id.to_owned())
        .collect();
    assert_eq!(cmds, ["greet"]);
    assert!(rig.version_dir().join("plugin.toml").is_file());
    assert_eq!(entries(&rig.plugins), ["hello-plugin"], "no staging left");

    let rows = rig.store.plugins().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(&*rows[0].source, "file");
    assert_eq!(&*rows[0].state, "installed");
    assert_eq!(&*rows[0].version, "0.1.0");
    let stored = grant_from_rows(&rig.store.grants("hello-plugin").await.unwrap());
    assert_eq!(stored, consent.grant);
    assert_eq!(
        *rig.settings.writes.lock().unwrap(),
        [("hello-plugin".to_owned(), true)]
    );
    assert_eq!(
        inst.lifecycle(&id()).map(|l| l.state),
        Some(PluginState::Installed)
    );
}

#[tokio::test]
async fn hash_mismatch_installs_nothing() {
    let rig = Rig::new();
    let inst = rig.installer();
    let bytes = package(MANIFEST);
    let mut wrong: [u8; 32] = Sha256::digest(&bytes).into();
    wrong[0] ^= 1;
    let err = inst.stage_from_bytes(&bytes, wrong).unwrap_err();
    assert!(
        matches!(err, InstallError::Package(PackageError::HashMismatch)),
        "{err:?}"
    );
    assert!(entries(&rig.plugins).is_empty());
    assert!(rig.store.plugins().await.unwrap().is_empty());

    // The right hash stages from the registry with the verified chip.
    let ok = inst
        .stage_from_bytes(&bytes, Sha256::digest(&bytes).into())
        .unwrap();
    assert_eq!(ok.source(), InstallSource::Registry);
    assert_eq!(
        inst.consent_request(&ok).await.unwrap().source,
        SourceChip::ChecksumVerified
    );
    drop(ok);
    assert!(
        entries(&rig.plugins).is_empty(),
        "dropping staged cleans up"
    );
}

#[tokio::test]
async fn failure_mid_commit_leaves_no_row() {
    let rig = Rig::with_settings(FakeSettings {
        fail: true,
        ..Default::default()
    });
    let mut inst = rig.installer();
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    let consent = inst.consent_request(&staged).await.unwrap();
    let err = inst.commit(staged, consent.grant).await.unwrap_err();
    assert!(matches!(err, InstallError::Settings(_)), "{err:?}");
    assert!(rig.store.plugins().await.unwrap().is_empty());
    assert!(rig.store.grants("hello-plugin").await.unwrap().is_empty());
    assert!(entries(&rig.plugins).is_empty(), "no folder, no staging");
    assert!(inst.installed().is_empty());
    assert!(rig.queue.queued.lock().unwrap().is_empty());
}

#[tokio::test]
async fn commit_refuses_a_grant_wider_than_the_manifest() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    let mut grant = inst.consent_request(&staged).await.unwrap().grant;
    grant.windows = true;
    let err = inst.commit(staged, grant).await.unwrap_err();
    assert!(matches!(err, InstallError::GrantMismatch), "{err:?}");
    assert!(entries(&rig.plugins).is_empty());
    assert!(rig.store.plugins().await.unwrap().is_empty());
}

#[tokio::test]
async fn needs_newer_app_stages_nothing() {
    let rig = Rig::new();
    let inst = rig.installer();
    let m = MANIFEST.replace(
        "license = \"MIT\"",
        "license = \"MIT\"\nmin_app = \"1.2.0\"",
    );
    let err = inst.stage_from_file(&rig.file(&package(&m))).unwrap_err();
    assert!(matches!(err, InstallError::NeedsNewerApp { .. }), "{err:?}");
    assert!(entries(&rig.plugins).is_empty());
}

#[tokio::test]
async fn editing_manifest_on_disk_does_not_widen_grant() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    let before = inst.grant(&id()).cloned().unwrap();

    let toml = rig.version_dir().join("plugin.toml");
    let wider = MANIFEST.replace("storage = true", "storage = true\nwindows = true");
    fs::write(&toml, &wider).unwrap();
    let edited = parse(&wider, ManifestSource::Package).unwrap();
    assert_ne!(Grant::from_manifest(&edited).windows, before.windows);

    let mut fresh = rig.installer();
    fresh
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    let stored = fresh.stored_grant(&id()).await.unwrap();
    assert_eq!(stored, before);
    assert!(!stored.windows);
    assert!(fresh.grant(&id()).is_none_or(|g| !g.windows));
}

#[tokio::test]
async fn changed_package_on_disk_refuses_to_load() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    fs::write(
        rig.version_dir().join("plugin.wasm"),
        b"\0asm\x0d\0\x01\0\0",
    )
    .unwrap();

    let mut fresh = rig.installer();
    let out = fresh
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert_eq!(out, [LoadOutcome::ChangedOnDisk(id())]);
    assert!(fresh.installed().get(&id()).is_none());
    assert_eq!(
        fresh
            .installed()
            .contributions(ContributionKind::Command)
            .count(),
        0
    );

    // An unchanged install loads.
    let rig2 = Rig::new();
    let mut a = rig2.installer();
    rig2.install(&mut a).await;
    let mut b = rig2.installer();
    let out = b.load(&Default::default(), Instant::now()).await.unwrap();
    assert_eq!(out, [LoadOutcome::Loaded(id(), PluginState::Installed)]);
    assert_eq!(
        b.installed()
            .contributions(ContributionKind::Command)
            .count(),
        1
    );
}

#[tokio::test]
async fn uninstall_removes_dir_rows_secrets_and_storage() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    rig.store
        .kv_set("hello-plugin", &KvScope::Global, "k", b"v".to_vec())
        .await
        .unwrap();

    let done = inst.uninstall(&id(), false).await.unwrap();
    assert!(!done.effects.is_empty());
    assert_eq!(done.secrets_error, None);
    assert!(entries(&rig.plugins).is_empty(), "directory removed");
    assert!(rig.store.plugins().await.unwrap().is_empty());
    assert!(rig.store.grants("hello-plugin").await.unwrap().is_empty());
    assert_eq!(
        rig.store
            .kv_get("hello-plugin", &KvScope::Global, "k")
            .await
            .unwrap(),
        None
    );
    assert_eq!(*rig.secrets.removed.lock().unwrap(), ["hello-plugin"]);
    assert_eq!(
        *rig.queue.forgotten.lock().unwrap(),
        [("hello-plugin".to_owned(), "0.1.0".to_owned())]
    );
    assert!(inst.installed().is_empty());
    assert!(inst.lifecycle(&id()).is_none());
    assert!(matches!(
        inst.uninstall(&id(), false).await,
        Err(InstallError::NotInstalled)
    ));
}

#[tokio::test]
async fn uninstall_keep_data_keeps_kv() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    rig.store
        .kv_set("hello-plugin", &KvScope::Global, "k", b"v".to_vec())
        .await
        .unwrap();

    inst.uninstall(&id(), true).await.unwrap();
    assert!(entries(&rig.plugins).is_empty());
    assert!(rig.store.grants("hello-plugin").await.unwrap().is_empty());
    assert_eq!(*rig.secrets.removed.lock().unwrap(), ["hello-plugin"]);
    assert_eq!(
        rig.store
            .kv_get("hello-plugin", &KvScope::Global, "k")
            .await
            .unwrap()
            .as_deref(),
        Some(&b"v"[..])
    );
    assert!(inst.installed().is_empty());

    // Not installed after a restart either.
    let mut fresh = rig.installer();
    let out = fresh
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert!(out.is_empty(), "{out:?}");

    // Installing again finds the kept data.
    rig.install(&mut fresh).await;
    assert_eq!(
        rig.store
            .kv_get("hello-plugin", &KvScope::Global, "k")
            .await
            .unwrap()
            .as_deref(),
        Some(&b"v"[..])
    );
}

#[tokio::test]
async fn nothing_instantiated_on_install() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    assert_eq!(
        inst.lifecycle(&id()).map(|l| l.state),
        Some(PluginState::Installed)
    );
    let queued = rig.queue.queued.lock().unwrap().clone();
    assert_eq!(queued.len(), 1, "only a precompile is queued");
    assert_eq!(queued[0].2, rig.version_dir().join("plugin.wasm"));
    // Nothing compiled or cached in the plugin folder.
    assert_eq!(
        entries(&rig.version_dir()),
        ["LICENSE", "icon.svg", "plugin.toml", "plugin.wasm"]
    );
}

#[tokio::test]
async fn set_enabled_writes_setting_and_state() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    inst.set_enabled(&id(), false, Instant::now())
        .await
        .unwrap();
    assert_eq!(
        inst.installed()
            .contributions(ContributionKind::Command)
            .count(),
        0
    );
    assert_eq!(&*rig.store.plugins().await.unwrap()[0].state, "disabled");
    assert_eq!(
        rig.settings.writes.lock().unwrap().last(),
        Some(&("hello-plugin".to_owned(), false))
    );
    inst.set_enabled(&id(), true, Instant::now()).await.unwrap();
    assert_eq!(&*rig.store.plugins().await.unwrap()[0].state, "installed");
    assert_eq!(
        inst.installed()
            .contributions(ContributionKind::Command)
            .count(),
        1
    );
}

#[tokio::test]
async fn quarantined_under_block_survives_restart() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    let now = Instant::now();
    for _ in 0..3 {
        inst.apply(&id(), Event::Failed(FailureReason::Crashed), now)
            .await
            .unwrap();
    }
    assert_eq!(
        inst.lifecycle(&id()).unwrap().state,
        PluginState::Quarantined
    );
    let entry = BlockEntry {
        reason: "steals tokens".into(),
        advisory: None,
        malicious: false,
    };
    inst.apply(&id(), Event::Blocked(entry.clone()), now)
        .await
        .unwrap();

    // Restart while still listed: Blocked, entry rebuilt from the list.
    *rig.blocked.0.lock().unwrap() = Some(entry.clone());
    let mut fresh = rig.installer();
    fresh.load(&Default::default(), now).await.unwrap();
    let lc = fresh.lifecycle(&id()).unwrap();
    assert_eq!(lc.state, PluginState::Blocked);
    assert_eq!(lc.block.as_ref(), Some(&entry));
    assert!(lc.quarantined_under_block);

    // Restart after the block is lifted: back to Quarantined, not enabled.
    *rig.blocked.0.lock().unwrap() = None;
    let mut again = rig.installer();
    again.load(&Default::default(), now).await.unwrap();
    assert_eq!(
        again.lifecycle(&id()).unwrap().state,
        PluginState::Quarantined
    );
    assert_eq!(&*rig.store.plugins().await.unwrap()[0].state, "quarantined");
}

#[tokio::test]
async fn load_blocks_a_newly_listed_plugin_and_honours_the_setting() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    let entry = BlockEntry {
        reason: "bad".into(),
        advisory: Some("https://example.com/a".into()),
        malicious: true,
    };
    *rig.blocked.0.lock().unwrap() = Some(entry);
    let mut fresh = rig.installer();
    fresh
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert_eq!(fresh.lifecycle(&id()).unwrap().state, PluginState::Blocked);
    assert_eq!(&*rig.store.plugins().await.unwrap()[0].state, "blocked");
    assert_eq!(
        fresh
            .installed()
            .contributions(ContributionKind::Command)
            .count(),
        0
    );

    // `enabled: false` in settings.json is applied on load.
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    let mut settings = settings::PluginsSettings::default();
    settings.entries.insert(
        "hello-plugin".into(),
        settings::PluginEntry {
            enabled: false,
            ..Default::default()
        },
    );
    let mut fresh = rig.installer();
    fresh.load(&settings, Instant::now()).await.unwrap();
    assert_eq!(fresh.lifecycle(&id()).unwrap().state, PluginState::Disabled);
}

#[tokio::test]
async fn load_sweeps_leftover_staging() {
    let rig = Rig::new();
    fs::create_dir_all(rig.plugins.join(".staging-abc/x")).unwrap();
    fs::create_dir_all(rig.plugins.join(".removing-hello-plugin-1")).unwrap();
    let mut inst = rig.installer();
    inst.load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert!(entries(&rig.plugins).is_empty());
}

#[test]
fn grant_rows_round_trip() {
    let src = include_str!("fixtures/manifests/full.toml");
    let m = parse(src, ManifestSource::Package).unwrap();
    let mut g = Grant::from_manifest(&m);
    g.set_exec_path("gh", PathBuf::from("/opt/bin/gh"));
    let rows = grant_rows(&g, 7);
    assert!(rows.iter().all(|r| r.granted_at_ms == 7));
    assert_eq!(grant_from_rows(&rows), g);
    // Unknown or broken rows never widen a grant.
    let mut extra = rows.clone();
    extra.push(store::GrantRow {
        capability: "future".into(),
        value: "x".into(),
        granted_at_ms: 7,
    });
    extra.push(store::GrantRow {
        capability: "read".into(),
        value: "not json".into(),
        granted_at_ms: 7,
    });
    assert_eq!(grant_from_rows(&extra), g);
}

/// Runs `f` on its own task and expects it to die mid-way (a crash).
async fn crash<F>(f: F)
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    let r = tokio::spawn(f).await;
    assert!(r.is_err_and(|e| e.is_panic()), "expected a crash");
}

async fn plugin_rows(rig: &Rig) -> Vec<PluginRow> {
    rig.store.plugins().await.unwrap()
}

#[tokio::test]
async fn crash_mid_uninstall_then_reload() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    // Dies after the folder moved aside, before the row is removed.
    rig.store.set("delete_plugin", Fault::Panic);
    crash(async move {
        let _ = inst.uninstall(&id(), false).await;
    })
    .await;
    rig.store.clear();

    // The row is live, so its folder is put back, not swept.
    let mut fresh = rig.installer();
    let out = fresh
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert_eq!(out, [LoadOutcome::Loaded(id(), PluginState::Installed)]);
    assert!(rig.version_dir().join("plugin.wasm").is_file());

    fresh.uninstall(&id(), false).await.unwrap();
    assert!(entries(&rig.plugins).is_empty());
    assert!(plugin_rows(&rig).await.is_empty());
}

#[tokio::test]
async fn missing_folder_row_can_be_uninstalled() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    fs::remove_dir_all(rig.plugins.join("hello-plugin")).unwrap();

    let mut fresh = rig.installer();
    let out = fresh
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert_eq!(out, [LoadOutcome::MissingFolder("hello-plugin".into())]);
    fresh.uninstall(&id(), false).await.unwrap();
    assert!(plugin_rows(&rig).await.is_empty());
    assert_eq!(*rig.secrets.removed.lock().unwrap(), ["hello-plugin"]);
}

#[tokio::test]
async fn reinstall_after_changed_on_disk() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    rig.set_kv().await;
    fs::write(
        rig.version_dir().join("plugin.wasm"),
        b"\0asm\x0d\0\x01\0\0",
    )
    .unwrap();

    let mut fresh = rig.installer();
    let out = fresh
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert_eq!(out, [LoadOutcome::ChangedOnDisk(id())]);

    // The same package from the same source installs over the broken row
    // and keeps its data.
    rig.install(&mut fresh).await;
    assert_eq!(
        fresh
            .installed()
            .contributions(ContributionKind::Command)
            .count(),
        1
    );
    assert_eq!(rig.kv().await.as_deref(), Some(&b"v"[..]));
    assert_eq!(entries(&rig.plugins), ["hello-plugin"]);
    let mut again = rig.installer();
    let out = again
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert_eq!(out, [LoadOutcome::Loaded(id(), PluginState::Installed)]);
}

#[tokio::test]
async fn kept_data_reused_only_for_same_source_and_author() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    rig.set_kv().await;
    inst.uninstall(&id(), true).await.unwrap();
    // A fresh install starts from a clean settings entry.
    assert_eq!(*rig.settings.resets.lock().unwrap(), ["hello-plugin"]);
    rig.settings.resets.lock().unwrap().clear();

    // Same source and author: the dialog says the data is reused.
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    let consent = inst.consent_request(&staged).await.unwrap();
    assert!(consent.reuses_data);
    inst.commit(staged, consent.grant).await.unwrap();
    assert_eq!(rig.kv().await.as_deref(), Some(&b"v"[..]));
    assert!(
        rig.settings.resets.lock().unwrap().is_empty(),
        "config kept"
    );
    inst.uninstall(&id(), true).await.unwrap();

    // Another author with the same id: old data and settings entry go.
    let other = MANIFEST.replace("authors = [\"Ada\"]", "authors = [\"Mallory\"]");
    let staged = inst.stage_from_file(&rig.file(&package(&other))).unwrap();
    let consent = inst.consent_request(&staged).await.unwrap();
    assert!(!consent.reuses_data);
    inst.commit(staged, consent.grant).await.unwrap();
    assert_eq!(rig.kv().await, None);
    assert_eq!(*rig.settings.resets.lock().unwrap(), ["hello-plugin"]);
    rig.set_kv().await;
    inst.uninstall(&id(), true).await.unwrap();

    // Another source (the registry) with the same author: not reused either.
    let bytes = package(&other);
    let staged = inst
        .stage_from_bytes(&bytes, Sha256::digest(&bytes).into())
        .unwrap();
    let consent = inst.consent_request(&staged).await.unwrap();
    assert!(!consent.reuses_data);
    inst.commit(staged, consent.grant).await.unwrap();
    assert_eq!(rig.kv().await, None);
}

#[tokio::test]
async fn crash_between_rename_and_row_write_leaves_nothing() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    let grant = inst.consent_request(&staged).await.unwrap().grant;
    // Dies after the folder is in place, before the grant is written.
    rig.store.set("set_grants", Fault::Panic);
    crash(async move {
        let _ = inst.commit(staged, grant).await;
    })
    .await;
    rig.store.clear();
    assert!(rig.version_dir().is_dir(), "the crash left the folder");

    let mut fresh = rig.installer();
    let out = fresh
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert!(out.is_empty(), "{out:?}");
    assert!(entries(&rig.plugins).is_empty());
    assert!(plugin_rows(&rig).await.is_empty());
    // And it installs cleanly afterwards.
    rig.install(&mut fresh).await;
}

#[tokio::test]
async fn load_takes_the_grant_from_the_store() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    let mut narrow = inst.grant(&id()).cloned().unwrap();
    narrow.storage = false;
    narrow.network.clear();
    rig.store
        .set_grants("hello-plugin", grant_rows(&narrow, 1))
        .await
        .unwrap();

    let mut fresh = rig.installer();
    let out = fresh
        .load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert_eq!(out, [LoadOutcome::Loaded(id(), PluginState::Installed)]);
    assert_eq!(fresh.grant(&id()), Some(&narrow));
}

#[tokio::test]
async fn digest_checked_for_every_source() {
    for source in ["dev", "something-new"] {
        let rig = Rig::new();
        let mut inst = rig.installer();
        rig.install(&mut inst).await;
        let mut row = plugin_rows(&rig).await.remove(0);
        row.source = source.into();
        row.files_digest = None;
        rig.store.upsert_plugin(row).await.unwrap();
        let mut fresh = rig.installer();
        let out = fresh
            .load(&Default::default(), Instant::now())
            .await
            .unwrap();
        assert_eq!(out, [LoadOutcome::ChangedOnDisk(id())], "{source}");
    }
}

#[tokio::test]
async fn uninstall_store_failure_restores_folder_and_keeps_secrets() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    rig.install(&mut inst).await;
    rig.store.set("delete_plugin", Fault::Fail);
    let err = inst.uninstall(&id(), false).await.unwrap_err();
    assert!(matches!(err, InstallError::Store(_)), "{err:?}");
    assert!(rig.version_dir().join("plugin.wasm").is_file());
    assert_eq!(entries(&rig.plugins), ["hello-plugin"]);
    assert_eq!(plugin_rows(&rig).await.len(), 1);
    assert!(
        rig.secrets.removed.lock().unwrap().is_empty(),
        "store first"
    );
    assert!(inst.lifecycle(&id()).is_some());
    rig.store.clear();

    // A keychain failure after the store write still uninstalls and says so.
    rig.secrets.fail.store(true, Ordering::SeqCst);
    let done = inst.uninstall(&id(), false).await.unwrap();
    assert!(done.secrets_error.is_some());
    assert!(entries(&rig.plugins).is_empty());
    assert!(plugin_rows(&rig).await.is_empty());
}

#[tokio::test]
async fn commit_requires_the_resolved_absolute_exec_path() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    let mut grant = inst.consent_request(&staged).await.unwrap().grant;
    grant.set_exec_path("gh", PathBuf::from("/tmp/evil/gh"));
    let err = inst.commit(staged, grant).await.unwrap_err();
    assert!(matches!(err, InstallError::GrantMismatch), "{err:?}");

    // `gh` moved on the PATH between consent and Install.
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    let grant = inst.consent_request(&staged).await.unwrap().grant;
    *rig.gh.lock().unwrap() = Some(PathBuf::from("/usr/bin/gh"));
    let err = inst.commit(staged, grant).await.unwrap_err();
    assert!(matches!(err, InstallError::GrantMismatch), "{err:?}");

    // A resolver that answers a relative path is refused.
    *rig.gh.lock().unwrap() = Some(PathBuf::from("bin/gh"));
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    let grant = inst.consent_request(&staged).await.unwrap().grant;
    let err = inst.commit(staged, grant).await.unwrap_err();
    assert!(matches!(err, InstallError::ExecNotFound(_)), "{err:?}");
    assert!(entries(&rig.plugins).is_empty());
    assert!(plugin_rows(&rig).await.is_empty());
}

#[cfg(unix)]
#[test]
fn resolve_on_path_contract() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let d = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(d.path()).unwrap();
    let bin = root.join("bin");
    let links = root.join("links");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&links).unwrap();
    let gh = bin.join("gh");
    fs::write(&gh, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    symlink(&gh, links.join("gh")).unwrap();
    fs::write(bin.join("plain"), "x").unwrap();

    let path = |entries: &[&str]| std::env::join_paths(entries).unwrap();
    let links_s = links.to_str().unwrap();
    let bin_s = bin.to_str().unwrap();
    // Symlinks resolve to the canonical file.
    assert_eq!(resolve_on_path("gh", &path(&[links_s])), Some(gh.clone()));
    // `.`, empty and relative entries are skipped.
    assert_eq!(
        resolve_on_path("gh", &path(&[".", "", "bin", bin_s])),
        Some(gh.clone())
    );
    assert_eq!(resolve_on_path("gh", &path(&[".", "bin"])), None);
    // Not executable, missing, or a relative path with a slash: none.
    assert_eq!(resolve_on_path("plain", &path(&[bin_s])), None);
    assert_eq!(resolve_on_path("nope", &path(&[bin_s])), None);
    assert_eq!(resolve_on_path("bin/gh", &path(&[bin_s])), None);
    // An absolute program is canonicalised, PATH unused.
    assert_eq!(
        resolve_on_path(links.join("gh").to_str().unwrap(), &path(&[])),
        Some(gh)
    );
}

#[tokio::test]
async fn commit_refuses_a_blocked_plugin() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    let grant = inst.consent_request(&staged).await.unwrap().grant;
    *rig.blocked.0.lock().unwrap() = Some(BlockEntry {
        reason: "bad".into(),
        advisory: None,
        malicious: true,
    });
    let err = inst.commit(staged, grant).await.unwrap_err();
    assert!(matches!(err, InstallError::Blocked(_)), "{err:?}");
    assert!(entries(&rig.plugins).is_empty());
    assert!(plugin_rows(&rig).await.is_empty());
}

#[tokio::test]
async fn load_keeps_a_live_staging_folder() {
    let rig = Rig::new();
    let mut inst = rig.installer();
    let staged = inst.stage_from_file(&rig.file(&package(MANIFEST))).unwrap();
    inst.load(&Default::default(), Instant::now())
        .await
        .unwrap();
    assert_eq!(entries(&rig.plugins).len(), 1, "staging kept");
    let grant = inst.consent_request(&staged).await.unwrap().grant;
    inst.commit(staged, grant).await.unwrap();
    assert_eq!(entries(&rig.plugins), ["hello-plugin"]);
}
