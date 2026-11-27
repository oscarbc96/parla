//! MCP status and the audit log window.

mod support;

use std::time::Duration;

use components::AuditLogView;
use gpui::TestAppContext;
use parla::mcp_status::McpStatus;
use rmcp::ServiceExt as _;
use support::mcp_env::{settings_json, start};
use support::settle;

fn text(cx: &mut TestAppContext) -> Option<String> {
    cx.update(|cx| McpStatus::text(cx))
}

#[gpui::test]
async fn status_counts_connected_clients(cx: &mut TestAppContext) {
    // Never enabled: nothing to show.
    let m = start(cx, &settings_json(false, ""));
    assert_eq!(text(cx), None);
    m.set_settings(cx, &settings_json(true, ""));
    let on_idle = text(cx).expect("shown once enabled");
    // Two clients connect.
    let mut clients = Vec::new();
    for _ in 0..2 {
        let stream = m.connect().await.unwrap();
        let c =
            m.rt.spawn(async move { ().serve(stream).await.unwrap() })
                .await
                .unwrap();
        clients.push(c);
    }
    settle(cx);
    assert_eq!(cx.update(|cx| McpStatus::clients(cx)), 2);
    assert!(text(cx).unwrap().contains('2'), "{:?}", text(cx));
    // One leaves.
    let gone = clients.pop().unwrap();
    m.rt.spawn(async move { gone.cancel().await.ok() })
        .await
        .unwrap();
    settle(cx);
    cx.executor().advance_clock(Duration::from_millis(100));
    settle(cx);
    assert_eq!(cx.update(|cx| McpStatus::clients(cx)), 1);
    assert!(text(cx).unwrap().contains('1'), "{:?}", text(cx));
    // Turning it off keeps the strip, now saying off.
    m.set_settings(cx, &settings_json(false, ""));
    settle(cx);
    let off = text(cx).expect("still shown");
    assert_ne!(off, on_idle);
    assert_eq!(cx.update(|cx| McpStatus::clients(cx)), 0);
}

#[gpui::test]
async fn open_audit_log_reuses_one_window(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(true, ""));
    let _ = &m;
    let first = cx
        .update(parla::mcp_status::open_audit_log)
        .expect("window");
    settle(cx);
    let second = cx
        .update(parla::mcp_status::open_audit_log)
        .expect("window");
    assert_eq!(first.window_id(), second.window_id());
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    assert!(first.downcast::<AuditLogView>().is_some());
}

#[gpui::test]
async fn the_idle_lock_closes_the_audit_log(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(true, ""));
    settle(cx);
    let _ = &m;
    cx.update(parla::mcp_status::open_audit_log)
        .expect("window");
    settle(cx);
    assert_eq!(cx.update(|cx| cx.windows().len()), 1);
    cx.update(parla::mcp_status::close_audit_log);
    settle(cx);
    assert_eq!(cx.update(|cx| cx.windows().len()), 0);
}
