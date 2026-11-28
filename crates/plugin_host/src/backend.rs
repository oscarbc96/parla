//! What a plugin host call needs from the app: the transport, the keychain
//! and the settings store, behind one object-safe trait. Plain owned data
//! only, so `plugin_host` stays free of the app's crates.

#[cfg(any(test, feature = "test-support"))]
use std::collections::BTreeMap;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Mutex;

use futures::future::BoxFuture;

use crate::manifest::PluginId;

/// One HTTP request, as a plugin asked for it and as the host passes it on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// Already clamped by the host to 30,000 ms; never zero.
    pub timeout_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    /// The wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

/// The backend's answer to one request (redirects are not followed there).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// What `host.get-app-info` reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppInfo {
    pub app_version: String,
    pub platform: String,
    pub locale: String,
    pub appearance: Appearance,
    pub density: Density,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Density {
    Compact,
    Cozy,
}

/// A declared setting's value; the host maps it to the WIT variant.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    Text(String),
    Number(f64),
    Toggle(bool),
    Choice(String),
}

/// A host call's failure, mapped one to one onto the WIT `error` variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    #[error("denied: {0}")]
    Denied(String),
    #[error("unsupported")]
    Unsupported,
    #[error("not found")]
    NotFound,
    #[error("limit: {0}")]
    Limit(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("offline")]
    Offline,
    #[error("cancelled")]
    Cancelled,
    #[error("failed: {0}")]
    Failed(String),
}

/// The app side of the host API. Futures are boxed like
/// [`store::AppStore`](store::AppStore)'s, so implementations stay object
/// safe. A `plugin` argument namespaces every call to one plugin.
pub trait HostBackend: Send + Sync + 'static {
    /// The running app's version, platform, locale and appearance.
    fn app_info(&self) -> AppInfo;

    /// Sends one request. The implementation does not follow redirects and
    /// does not add cookies. It sets `User-Agent: Parla-Plugin/<id>`, reads at
    /// most `max_body` bytes of the body and returns [`ApiError::Limit`]
    /// naming `response body` when there are more.
    fn http_fetch(
        &self,
        plugin: &PluginId,
        request: HttpRequest,
        max_body: usize,
    ) -> BoxFuture<'_, Result<HttpResponse, ApiError>>;

    fn secret_get(
        &self,
        plugin: &PluginId,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<String>, ApiError>>;

    fn secret_set(
        &self,
        plugin: &PluginId,
        key: &str,
        value: &str,
    ) -> BoxFuture<'_, Result<(), ApiError>>;

    fn secret_delete(&self, plugin: &PluginId, key: &str) -> BoxFuture<'_, Result<(), ApiError>>;

    /// Every key the plugin has, sorted; the host caps the count.
    fn secret_keys(&self, plugin: &PluginId) -> BoxFuture<'_, Result<Vec<String>, ApiError>>;

    /// One declared setting, or `None` when it has no value yet.
    fn setting(
        &self,
        plugin: &PluginId,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<SettingValue>, ApiError>>;
}

/// A backend that refuses every call, for instances made without the app
/// (the engine's own tests).
pub struct RefusingBackend;

impl HostBackend for RefusingBackend {
    fn app_info(&self) -> AppInfo {
        AppInfo {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            platform: std::env::consts::OS.to_owned(),
            locale: "en".to_owned(),
            appearance: Appearance::Light,
            density: Density::Cozy,
        }
    }

    fn http_fetch(
        &self,
        _plugin: &PluginId,
        _request: HttpRequest,
        _max_body: usize,
    ) -> BoxFuture<'_, Result<HttpResponse, ApiError>> {
        Box::pin(async { Err(ApiError::Failed("no plugin backend".to_owned())) })
    }

    fn secret_get(
        &self,
        _plugin: &PluginId,
        _key: &str,
    ) -> BoxFuture<'_, Result<Option<String>, ApiError>> {
        Box::pin(async { Err(ApiError::Failed("no plugin backend".to_owned())) })
    }

    fn secret_set(
        &self,
        _plugin: &PluginId,
        _key: &str,
        _value: &str,
    ) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(async { Err(ApiError::Failed("no plugin backend".to_owned())) })
    }

    fn secret_delete(&self, _plugin: &PluginId, _key: &str) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(async { Err(ApiError::Failed("no plugin backend".to_owned())) })
    }

    fn secret_keys(&self, _plugin: &PluginId) -> BoxFuture<'_, Result<Vec<String>, ApiError>> {
        Box::pin(async { Err(ApiError::Failed("no plugin backend".to_owned())) })
    }

    fn setting(
        &self,
        _plugin: &PluginId,
        _key: &str,
    ) -> BoxFuture<'_, Result<Option<SettingValue>, ApiError>> {
        Box::pin(async { Err(ApiError::Failed("no plugin backend".to_owned())) })
    }
}

