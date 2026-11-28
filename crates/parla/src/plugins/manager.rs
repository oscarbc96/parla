//! The built-in GIF providers in the app: the manager that loads their wasm
//! components, runs one instance per plugin and answers the picker.
//!
//! - Created at startup as a GPUI global, doing no work then: no engine, no
//!   file access, no thread. The wasmtime engine comes with the first call.
//! - One instance per plugin behind an async mutex, so calls to one provider
//!   are serialised. `activate(invoked)` runs once per instance. A trapped
//!   instance is dropped and the next call makes a new one; three crashes in
//!   a row within a minute stop the retries until the picker is closed.
//! - The embedded wasm is staged under `<cache>/plugins/builtin/` (writing it
//!   only when it differs) and compiled through [`PrecompileCache`], whose
//!   child process is this same binary (`__plugin-precompile`).
//! - Nothing runs until the user accepts the set-up card: a provider is
//!   `Ready` only when `plugins.<id>.enabled` is true and it has an
//!   `api_key` secret.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use components::gif_picker::{
    GifError, GifItem, GifPage, GifSource, GifSources, GifTab, GifTabState,
};
use gpui::{App, Global, SharedString, Task};
use plugin_host::backend::{AppInfo, Appearance, Density};
use plugin_host::cache::{
    Compiler, CwasmHashes, MemoryCwasmHashes, PRECOMPILE_SUBCOMMAND, PrecompileCache,
};
use plugin_host::caps::Grant;
use plugin_host::context::PluginContext;
use plugin_host::engine::{PluginEngine, PluginInstance};
use plugin_host::install::SettingsFile;
use plugin_host::media::{self, ContentRating, MediaError, MediaItem, MediaPage};
use plugin_host::wasmtime::component::Val;
use settings::{ConfigPaths, GifSettings, PluginsSettings, Settings, SettingsStore};
use tokio::sync::Mutex as AsyncMutex;

use super::backend::{AppBackend, HttpTransport, Transport};
use super::builtin::{self, Builtin};
use super::secrets::{PluginSecretStore, Secrets};

/// The secret the providers read.
pub const API_KEY: &str = "api_key";
/// Items per page (the picker's grid).
pub const PAGE_SIZE: u32 = 24;
/// Crashes in a row that stop the retries.
const CRASH_LIMIT: u32 = 3;
/// The window the crash counter counts over.
const CRASH_WINDOW: Duration = Duration::from_secs(60);
/// `activate`'s deadline (`lifecycle.activate` in the plugin API).
const ACTIVATE_DEADLINE: Duration = Duration::from_secs(1);

/// Where the manager works and how it names itself to the plugins.
#[derive(Clone, Debug)]
pub struct ManagerConfig {
    /// `$CACHE/parla`: the staged wasm and compiled components live below it.
    pub cache_dir: PathBuf,
    /// `settings.json`, where `plugins.<id>.enabled` is written.
    pub settings_file: PathBuf,
    pub app_version: String,
    pub platform: String,
    pub locale: String,
    /// The program that runs the hidden precompile subcommand. `None` is the
    /// running binary; tests point it at `CARGO_BIN_EXE_parla` because the
    /// test harness is not the app.
    pub compiler_program: Option<PathBuf>,
}

impl ManagerConfig {
    pub fn new(cache_dir: impl Into<PathBuf>, settings_file: impl Into<PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.into(),
            settings_file: settings_file.into(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            platform: std::env::consts::OS.to_owned(),
            locale: "en".to_owned(),
            compiler_program: None,
        }
    }

    pub fn with_compiler_program(mut self, program: impl Into<PathBuf>) -> Self {
        self.compiler_program = Some(program.into());
        self
    }
}

/// A provider as the picker and the settings page show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderInfo {
    pub id: String,
    /// The service's own spelling (`GIPHY`, `KLIPY`).
    pub title: String,
    /// The hosts it talks to, from the manifest.
    pub hosts: Vec<String>,
    /// Where the user creates a key (https).
    pub key_url: Option<String>,
    pub state: ProviderState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderState {
    Ready,
    NeedsSetup,
}

