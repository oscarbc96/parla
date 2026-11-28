//! What one instantiated plugin may do: its id, the manifest it shipped, the
//! grant the user accepted, the app backend and the per-instance HTTP limits.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::backend::{ApiError, HostBackend, RefusingBackend};
use crate::caps::Grant;
use crate::manifest::{Manifest, ManifestSource, PluginId, parse};

/// Requests one instance may have in flight.
pub const HTTP_CONCURRENT: usize = 4;
/// Requests one instance may make per minute.
pub const HTTP_PER_MINUTE: u32 = 120;
/// The window [`HTTP_PER_MINUTE`] counts over.
const HTTP_WINDOW: Duration = Duration::from_secs(60);

/// Everything a host call of one instance needs besides the call itself.
/// One context goes to one instance; a clone shares its HTTP limiter.
#[derive(Clone)]
pub struct PluginContext {
    /// The plugin's id: namespaces secrets and names the `User-Agent`.
    pub id: PluginId,
    /// The grant checked on every host call.
    pub grant: Arc<Grant>,
    /// The manifest, for declared settings and the memory cap.
    pub manifest: Arc<Manifest>,
    /// The app side of the host API.
    pub backend: Arc<dyn HostBackend>,
    /// `[limits] memory_mb`; `None` is the default 16 MiB.
    pub memory_mb: Option<u32>,
    pub(crate) http: Arc<HttpLimiter>,
}

impl PluginContext {
    /// A context for `manifest`'s plugin with the user's `grant`.
    pub fn new(
        id: PluginId,
        manifest: Arc<Manifest>,
        grant: Arc<Grant>,
        backend: Arc<dyn HostBackend>,
    ) -> Self {
        Self {
            id,
            memory_mb: Some(manifest.limits.memory_mb),
            manifest,
            grant,
            backend,
            http: Arc::new(HttpLimiter::new()),
        }
    }

    /// A context with `memory_mb`, an empty grant and a backend that refuses
    /// everything: what [`PluginEngine::instantiate`](crate::engine::PluginEngine::instantiate)
    /// gives an instance made without the app.
    pub fn with_memory(memory_mb: Option<u32>) -> Self {
        Self {
            memory_mb,
            ..Self::default()
        }
    }
}

impl Default for PluginContext {
    fn default() -> Self {
        Self {
            id: plugin_host_id(),
            grant: Arc::new(Grant::default()),
            manifest: default_manifest(),
            backend: Arc::new(RefusingBackend),
            memory_mb: None,
            http: Arc::new(HttpLimiter::new()),
        }
    }
}

/// The id the host's own test instances carry.
const HOST_ID: &str = "plugin-host";

fn plugin_host_id() -> PluginId {
    PluginId::parse(HOST_ID).expect("valid plugin id")
}

/// The empty manifest an instance without a plugin uses.
fn default_manifest() -> Arc<Manifest> {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    let manifest = MANIFEST.get_or_init(|| {
        parse(
            "id = \"plugin-host\"\nname = \"Plugin host\"\nversion = \"0.0.0\"\napi = \"0.1\"\n",
            ManifestSource::Package,
        )
        .expect("the host's own manifest is valid")
    });
    Arc::new(manifest.clone())
}

/// One instance's HTTP concurrency and rate counters. Bounded: a counter and
/// one window, reset in place.
pub(crate) struct HttpLimiter {
    concurrent: AtomicUsize,
    window: Mutex<Window>,
}

struct Window {
    start: Instant,
    used: u32,
}

impl HttpLimiter {
    pub(crate) fn new() -> Self {
        Self {
            concurrent: AtomicUsize::new(0),
            window: Mutex::new(Window {
                start: Instant::now(),
                used: 0,
            }),
        }
    }

    /// Reserves one of the [`HTTP_CONCURRENT`] slots until the permit drops.
    pub(crate) fn permit(self: &Arc<Self>) -> Result<HttpPermit, ApiError> {
        let now = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
        if now > HTTP_CONCURRENT {
            self.concurrent.fetch_sub(1, Ordering::SeqCst);
            return Err(ApiError::Limit("http rate".to_owned()));
        }
        Ok(HttpPermit {
            limiter: self.clone(),
        })
    }

    /// Counts one request against [`HTTP_PER_MINUTE`].
    pub(crate) fn count(&self) -> Result<(), ApiError> {
        let mut window = self.window.lock().unwrap_or_else(PoisonError::into_inner);
        if window.start.elapsed() >= HTTP_WINDOW {
            window.start = Instant::now();
            window.used = 0;
        }
        if window.used >= HTTP_PER_MINUTE {
            return Err(ApiError::Limit("http rate".to_owned()));
        }
        window.used += 1;
        Ok(())
    }
}

/// One in-flight slot; dropping it frees it.
pub(crate) struct HttpPermit {
    limiter: Arc<HttpLimiter>,
}

impl Drop for HttpPermit {
    fn drop(&mut self) {
        self.limiter.concurrent.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_requests_at_a_time() {
        let limiter = Arc::new(HttpLimiter::new());
        let permits: Vec<HttpPermit> = (0..HTTP_CONCURRENT)
            .map(|_| limiter.permit().unwrap())
            .collect();
        let err = match limiter.permit() {
            Ok(_) => panic!("a fifth slot"),
            Err(e) => e,
        };
        assert_eq!(err, ApiError::Limit("http rate".into()));
        drop(permits);
        assert!(limiter.permit().is_ok());
    }

    #[test]
    fn a_hundred_and_twenty_a_minute() {
        let limiter = HttpLimiter::new();
        for _ in 0..HTTP_PER_MINUTE {
            limiter.count().unwrap();
        }
        assert_eq!(
            limiter.count().unwrap_err(),
            ApiError::Limit("http rate".into())
        );
    }
}