/// A scripted in-memory backend: HTTP answers come from a queue, secrets and
/// settings from maps keyed by plugin.
#[cfg(any(test, feature = "test-support"))]
pub struct MemoryBackend {
    state: Mutex<MemoryState>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct MemoryState {
    requests: Vec<HttpRequest>,
    responses: std::collections::VecDeque<Result<HttpResponse, ApiError>>,
    secrets: BTreeMap<(String, String), String>,
    settings: BTreeMap<(String, String), SettingValue>,
}

#[cfg(any(test, feature = "test-support"))]
impl MemoryBackend {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryState::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Queues one HTTP answer. With no answer left the backend replies 200
    /// with an empty body.
    pub fn script(&self, status: u16, headers: Vec<(String, String)>, body: Vec<u8>) {
        self.lock().responses.push_back(Ok(HttpResponse {
            status,
            headers,
            body,
        }));
    }

    /// Queues one HTTP error.
    pub fn script_error(&self, error: ApiError) {
        self.lock().responses.push_back(Err(error));
    }

    /// Every request that reached the backend, in order.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.lock().requests.clone()
    }

    pub fn set_secret(&self, plugin: &str, key: &str, value: &str) {
        self.lock()
            .secrets
            .insert((plugin.to_owned(), key.to_owned()), value.to_owned());
    }

    pub fn set_setting(&self, plugin: &str, key: &str, value: SettingValue) {
        self.lock()
            .settings
            .insert((plugin.to_owned(), key.to_owned()), value);
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl HostBackend for MemoryBackend {
    fn app_info(&self) -> AppInfo {
        AppInfo {
            app_version: "0.0.0-test".to_owned(),
            platform: "test".to_owned(),
            locale: "en".to_owned(),
            appearance: Appearance::Dark,
            density: Density::Cozy,
        }
    }

    fn http_fetch(
        &self,
        _plugin: &PluginId,
        request: HttpRequest,
        _max_body: usize,
    ) -> BoxFuture<'_, Result<HttpResponse, ApiError>> {
        let mut state = self.lock();
        state.requests.push(request);
        let answer = state.responses.pop_front().unwrap_or(Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
        }));
        Box::pin(async move { answer })
    }

    fn secret_get(
        &self,
        plugin: &PluginId,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<String>, ApiError>> {
        let value = self
            .lock()
            .secrets
            .get(&(plugin.as_str().to_owned(), key.to_owned()))
            .cloned();
        Box::pin(async move { Ok(value) })
    }

    fn secret_set(
        &self,
        plugin: &PluginId,
        key: &str,
        value: &str,
    ) -> BoxFuture<'_, Result<(), ApiError>> {
        self.lock().secrets.insert(
            (plugin.as_str().to_owned(), key.to_owned()),
            value.to_owned(),
        );
        Box::pin(async { Ok(()) })
    }

    fn secret_delete(&self, plugin: &PluginId, key: &str) -> BoxFuture<'_, Result<(), ApiError>> {
        self.lock()
            .secrets
            .remove(&(plugin.as_str().to_owned(), key.to_owned()));
        Box::pin(async { Ok(()) })
    }

    fn secret_keys(&self, plugin: &PluginId) -> BoxFuture<'_, Result<Vec<String>, ApiError>> {
        let keys: Vec<String> = self
            .lock()
            .secrets
            .keys()
            .filter(|(id, _)| id == plugin.as_str())
            .map(|(_, key)| key.clone())
            .collect();
        Box::pin(async move { Ok(keys) })
    }

    fn setting(
        &self,
        plugin: &PluginId,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<SettingValue>, ApiError>> {
        let value = self
            .lock()
            .settings
            .get(&(plugin.as_str().to_owned(), key.to_owned()))
            .cloned();
        Box::pin(async move { Ok(value) })
    }
}
