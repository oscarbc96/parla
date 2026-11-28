//! `MediaFetcher` for the app: provider-owned URLs go through the owning
//! session's `ChatProvider::fetch` (host-checked, authenticated by the
//! provider); plain web URLs use a credential-free, https-only client.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use components::media::{DISK_CAP_BYTES, ImageCache, MediaError, MediaFetcher, prune_disk_cache};
use domain::{ChatProvider, ImageRef, ProviderError};
use futures::future::BoxFuture;
use gpui::App;
use reqwest::redirect::Policy;
use tokio::runtime::Handle;

const WEB_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: usize = 3;

pub struct Fetcher {
    provider: Arc<dyn ChatProvider>,
    web: reqwest::Client,
    allow_http: bool,
    /// Runtime that reqwest futures need; `None` runs them on the caller's
    /// executor (tests that already run inside Tokio).
    handle: Option<Handle>,
}

fn client(https_only: bool) -> reqwest::Client {
    // No cookie store, no default headers, no credentials of any kind.
    reqwest::Client::builder()
        .https_only(https_only)
        .timeout(WEB_TIMEOUT)
        .redirect(Policy::custom(move |a| {
            if a.previous().len() >= MAX_REDIRECTS || (https_only && a.url().scheme() != "https") {
                a.stop()
            } else {
                a.follow()
            }
        }))
        .build()
        .expect("static client configuration is valid")
}

fn map_provider(e: ProviderError) -> MediaError {
    match e {
        ProviderError::NotAllowed(_) => MediaError::Refused,
        ProviderError::TooLarge => MediaError::TooLarge,
        e => MediaError::Network(e.to_string()),
    }
}

async fn fetch_web(c: reqwest::Client, url: String, max_bytes: u64) -> Result<Vec<u8>, MediaError> {
    let mut resp = c
        .get(&url)
        .send()
        .await
        .map_err(|e| MediaError::Network(e.without_url().to_string()))?;
    if !resp.status().is_success() {
        return Err(MediaError::Network(format!("status {}", resp.status())));
    }
    if resp.content_length().is_some_and(|n| n > max_bytes) {
        return Err(MediaError::TooLarge);
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| MediaError::Network(e.without_url().to_string()))?
    {
        if (out.len() + chunk.len()) as u64 > max_bytes {
            return Err(MediaError::TooLarge);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Fetches a plain web URL for the app's own uploads (a picked GIF): https
/// only, the same credential-free client and redirect rules as image loading,
/// and at most `max_bytes`.
pub(crate) async fn fetch_web_bytes(
    url: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, components::media::MediaError> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| client(true)).clone();
    fetch_web(client, url.to_owned(), max_bytes).await
}

impl Fetcher {
    pub fn new(provider: Arc<dyn ChatProvider>, handle: Option<Handle>) -> Self {
        Self {
            provider,
            web: client(true),
            allow_http: false,
            handle,
        }
    }

    /// Plain-http client so tests can use a local mock server. Never used by the app.
    #[doc(hidden)]
    pub fn allow_http_for_tests(provider: Arc<dyn ChatProvider>) -> Self {
        Self {
            provider,
            web: client(false),
            allow_http: true,
            handle: None,
        }
    }
}

impl MediaFetcher for Fetcher {
    fn fetch(
        &self,
        src: &ImageRef,
        max_bytes: u64,
    ) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        let handle = self.handle.clone();
        let fut: BoxFuture<'static, Result<Vec<u8>, MediaError>> = match src {
            ImageRef::Provider(u) => {
                let (p, u) = (self.provider.clone(), u.clone());
                Box::pin(async move { p.fetch(&u, max_bytes).await.map_err(map_provider) })
            }
            ImageRef::Web(u) => {
                let scheme_ok =
                    u.starts_with("https://") || (self.allow_http && u.starts_with("http://"));
                if !scheme_ok {
                    return Box::pin(async { Err(MediaError::Refused) });
                }
                Box::pin(fetch_web(self.web.clone(), u.to_string(), max_bytes))
            }
        };
        match handle {
            Some(h) => spawn_aborting(&h, fut),
            None => fut,
        }
    }
}

/// Runs `fut` on the runtime behind `h`; dropping the returned future aborts
/// the task, so a closed viewer leaves no download running.
pub fn spawn_aborting(
    h: &Handle,
    fut: BoxFuture<'static, Result<Vec<u8>, MediaError>>,
) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
    struct AbortOnDrop(tokio::task::AbortHandle);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let task = h.spawn(fut);
    let guard = AbortOnDrop(task.abort_handle());
    Box::pin(async move {
        let _guard = guard;
        task.await
            .unwrap_or_else(|e| Err(MediaError::Network(e.to_string())))
    })
}

/// `<cache_root>/media`.
pub fn media_root(cache_root: &Path) -> PathBuf {
    cache_root.join("media")
}

/// Removes the oldest cached files beyond 256 MB. Call once at startup, before
/// any [`install`].
pub fn prune_at_startup(cache_root: &Path) {
    prune_disk_cache(&media_root(cache_root), DISK_CAP_BYTES);
}

/// `<cache>/media/<provider>/<id>/`: one workspace's cached files.
pub fn workspace_media_dir(cache_root: &Path, ws: &settings::WorkspaceKey) -> PathBuf {
    media_root(cache_root)
        .join(component(&ws.provider.0))
        .join(component(&ws.id.0))
}

fn component(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Registers the [`ImageCache`] global for the active workspace. Call from the
/// session wiring once the provider is up.
pub fn install(
    cx: &mut App,
    provider: Arc<dyn ChatProvider>,
    ws: &settings::WorkspaceKey,
    cache_root: &Path,
    image_cache_mb: u32,
) {
    let handle = crate::runtime::handle(cx);
    let dir = workspace_media_dir(cache_root, ws);
    let budget = image_cache_mb as usize * 1024 * 1024;
    let report = Arc::new(LoadLog::default());
    let (fail, ok) = (report.clone(), report);
    cx.set_global(
        ImageCache::new(Arc::new(Fetcher::new(provider, handle)), dir, budget)
            .with_failure_hook(Arc::new(move |e| fail.failed(e.kind(), Instant::now())))
            .with_success_hook(Arc::new(move || ok.loaded())),
    );
}

/// Logs image load outcomes at info: a failure kind at most once a minute, the
/// first success once. Never a URL, id or name.
#[derive(Default)]
pub(crate) struct LoadLog {
    last: std::sync::Mutex<std::collections::HashMap<&'static str, Instant>>,
    loaded: std::sync::atomic::AtomicBool,
}

impl LoadLog {
    const EVERY: Duration = Duration::from_secs(60);

    /// The line to log for a failure of `kind` at `now`, if one is due.
    pub(crate) fn due(&self, kind: &'static str, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap();
        match last.get(kind) {
            Some(t) if now.saturating_duration_since(*t) < Self::EVERY => false,
            _ => {
                last.insert(kind, now);
                true
            }
        }
    }

    fn failed(&self, kind: &'static str, now: Instant) {
        if self.due(kind, now) {
            crate::log::info(&format!("image load failed: {kind}"));
        }
    }

    fn loaded(&self) {
        if !self.loaded.swap(true, std::sync::atomic::Ordering::Relaxed) {
            crate::log::info("image load ok (first this session)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_kind_is_logged_once_a_minute() {
        let l = LoadLog::default();
        let t = Instant::now();
        assert!(l.due("network", t));
        assert!(!l.due("network", t + Duration::from_secs(59)));
        assert!(l.due("refused", t + Duration::from_secs(1)));
        assert!(l.due("network", t + Duration::from_secs(61)));
    }
}