/// One loaded instance and the settings entry it was made for.
struct Loaded {
    instance: PluginInstance,
    entry: String,
}

/// Three crashes in a row within a minute stop the retries.
#[derive(Default)]
struct Crashes {
    first: Option<Instant>,
    count: u32,
    blocked: bool,
}

impl Crashes {
    /// Counts one crash; returns whether retries are now blocked.
    fn record(&mut self) -> bool {
        let now = Instant::now();
        match self.first {
            Some(first) if now.duration_since(first) <= CRASH_WINDOW => self.count += 1,
            _ => {
                self.first = Some(now);
                self.count = 1;
            }
        }
        if self.count >= CRASH_LIMIT {
            self.blocked = true;
        }
        self.blocked
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// One plugin's instance slot and crash counter.
struct Slot {
    loaded: AsyncMutex<Option<Loaded>>,
    crashes: Mutex<Crashes>,
}

impl Slot {
    fn new() -> Self {
        Self {
            loaded: AsyncMutex::new(None),
            crashes: Mutex::new(Crashes::default()),
        }
    }

    fn crash(&self) -> bool {
        self.crashes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record()
    }

    fn blocked(&self) -> bool {
        self.crashes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .blocked
    }

    fn survived(&self) {
        self.crashes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

struct State {
    config: ManagerConfig,
    transport: Arc<dyn Transport>,
    engine: PluginEngine,
    cache: OnceLock<Result<PrecompileCache, String>>,
    slots: Mutex<HashMap<&'static str, Arc<Slot>>>,
    /// Whether a provider's `api_key` exists, as last read (`None` unknown).
    keys: Mutex<HashMap<&'static str, bool>>,
}

/// The app's GIF providers. Cheap to clone; the work sits in one shared state.
#[derive(Clone)]
pub struct PluginManager {
    state: Arc<State>,
    secrets: Arc<dyn Secrets>,
}

impl Global for PluginManager {}

impl PluginManager {
    /// Builds the manager without touching the disk, the engine or the
    /// network.
    pub fn with_parts(
        config: ManagerConfig,
        secrets: Arc<dyn Secrets>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            state: Arc::new(State {
                config,
                transport,
                engine: PluginEngine::new(),
                cache: OnceLock::new(),
                slots: Mutex::new(HashMap::new()),
                keys: Mutex::new(HashMap::new()),
            }),
            secrets,
        }
    }

    /// True once the wasmtime engine exists.
    pub fn engine_created(&self) -> bool {
        self.state.engine.is_created()
    }

    /// Instances loaded now (0 or 1 per plugin).
    pub fn instance_count(&self, id: &str) -> usize {
        let Some(slot) = self.slot_of(id) else {
            return 0;
        };
        match slot.loaded.try_lock() {
            Ok(loaded) => usize::from(loaded.is_some()),
            // A call holds it: there is an instance.
            Err(_) => 1,
        }
    }

    /// Whether `id`'s `api_key` is known to exist (`None` until read).
    pub fn key_known(&self, id: &str) -> Option<bool> {
        self.state
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .copied()
    }

    /// The bundled providers in `gifs.providers` order, reading each one's
    /// key from the secret store.
    pub async fn providers(
        &self,
        gifs: &GifSettings,
        plugins: &PluginsSettings,
    ) -> Vec<ProviderInfo> {
        let mut out = Vec::new();
        for id in &gifs.providers {
            let Some(b) = builtin::get(id) else {
                continue;
            };
            if !b.bundled() {
                continue;
            }
            let has_key = self
                .secrets
                .keys(id)
                .await
                .is_ok_and(|keys| keys.iter().any(|k| k == API_KEY));
            self.state
                .keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(b.id, has_key);
            out.push(self.info(b, enabled(plugins, id) && has_key));
        }
        out
    }

    /// [`providers`](Self::providers) for render, without the secret store: an
    /// enabled provider whose key was not read yet is `Ready`, and a missing
    /// key comes back as [`GifError::NeedsSetup`] from [`load`](Self::load).
    pub fn providers_cached(
        &self,
        gifs: &GifSettings,
        plugins: &PluginsSettings,
    ) -> Vec<ProviderInfo> {
        let mut out = Vec::new();
        for id in &gifs.providers {
            let Some(b) = builtin::get(id) else {
                continue;
            };
            if !b.bundled() {
                continue;
            }
            let has_key = self.key_known(id);
            out.push(self.info(b, enabled(plugins, id) && has_key != Some(false)));
        }
        out
    }

    /// Reads every bundled provider's key names once (the settings page and
    /// the picker call it when they first open).
    pub async fn prime_keys(&self) {
        for b in builtin::all() {
            if !b.bundled() {
                continue;
            }
            let has_key = self
                .secrets
                .keys(b.id)
                .await
                .is_ok_and(|keys| keys.iter().any(|k| k == API_KEY));
            self.state
                .keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(b.id, has_key);
        }
    }

    /// Stores `key` (an empty one is ignored) and enables the provider. The
    /// settings file watcher applies `plugins.<id>.enabled` like a hand edit.
    pub async fn set_up(&self, id: &str, key: &str) -> Result<(), GifError> {
        let Some(b) = builtin::get(id).filter(|b| b.bundled()) else {
            return Err(GifError::NeedsSetup);
        };
        let key = key.trim();
        if !key.is_empty() {
            self.secrets
                .set(b.id, API_KEY, domain::SecretText::new(key))
                .await
                .map_err(GifError::Failed)?;
            self.state
                .keys
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(b.id, true);
        }
        set_enabled(&self.state.config.settings_file, b.id, true).map_err(GifError::Failed)
    }

    /// Removes the provider's key (the switch stays as it is; the tab then
    /// asks for set-up again).
    pub async fn remove_key(&self, id: &str) -> Result<(), GifError> {
        let Some(b) = builtin::get(id).filter(|b| b.bundled()) else {
            return Err(GifError::NeedsSetup);
        };
        self.secrets
            .delete(b.id, API_KEY)
            .await
            .map_err(GifError::Failed)?;
        self.state
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(b.id, false);
        Ok(())
    }

    /// One page: `media::trending` when `query` is blank, else
    /// `media::search`.
    pub async fn load(
        &self,
        id: &str,
        query: Option<&str>,
        cursor: Option<&str>,
        gifs: &GifSettings,
        plugins: &PluginsSettings,
    ) -> Result<GifPage, GifError> {
        let Some(b) = builtin::get(id).filter(|b| b.bundled()) else {
            return Err(GifError::NeedsSetup);
        };
        if !enabled(plugins, id) {
            return Err(GifError::NeedsSetup);
        }
        let slot = self.slot(b.id);
        if slot.blocked() {
            return Err(GifError::Failed(format!(
                "the {id} provider keeps crashing; close the picker to try again"
            )));
        }
        let entry = entry_fingerprint(plugins, id);
        let mut guard = slot.loaded.lock().await;
        if guard
            .as_ref()
            .is_none_or(|loaded| loaded.instance.trapped())
        {
            *guard = None;
        }
        if guard.as_ref().is_some_and(|loaded| loaded.entry != entry) {
            // The plugin's settings changed: a new instance reads them.
            *guard = None;
        }
        if guard.is_none() {
            match self.build(b, plugins).await {
                Ok(loaded) => *guard = Some(loaded),
                Err(e) => {
                    fail(&slot, &e);
                    return Err(e);
                }
            }
        }
        let loaded = guard.as_mut().expect("made above");
        let rating = ContentRating::from_setting(&gifs.rating);
        let query = query.filter(|q| !q.trim().is_empty());
        let page = match query {
            Some(query) => {
                media::search(
                    &self.state.engine,
                    &mut loaded.instance,
                    b.media_id(),
                    query,
                    cursor,
                    PAGE_SIZE,
                    rating,
                )
                .await
            }
            None => {
                media::trending(
                    &self.state.engine,
                    &mut loaded.instance,
                    b.media_id(),
                    cursor,
                    PAGE_SIZE,
                    rating,
                )
                .await
            }
        };
        match page {
            Ok(page) => {
                slot.survived();
                Ok(gif_page(page))
            }
            Err(e) => {
                let trapped = loaded.instance.trapped();
                drop(guard);
                if trapped {
                    *slot.loaded.lock().await = None;
                }
                let mapped = map_error(id, e);
                if trapped {
                    fail(&slot, &mapped);
                }
                Err(mapped)
            }
        }
    }

    /// The picker closed: drop the instances (the engine stays) and clear the
    /// crash counters, so reopening retries.
    pub fn closed(&self) {
        self.state
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    // ------------------------------------------------------------ internals

    fn slot(&self, id: &'static str) -> Arc<Slot> {
        let mut slots = self
            .state
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slots
            .entry(id)
            .or_insert_with(|| Arc::new(Slot::new()))
            .clone()
    }

    fn slot_of(&self, id: &str) -> Option<Arc<Slot>> {
        self.state
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    fn info(&self, b: &Builtin, ready: bool) -> ProviderInfo {
        let manifest = b.manifest();
        ProviderInfo {
            id: b.id.to_owned(),
            title: b.title(),
            hosts: manifest
                .capabilities
                .network
                .iter()
                .map(|h| h.as_str().to_owned())
                .collect(),
            key_url: Some(b.key_url.to_owned()),
            state: if ready {
                ProviderState::Ready
            } else {
                ProviderState::NeedsSetup
            },
        }
    }

    fn app_info(&self) -> AppInfo {
        AppInfo {
            app_version: self.state.config.app_version.clone(),
            platform: self.state.config.platform.clone(),
            locale: self.state.config.locale.clone(),
            // The picker's appearance does not change what a provider asks
            // for; the host API needs a value.
            appearance: Appearance::Light,
            density: Density::Cozy,
        }
    }

    /// The compiled-component cache, made on first use. Its hashes live in
    /// memory for now: a new launch compiles each built-in once.
    fn cache(&self) -> Result<&PrecompileCache, GifError> {
        let cache = self.state.cache.get_or_init(|| {
            let compiler = match &self.state.config.compiler_program {
                Some(program) => Compiler::new(program).args([PRECOMPILE_SUBCOMMAND]),
                None => Compiler::current_exe()
                    .map_err(|e| format!("cannot name the app binary: {e}"))?
                    .args([PRECOMPILE_SUBCOMMAND]),
            };
            // TODO(app.db): keep the `.cwasm` hashes across launches with a
            // store-backed `CwasmHashes`; in memory, each launch compiles the
            // built-ins once (about 0.2 s each in the child process).
            let hashes: Arc<dyn CwasmHashes> = Arc::new(MemoryCwasmHashes::default());
            Ok(PrecompileCache::new(
                self.state.config.cache_dir.join("plugins/compiled"),
                compiler,
                hashes,
            ))
        });
        cache.as_ref().map_err(|e| {
            crate::log::warn(&format!("gif plugins: {e}"));
            GifError::Failed(e.clone())
        })
    }

    /// Writes the embedded wasm under `<cache>/plugins/builtin/` when it is
    /// absent or differs, and returns its path.
    fn stage_wasm(&self, b: &Builtin) -> Result<PathBuf, GifError> {
        let dir = self.state.config.cache_dir.join("plugins/builtin");
        let path = dir.join(format!("{}-{}.wasm", b.id, b.version()));
        if std::fs::read(&path).is_ok_and(|old| old == b.wasm) {
            return Ok(path);
        }
        std::fs::create_dir_all(&dir)
            .map_err(|e| GifError::Failed(format!("cannot create the plugin cache: {e}")))?;
        let tmp = dir.join(format!(".{}-{}.tmp", b.id, std::process::id()));
        std::fs::write(&tmp, b.wasm)
            .map_err(|e| GifError::Failed(format!("cannot stage the plugin: {e}")))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            GifError::Failed(format!("cannot stage the plugin: {e}"))
        })?;
        Ok(path)
    }

    /// Loads the component, instantiates it with the manifest's grant and
    /// calls `activate(invoked)`.
    async fn build(&self, b: &Builtin, plugins: &PluginsSettings) -> Result<Loaded, GifError> {
        let manifest = b.manifest().clone();
        let version = manifest.version.to_string();
        let wasm = self.stage_wasm(b)?;
        let cache = self.cache()?;
        let component = self
            .state
            .engine
            .load(cache, b.id, &version, &wasm)
            .map_err(|e| {
                crate::log::warn(&format!("gif plugin {}: {e}", b.id));
                GifError::Failed(format!("{e}"))
            })?;
        let grant = Arc::new(Grant::from_manifest(&manifest));
        let backend = Arc::new(AppBackend::new(
            manifest.clone(),
            self.state.transport.clone(),
            self.secrets.clone(),
            plugins.clone(),
            self.app_info(),
        ));
        let context = PluginContext::new(manifest.id.clone(), manifest.clone(), grant, backend);
        let mut instance = self
            .state
            .engine
            .instantiate_with(&component, context)
            .await
            .map_err(|e| {
                crate::log::warn(&format!("gif plugin {}: {e}", b.id));
                GifError::Failed(format!("{e}"))
            })?;
        activate(&self.state.engine, &mut instance, b.id).await?;
        Ok(Loaded {
            instance,
            entry: entry_fingerprint(plugins, b.id),
        })
    }
}

/// The manifest host list. Kept here so `providers_cached` does not parse.
fn enabled(plugins: &PluginsSettings, id: &str) -> bool {
    // A built-in is offered, not enabled: only an explicit `enabled = true`
    // runs it (unlike an installed plugin, where a missing entry means on).
    plugins.entry(id).is_some_and(|entry| entry.enabled)
}

/// The `plugins.<id>` entry as text: a changed one makes a new instance.
fn entry_fingerprint(plugins: &PluginsSettings, id: &str) -> String {
    serde_json::to_string(&plugins.entry(id)).unwrap_or_default()
}

fn set_enabled(file: &std::path::Path, id: &str, on: bool) -> Result<(), String> {
    use plugin_host::install::SettingsWriter as _;
    SettingsFile(file.to_path_buf())
        .set_enabled(id, on)
        .map_err(|e| format!("cannot save the plugin setting: {e}"))
}

/// Counts `e` as a crash when the instance has to be dropped.
fn fail(slot: &Slot, _e: &GifError) {
    slot.crash();
}

async fn activate(
    engine: &PluginEngine,
    instance: &mut PluginInstance,
    id: &str,
) -> Result<(), GifError> {
    let Some(func) = instance.export_func("parla:plugin/lifecycle@0.1.0", "activate") else {
        return Err(GifError::Failed("the provider has no lifecycle".to_owned()));
    };
    let mut results = [Val::Result(Err(None))];
    engine
        .call(
            instance,
            func,
            &[Val::Enum("invoked".to_owned())],
            &mut results,
            ACTIVATE_DEADLINE,
        )
        .await
        .map_err(|e| {
            crate::log::warn(&format!("gif plugin {id}: activate: {e}"));
            GifError::Failed(format!("the provider did not start: {e}"))
        })?;
    match results {
        [Val::Result(Ok(_))] => Ok(()),
        _ => Err(GifError::Failed("the provider did not start".to_owned())),
    }
}

/// `MediaError` as the picker's error. Anything else is logged without a URL
/// or a key and shown as a generic failure.
fn map_error(id: &str, e: MediaError) -> GifError {
    match e {
        MediaError::Invalid(text) if text == "api key" => GifError::NeedsSetup,
        MediaError::Offline => GifError::Offline,
        MediaError::Limit(_) => GifError::RateLimited,
        other => {
            crate::log::warn(&format!("gif provider {id}: {other}"));
            GifError::Failed(other.to_string())
        }
    }
}

fn gif_page(page: MediaPage) -> GifPage {
    GifPage {
        items: page.items.into_iter().map(gif_item).collect(),
        next: page.next.map(SharedString::from),
    }
}

fn gif_item(item: MediaItem) -> GifItem {
    GifItem {
        id: item.id.into(),
        preview_url: item.preview_url.into(),
        full_url: item.full_url.into(),
        width: item.width,
        height: item.height,
        bytes: item.bytes,
        title: item.title.into(),
        attribution: item.attribution.map(SharedString::from),
    }
}

// ------------------------------------------------------------------- app

/// The [`GifSource`] the app installs over [`PluginManager`].
pub struct GifSourceHandle(pub PluginManager);

impl GifSource for GifSourceHandle {
    fn tabs(&self, cx: &App) -> Vec<GifTab> {
        let settings = current_settings(cx);
        self.0
            .providers_cached(&settings.gifs, &settings.plugins)
            .into_iter()
            .map(|p| GifTab {
                id: p.id.into(),
                title: p.title.into(),
                state: match p.state {
                    ProviderState::Ready => GifTabState::Ready,
                    ProviderState::NeedsSetup => GifTabState::NeedsSetup,
                },
                hosts: p.hosts.into_iter().map(SharedString::from).collect(),
                key_url: p.key_url.map(SharedString::from),
            })
            .collect()
    }

    fn load(
        &self,
        tab: &str,
        query: Option<String>,
        cursor: Option<SharedString>,
        cx: &mut App,
    ) -> Task<Result<GifPage, GifError>> {
        let manager = self.0.clone();
        let settings = current_settings(cx);
        let handle = crate::runtime::handle(cx);
        let (id, cursor) = (tab.to_owned(), cursor.map(String::from));
        cx.spawn(async move |_| {
            let future = async move {
                manager
                    .load(
                        &id,
                        query.as_deref(),
                        cursor.as_deref(),
                        &settings.gifs,
                        &settings.plugins,
                    )
                    .await
            };
            on_runtime(handle, future).await
        })
    }

    fn set_up(&self, tab: &str, api_key: String, cx: &mut App) -> Task<Result<(), GifError>> {
        let manager = self.0.clone();
        let handle = crate::runtime::handle(cx);
        let id = tab.to_owned();
        cx.spawn(async move |_| {
            let future = async move { manager.set_up(&id, &api_key).await };
            on_runtime(handle, future).await
        })
    }

    fn closed(&self, _cx: &mut App) {
        self.0.closed();
    }
}

/// Runs `future` on the app's Tokio runtime when there is one, else on the
/// caller's executor (tests).
async fn on_runtime<T: Send + 'static>(
    handle: Option<tokio::runtime::Handle>,
    future: impl Future<Output = T> + Send + 'static,
) -> T {
    match handle {
        Some(handle) => handle
            .spawn(future)
            .await
            .unwrap_or_else(|e| panic!("the GIF task panicked: {e}")),
        None => future.await,
    }
}

fn current_settings(cx: &App) -> Settings {
    match cx.try_global::<SettingsStore>() {
        Some(store) => store.current().clone(),
        None => Settings::default(),
    }
}

/// Installs the manager and the picker's source. No work happens here: the
/// engine, the cache and the secrets are created on the first call.
pub fn install(cx: &mut App, paths: &ConfigPaths) {
    if !builtin::any_bundled() {
        return;
    }
    let mut config = ManagerConfig::new(settings::paths::cache_dir(), paths.settings.clone());
    config.locale = i18n::gpui::localizer(cx).locale().to_string();
    let secrets: Arc<dyn Secrets> = match crate::dev_credentials::current() {
        crate::dev_credentials::Backend::DevFile => Arc::new(PluginSecretStore::dev(
            crate::dev_credentials::DevFile::shared(
                settings::paths::data_dir().join(crate::dev_credentials::FILE_NAME),
            ),
        )),
        crate::dev_credentials::Backend::Keychain => Arc::new(PluginSecretStore::keychain(cx)),
    };
    let manager = PluginManager::with_parts(config, secrets, Arc::new(HttpTransport::new()));
    cx.set_global(manager.clone());
    cx.set_global(GifSources(Arc::new(GifSourceHandle(manager))));
}
