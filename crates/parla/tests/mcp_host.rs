//! The MCP lifecycle inside the app: settings turn the server on
//! and off without a restart, over the real IPC socket and the real store.

mod support;

use std::time::Duration;

use gpui::TestAppContext;
use mcp::ipc::IpcError;
use parla::instance::{Instance, handoff};
use parla::toast::ConfigBanner;
use serde_json::json;
use store::{AuditEntry, AuditOutcome, AuditQuery};
use support::mcp_env::{settings_json, start, start_with};
use support::{env, settle};

fn secs(cx: &mut TestAppContext, s: u64) {
    cx.executor().advance_clock(Duration::from_secs(s));
    settle(cx);
}

#[gpui::test]
async fn enabling_mcp_binds_without_restart(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(false, ""));
    let off = m.connect().await.err();
    assert!(matches!(off, Some(IpcError::McpOff)), "{off:?}");
    m.set_settings(cx, &settings_json(true, ""));
    let r = m.call("list_workspaces", json!({})).await;
    assert!(r.to_string().contains("fake/w1"), "{r}");
    assert!(
        !r.to_string().contains("fake/w2"),
        "only enabled workspaces are listed: {r}"
    );
}

#[gpui::test]
async fn disabling_mcp_closes_listener_and_connections(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(true, ""));
    // A client that stays connected.
    let stream = m.connect().await.unwrap();
    let client =
        m.rt.spawn(async move {
            use rmcp::ServiceExt as _;
            ().serve(stream).await.unwrap()
        })
        .await
        .unwrap();
    m.set_settings(cx, &settings_json(false, ""));
    secs(cx, 1);
    let closed =
        m.rt.spawn(async move {
            tokio::time::timeout(Duration::from_secs(1), client.waiting())
                .await
                .is_ok()
        })
        .await
        .unwrap();
    assert!(closed, "the client sees the connection end within 1 s");
    let again = m.connect().await.err();
    assert!(matches!(again, Some(IpcError::McpOff)), "{again:?}");
    // Single instance still works.
    let p = m.socket.clone();
    assert!(std::thread::spawn(move || handoff(&p)).join().unwrap());
}

#[gpui::test]
async fn removed_transport_key_is_dropped_from_the_file_without_a_banner(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(true, r#""transport": "http","#));
    cx.update(|cx| {
        let b = cx.global::<ConfigBanner>();
        assert!(b.notices.is_empty(), "{:?}", b.notices);
    });
    let text = std::fs::read_to_string(m.env.config.join("settings.json")).unwrap();
    assert!(!text.contains("transport"), "{text}");
    let r = m.call("list_workspaces", json!({})).await;
    assert!(r.to_string().contains("fake/w1"), "stdio is used: {r}");
}

/// The audit drain runs on the runtime; give it real time.
async fn audit_rows(m: &support::mcp_env::Mcp, cx: &mut TestAppContext) -> Vec<store::AuditRow> {
    let app = m.env.sqlite.app();
    for _ in 0..40 {
        settle(cx);
        let a = app.clone();
        let rows =
            m.rt.spawn(async move {
                a.audit_page(AuditQuery {
                    limit: 50,
                    ..Default::default()
                })
                .await
                .unwrap_or_default()
            })
            .await
            .unwrap();
        if !rows.is_empty() {
            return rows;
        }
    }
    Vec::new()
}

#[gpui::test]
async fn audit_rows_land_in_app_db(cx: &mut TestAppContext) {
    let m = start(cx, &settings_json(true, ""));
    m.call("list_workspaces", json!({})).await;
    let rows = audit_rows(&m, cx).await;
    assert!(
        rows.iter().any(|r| &*r.entry.tool == "list_workspaces"),
        "{rows:?}"
    );
}

#[gpui::test]
async fn audit_pruned_once_at_startup(cx: &mut TestAppContext) {
    let e = env(cx, &settings_json(false, r#""audit_retention_days": 30,"#));
    // An old row and a fresh one exist before MCP is first enabled.
    let sq = e.sqlite.clone();
    let entry = |at_ms| AuditEntry {
        at_ms,
        client: "c".into(),
        peer_pid: None,
        tool: "old".into(),
        workspace: None,
        target: None,
        outcome: AuditOutcome::Ok,
        detail: None,
        decision: None,
        duration_ms: 1,
        content: None,
    };
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        sq.open_app().await.unwrap();
        sq.app().audit_append(entry(1)).await.unwrap();
        sq.app().audit_append(entry(now)).await.unwrap();
        sq.close_app().await.unwrap();
    });
    let m = start_with(cx, e);
    m.set_settings(cx, &settings_json(true, r#""audit_retention_days": 30,"#));
    m.call("list_workspaces", json!({})).await;
    let rows = audit_rows(&m, cx).await;
    assert!(
        rows.iter().all(|r| r.entry.at_ms > 1),
        "old row pruned: {rows:?}"
    );
    assert!(rows.iter().any(|r| r.entry.at_ms == now), "fresh row kept");
}

#[gpui::test]
async fn unreadable_app_db_keeps_mcp_running_with_banner(cx: &mut TestAppContext) {
    let e = env(cx, &settings_json(false, ""));
    std::fs::create_dir_all(e.data.join("store")).unwrap();
    std::fs::write(
        e.data.join("store").join("app.db"),
        b"not a database at all",
    )
    .unwrap();
    let m = start_with(cx, e);
    m.set_settings(cx, &settings_json(true, ""));
    let r = m.call("list_workspaces", json!({})).await;
    assert!(r.to_string().contains("fake/w1"), "MCP keeps running: {r}");
    settle(cx);
    cx.update(|cx| {
        let b = cx.global::<ConfigBanner>();
        assert!(
            b.notices.iter().any(|(k, _)| *k == "mcp-audit"),
            "{:?}",
            b.notices
        );
    });
    // The action replaces the file; the banner goes and rows are written again.
    cx.update(parla::mcp_host::McpHost::rebuild_audit);
    for _ in 0..6 {
        settle(cx);
    }
    cx.update(|cx| {
        assert!(cx.global::<ConfigBanner>().notices.is_empty());
    });
}

#[gpui::test]
async fn off_by_default_binds_nothing_but_show(cx: &mut TestAppContext) {
    let m = start(cx, "{}");
    cx.update(|cx| assert!(!Instance::is_headless(cx)));
    let off = m.connect().await.err();
    assert!(matches!(off, Some(IpcError::McpOff)), "{off:?}");
    // No audit database was opened and no limiter built.
    assert!(!m.env.data.join("store").join("app.db").exists());
}
