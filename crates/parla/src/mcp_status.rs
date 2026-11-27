//! MCP status and the audit log window.
//!
//! [`McpStatus`] is a small global the status strip of the main window reads:
//! nothing until MCP has been enabled once, then "MCP: off" or "MCP: on, N
//! clients". It changes only when the setting flips or a client connects or
//! leaves, so it costs nothing while idle. Clicking the strip, the
//! `mcp::OpenAuditLog` action and the command palette open the audit log in
//! its own window (reused when already open), which works without the main
//! window.

use std::rc::Rc;

use components::{AuditLogView, AuditNames};
use domain::ConversationId;
use gpui::{AnyWindowHandle, App, AppContext as _, Bounds, Global, SharedString, size};
use settings::WorkspaceKey;

use crate::mcp_host::McpHost;
use crate::workspaces::WorkspacesHandle;

/// What the status strip shows.
#[derive(Default)]
pub struct McpStatus {
    ever_enabled: bool,
    enabled: bool,
    clients: usize,
}

impl Global for McpStatus {}

impl McpStatus {
    /// The strip text, `None` when MCP has never been enabled.
    pub fn text(cx: &App) -> Option<String> {
        let s = cx.try_global::<McpStatus>()?;
        if !s.ever_enabled {
            return None;
        }
        let l = i18n::gpui::localizer(cx);
        Some(if s.enabled {
            t!(&l, "mcp-status-on", count = s.clients as i64).into_owned()
        } else {
            t!(&l, "mcp-status-off").into_owned()
        })
    }

    /// Connected clients (tests).
    pub fn clients(cx: &App) -> usize {
        cx.try_global::<McpStatus>().map_or(0, |s| s.clients)
    }

    fn update(cx: &mut App, f: impl FnOnce(&mut McpStatus)) {
        f(cx.default_global::<McpStatus>());
        cx.refresh_windows();
    }

    /// `mcp.enabled` changed (or was read at startup).
    pub fn set_enabled(cx: &mut App, on: bool) {
        let s = cx.default_global::<McpStatus>();
        if s.enabled == on && (s.ever_enabled || !on) {
            return;
        }
        Self::update(cx, |s| {
            s.enabled = on;
            s.ever_enabled |= on;
            if !on {
                s.clients = 0;
            }
        });
    }

    pub fn client_connected(cx: &mut App) {
        Self::update(cx, |s| s.clients += 1);
    }

    pub fn client_disconnected(cx: &mut App) {
        Self::update(cx, |s| s.clients = s.clients.saturating_sub(1));
    }
}

/// Workspace and conversation names from the live sessions.
struct AppNames;

impl AuditNames for AppNames {
    fn workspaces(&self, cx: &App) -> Vec<(WorkspaceKey, SharedString)> {
        let Some(h) = cx.try_global::<WorkspacesHandle>() else {
            return Vec::new();
        };
        h.0.read(cx)
            .sessions()
            .map(|s| {
                let st = s.read(cx).state();
                (
                    st.workspace.clone(),
                    SharedString::from(st.name.to_string()),
                )
            })
            .collect()
    }

    fn tools(&self) -> Vec<SharedString> {
        mcp::server::tools()
            .iter()
            .map(|t| SharedString::from(t.name.to_string()))
            .collect()
    }

    fn conversation(&self, ws: &WorkspaceKey, id: &str, cx: &App) -> Option<SharedString> {
        let session = cx
            .try_global::<WorkspacesHandle>()?
            .0
            .read(cx)
            .session(ws)?;
        let st = session.read(cx);
        let c = st.state().conversations.get(&ConversationId(id.into()))?;
        Some(SharedString::from(c.name.to_string()))
    }
}

/// The open audit log window, if any.
#[derive(Default)]
struct AuditWindow(Option<AnyWindowHandle>);

impl Global for AuditWindow {}

/// Opens the audit log window, or brings the open one forward.
pub fn open_audit_log(cx: &mut App) -> Option<AnyWindowHandle> {
    if crate::idle_lock::is_locked(cx) {
        return None;
    }
    if let Some(h) = cx.default_global::<AuditWindow>().0
        && h.update(cx, |_, w, _| w.activate_window()).is_ok()
    {
        crate::tray::bring_to_front(cx);
        return Some(h);
    }
    let store = McpHost::audit_store(cx);
    let bounds = Bounds::centered(
        None,
        size(
            themes::metrics::LAYOUT_WINDOW_AUDIT_WIDTH,
            themes::metrics::LAYOUT_WINDOW_AUDIT_HEIGHT,
        ),
        cx,
    );
    let opened = cx
        .open_window(crate::window_options::audit_log(bounds), |_window, cx| {
            cx.new(|cx| AuditLogView::new(store, Rc::new(AppNames), cx))
        })
        .ok()?;
    let h: AnyWindowHandle = opened.into();
    cx.default_global::<AuditWindow>().0 = Some(h);
    crate::tray::bring_to_front(cx);
    Some(h)
}

/// Closes the audit log window if it is open (the idle lock).
pub fn close_audit_log(cx: &mut App) {
    if let Some(h) = cx.try_global::<AuditWindow>().and_then(|w| w.0) {
        let _ = h.update(cx, |_, window, _| window.remove_window());
        cx.default_global::<AuditWindow>().0 = None;
    }
}
