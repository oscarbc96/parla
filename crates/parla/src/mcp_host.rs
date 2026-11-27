//! The MCP server's lifecycle inside the app.
//!
//! `mcp.enabled` false costs nothing: no tool server, no limiter, no auditor,
//! no `app.db` open and no task beyond the single-instance listener of
//! [`crate::instance`]. Turning it on needs no restart: [`McpHost::sync`]
//! (called at startup and after every settings reload) opens the gate of the
//! IPC listener, opens `app.db` once and serves each accepted stream on the
//! Tokio runtime. Turning it off cancels every MCP connection within a
//! second; the `show` listener stays.

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use futures::future::Shared;
use gpui::{App, Global};
use mcp::{
    audit::Auditor,
    ipc::IpcStream,
    limits::Limits,
    policy::SettingsSource,
    server::{Confirmer, ParlaServer},
};
use settings::{McpSettings, SettingsStore};
use store::{AppStore, MemoryAppStore};
use tokio_util::sync::CancellationToken;

use crate::instance::Instance;
use crate::mcp_backend::{AppBackend, Hop};
use crate::root_view::SqliteHandle;
use crate::toast::ConfigBanner;

const DAY_MS: i64 = 86_400_000;

/// The settings snapshot the policy reads; replaced on every reload.
#[derive(Default)]
pub struct LiveSettings(Mutex<(Arc<McpSettings>, bool)>);

impl LiveSettings {
    fn set(&self, mcp: McpSettings, include_content: bool) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = (Arc::new(mcp), include_content);
    }
}

impl SettingsSource for LiveSettings {
    fn mcp(&self) -> Arc<McpSettings> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).0.clone()
    }
    fn include_content(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).1
    }
}

type Ready = Shared<futures::future::BoxFuture<'static, ()>>;

/// What exists only while MCP has been enabled at least once.
struct Running {
    store: Arc<dyn AppStore>,
    auditor: Auditor,
    ready: Ready,
}

/// The MCP lifecycle global.
pub struct McpHost {
    rt: tokio::runtime::Handle,
    hop: Hop,
    backend: Arc<AppBackend>,
    settings: Arc<LiveSettings>,
    confirmer: Arc<dyn Confirmer>,
    enabled: bool,
    cancel: CancellationToken,
    limits: Option<Arc<Limits>>,
    /// Shared by every connection, so a reconnect keeps its idempotency keys.
    idempotency: Arc<mcp::idempotency::Idempotency>,
    running: Option<Running>,
}

impl Global for McpHost {}

impl McpHost {
    /// Creates the host (no work yet), registers the audit-log action and
    /// applies the current settings.
    pub fn install(cx: &mut App, rt: tokio::runtime::Handle) {
        let hop = Hop::start(cx);
        let confirmer = Arc::new(crate::write_confirm_window::WindowConfirmer::new(
            hop.clone(),
        ));
        cx.set_global(McpHost {
            rt,
            backend: Arc::new(AppBackend::new(hop.clone())),
            hop,
            settings: Arc::new(LiveSettings::default()),
            confirmer,
            enabled: false,
            cancel: CancellationToken::new(),
            limits: None,
            idempotency: Arc::default(),
            running: None,
        });
        cx.on_action(|_: &crate::actions::RebuildAuditLog, cx| Self::rebuild_audit(cx));
        cx.on_action(|_: &crate::actions::OpenAuditLog, cx| {
            crate::mcp_status::open_audit_log(cx);
        });
        // Reached only when no audit log window has the focus: open it first.
        cx.on_action(|_: &components::ClearAuditLog, cx| {
            if let Some(w) = crate::mcp_status::open_audit_log(cx)
                && let Some(w) = w.downcast::<components::AuditLogView>()
            {
                let _ = w.update(cx, |v, _, cx| v.request_clear(cx));
            }
        });
        Self::sync(cx);
    }

    /// The store the audit log window reads: the real `app.db` once MCP has
    /// been enabled, an empty log before that.
    pub fn audit_store(cx: &App) -> Arc<dyn AppStore> {
        cx.try_global::<McpHost>()
            .and_then(|h| h.running.as_ref().map(|r| r.store.clone()))
            .unwrap_or_else(|| Arc::new(MemoryAppStore::default()))
    }

    /// Applies `mcp.enabled` from the current settings.
    pub fn sync(cx: &mut App) {
        if cx.try_global::<McpHost>().is_none() {
            return;
        }
        // A key a removed setting left in the file goes away once; if the
        // file cannot be written the key stays and is ignored, no banner.
        if !cx.global::<SettingsStore>().warnings().is_empty() {
            let removed = cx.global_mut::<SettingsStore>().remove_stale_keys();
            if !removed.is_empty() {
                crate::log::info("removed unused keys from settings.json");
            }
        }
        let (mcp, content) = {
            let s = cx.global::<SettingsStore>().current();
            (s.mcp.clone(), s.log.include_content)
        };
        let host = cx.global::<McpHost>();
        host.settings.set(mcp.clone(), content);
        let (was, on) = (host.enabled, mcp.enabled);
        crate::mcp_status::McpStatus::set_enabled(cx, on);
        Instance::set_mcp_gate(cx, on);
        if on == was {
            return;
        }
        if on {
            let host = cx.global_mut::<McpHost>();
            host.enabled = true;
            host.cancel = CancellationToken::new();
            if host.running.is_none() {
                Self::start(cx, mcp.audit_retention_days);
            }
        } else {
            let host = cx.global_mut::<McpHost>();
            host.enabled = false;
            host.cancel.cancel();
        }
    }

    /// First enable: the limiter, `app.db` (opened and pruned once) and the
    /// auditor over it.
    fn start(cx: &mut App, retention_days: u32) {
        let sqlite = cx.try_global::<SqliteHandle>().map(|h| h.0.clone());
        let (rt, hop, settings) = {
            let h = cx.global::<McpHost>();
            (h.rt.clone(), h.hop.clone(), h.settings.clone())
        };
        let (store, ready): (Arc<dyn AppStore>, Ready) = match sqlite {
            Some(db) => {
                let app = db.app();
                let (a, hop) = (app.clone(), hop.clone());
                let ready = async move {
                    match db.open_app().await {
                        Ok(()) => {
                            let cutoff = now_ms() - i64::from(retention_days) * DAY_MS;
                            let _ = a.audit_prune(cutoff).await;
                            hop.run(|cx| set_audit_notice(cx, false));
                        }
                        Err(e) => {
                            crate::log::warn(&format!("audit log unavailable: {e}"));
                            hop.run(|cx| set_audit_notice(cx, true));
                        }
                    }
                }
                .boxed()
                .shared();
                (app, ready)
            }
            None => (
                Arc::new(MemoryAppStore::default()),
                async {}.boxed().shared(),
            ),
        };
        rt.spawn(ready.clone());
        let auditor = {
            let _enter = rt.enter();
            Auditor::new(store.clone(), settings, now_ms)
        };
        let host = cx.global_mut::<McpHost>();
        host.limits = Some(Arc::new(Limits::system()));
        host.running = Some(Running {
            store,
            auditor,
            ready,
        });
    }

    /// An authenticated MCP stream from the IPC listener: serves it until the
    /// client leaves or MCP is turned off.
    pub fn accept(stream: IpcStream, peer_pid: Option<u32>, cx: &mut App) {
        let Some(host) = cx.try_global::<McpHost>() else {
            return;
        };
        let (Some(limits), Some(running), true) =
            (host.limits.clone(), host.running.as_ref(), host.enabled)
        else {
            return;
        };
        let server = ParlaServer::new(
            host.backend.clone(),
            host.settings.clone(),
            limits,
            running.auditor.clone(),
            host.confirmer.clone(),
            peer_pid,
        )
        .with_idempotency(host.idempotency.clone());
        let (ready, cancel, hop) = (running.ready.clone(), host.cancel.clone(), host.hop.clone());
        let rt = host.rt.clone();
        Instance::mcp_connected(cx);
        crate::mcp_status::McpStatus::client_connected(cx);
        rt.spawn(async move {
            ready.await;
            tokio::select! {
                () = cancel.cancelled() => {}
                _ = server.serve(stream) => {}
            }
            hop.run(|cx| {
                Instance::mcp_disconnected(cx);
                crate::mcp_status::McpStatus::client_disconnected(cx);
            });
        });
    }

    /// `mcp::RebuildAuditLog`: replaces an unreadable `app.db` with an empty
    /// one. Only acts while the audit-log notice is shown.
    pub fn rebuild_audit(cx: &mut App) {
        let shown = cx
            .global::<ConfigBanner>()
            .notices
            .iter()
            .any(|(k, _)| *k == "mcp-audit");
        let (Some(db), Some(h)) = (
            cx.try_global::<SqliteHandle>().map(|h| h.0.clone()),
            cx.try_global::<McpHost>(),
        ) else {
            return;
        };
        if !shown {
            return;
        }
        let hop = h.hop.clone();
        h.rt.spawn(async move {
            let ok = db.rebuild_app().await.is_ok();
            hop.run(move |cx| set_audit_notice(cx, !ok));
        });
    }
}

fn set_audit_notice(cx: &mut App, on: bool) {
    let l = i18n::gpui::localizer(cx);
    cx.global_mut::<ConfigBanner>().set_notice(
        "mcp-audit",
        on.then(|| t!(l, "mcp-notice-audit").into_owned()),
    );
}

/// Reports that the IPC endpoint could not be created safely.
pub fn report_bind_error(cx: &mut App, e: &mcp::ipc::IpcError) {
    if matches!(e, mcp::ipc::IpcError::UnsafeDir) {
        let l = i18n::gpui::localizer(cx);
        let dir = settings::paths::run_dir().display().to_string();
        cx.global_mut::<ConfigBanner>().set_notice(
            "mcp-socket",
            Some(t!(l, "mcp-notice-socket-unsafe", path = dir).into_owned()),
        );
    }
}

fn now_ms() -> i64 {
    use domain::Now as _;
    domain::SystemNow.now().as_millisecond()
}
