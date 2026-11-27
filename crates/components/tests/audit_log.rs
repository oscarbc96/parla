//! The MCP audit log view: newest first, paging on scroll, filters, content
//! only when recorded, and clearing behind a confirmation.

mod common;

use std::rc::Rc;
use std::sync::Arc;

use components::{AuditLogView, AuditNames, ClearAuditLog};
use gpui::{App, Entity, SharedString, TestAppContext, VisualContext as _, VisualTestContext};
use settings::WorkspaceKey;
use store::{AppStore, AuditEntry, AuditOutcome, AuditQuery, MemoryAppStore, WriteDecision};

struct Names;

impl AuditNames for Names {
    fn workspaces(&self, _: &App) -> Vec<(WorkspaceKey, SharedString)> {
        vec![(WorkspaceKey::new("fake", "w1"), "Acme".into())]
    }
    fn tools(&self) -> Vec<SharedString> {
        vec!["get_unreads".into(), "send_message".into()]
    }
    fn conversation(&self, _: &WorkspaceKey, id: &str, _: &App) -> Option<SharedString> {
        (id == "C1").then(|| "general".into())
    }
}

fn entry(at: i64, tool: &str, ws: &str, content: Option<&str>) -> AuditEntry {
    let write = tool == "send_message";
    AuditEntry {
        at_ms: at,
        client: Arc::from("claude-code"),
        peer_pid: Some(42),
        tool: Arc::from(tool),
        workspace: Some(WorkspaceKey::new("fake", ws)),
        target: Some(Arc::from("C1")),
        outcome: AuditOutcome::Ok,
        detail: None,
        decision: write.then_some(WriteDecision::Confirmed),
        duration_ms: 12,
        content: content.map(Arc::from),
    }
}

fn fill(store: &MemoryAppStore, n: i64) {
    for i in 0..n {
        let tool = if i % 2 == 0 {
            "get_unreads"
        } else {
            "send_message"
        };
        let ws = if i % 3 == 0 { "w1" } else { "w2" };
        futures::executor::block_on(store.audit_append(entry(1_000 + i, tool, ws, None))).unwrap();
    }
}

fn open(
    cx: &mut TestAppContext,
    store: Arc<MemoryAppStore>,
    page: usize,
) -> (Entity<AuditLogView>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let s: Arc<dyn AppStore> = store;
    let (view, vcx) =
        cx.add_window_view(move |_, cx| AuditLogView::with_page(s, Rc::new(Names), page, cx));
    vcx.run_until_parked();
    (view, vcx)
}

fn ids(view: &Entity<AuditLogView>, vcx: &mut VisualTestContext) -> Vec<i64> {
    view.read_with(vcx, |v, _| v.rows().iter().map(|r| r.id).collect())
}

#[gpui::test]
fn rows_newest_first_and_page_on_scroll(cx: &mut TestAppContext) {
    let store = Arc::new(MemoryAppStore::default());
    fill(&store, 120);
    let (view, vcx) = open(cx, store, 50);
    let first = ids(&view, vcx);
    assert_eq!(first.len(), 50);
    assert!(first.windows(2).all(|w| w[0] > w[1]), "newest first");
    // Nothing more loads while the end is far away.
    view.update(vcx, |v, cx| v.on_visible(20, cx));
    vcx.run_until_parked();
    assert_eq!(ids(&view, vcx).len(), 50);
    // Scrolling near the end of what is loaded fetches the next page.
    view.update(vcx, |v, cx| v.on_visible(46, cx));
    vcx.run_until_parked();
    let second = ids(&view, vcx);
    assert_eq!(second.len(), 100);
    assert_eq!(second[..50], first[..]);
    assert!(second.windows(2).all(|w| w[0] > w[1]), "no gap or repeat");
    view.update(vcx, |v, cx| v.on_visible(98, cx));
    vcx.run_until_parked();
    assert_eq!(ids(&view, vcx).len(), 120);
    assert!(view.read_with(vcx, |v, _| v.is_exhausted()));
}

#[gpui::test]
fn filters_apply(cx: &mut TestAppContext) {
    let store = Arc::new(MemoryAppStore::default());
    fill(&store, 12);
    let (view, vcx) = open(cx, store, 50);
    assert_eq!(ids(&view, vcx).len(), 12);
    view.update(vcx, |v, cx| {
        v.set_workspace_filter(Some(WorkspaceKey::new("fake", "w1")), cx)
    });
    vcx.run_until_parked();
    // i % 3 == 0
    assert_eq!(ids(&view, vcx).len(), 4);
    view.update(vcx, |v, cx| {
        v.set_tool_filter(Some(Arc::from("send_message")), cx)
    });
    vcx.run_until_parked();
    // i % 3 == 0 and i odd: 3 and 9
    let rows = view.read_with(vcx, |v, _| v.rows().to_vec());
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| &*r.tool == "send_message"));
    view.update(vcx, |v, cx| {
        v.set_workspace_filter(None, cx);
        v.set_tool_filter(None, cx);
        v.set_writes_only(true, cx);
    });
    vcx.run_until_parked();
    let rows = view.read_with(vcx, |v, _| v.rows().to_vec());
    assert_eq!(rows.len(), 6);
    assert!(rows.iter().all(|r| r.decision.is_some()));
}

#[gpui::test]
fn labels_use_display_names_then_ids(cx: &mut TestAppContext) {
    let store = Arc::new(MemoryAppStore::default());
    futures::executor::block_on(store.audit_append(entry(1, "get_unreads", "w1", None))).unwrap();
    futures::executor::block_on(store.audit_append(entry(2, "get_unreads", "w9", None))).unwrap();
    let (view, vcx) = open(cx, store, 50);
    vcx.update(|_, cx| {
        let v = view.read(cx);
        let (newest, oldest) = (&v.rows()[0], &v.rows()[1]);
        assert_eq!(
            v.workspace_label(newest, cx),
            "fake/w9",
            "signed out: the key"
        );
        assert_eq!(v.workspace_label(oldest, cx), "Acme");
        assert_eq!(v.target_label(oldest, cx), "#general");
    });
}

#[gpui::test]
fn content_hidden_when_absent(cx: &mut TestAppContext) {
    let store = Arc::new(MemoryAppStore::default());
    futures::executor::block_on(store.audit_append(entry(
        1,
        "send_message",
        "w1",
        Some("hello team"),
    )))
    .unwrap();
    futures::executor::block_on(store.audit_append(entry(2, "send_message", "w1", None))).unwrap();
    let (view, vcx) = open(cx, store, 50);
    let (without, with) = view.read_with(vcx, |v, _| (v.rows()[0].id, v.rows()[1].id));
    view.update(vcx, |v, cx| v.select(without, cx));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.selected_content().map(String::from)),
        None
    );
    assert!(vcx.debug_bounds("audit-content").is_none());
    view.update(vcx, |v, cx| v.select(with, cx));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.selected_content().map(String::from))
            .as_deref(),
        Some("hello team")
    );
    assert!(vcx.debug_bounds("audit-content").is_some());
}

#[gpui::test]
fn clear_requires_confirmation(cx: &mut TestAppContext) {
    let store = Arc::new(MemoryAppStore::default());
    fill(&store, 5);
    let (view, vcx) = open(cx, store.clone(), 50);
    vcx.focus(&view);
    vcx.dispatch_action(ClearAuditLog);
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.is_confirming_clear()));
    let left = futures::executor::block_on(store.audit_page(AuditQuery {
        limit: 10,
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(left.len(), 5, "nothing deleted before the yes");
    // Keeping changes nothing.
    view.update(vcx, |v, cx| v.cancel_clear(cx));
    view.update(vcx, |v, cx| v.confirm_clear(cx));
    vcx.run_until_parked();
    assert_eq!(ids(&view, vcx).len(), 5);
    // Ask again and confirm.
    view.update(vcx, |v, cx| v.request_clear(cx));
    view.update(vcx, |v, cx| v.confirm_clear(cx));
    vcx.run_until_parked();
    assert!(ids(&view, vcx).is_empty());
    let left = futures::executor::block_on(store.audit_page(AuditQuery {
        limit: 10,
        ..Default::default()
    }))
    .unwrap();
    assert!(left.is_empty());
}

/// A store whose page loads and clear can be made to fail.
struct Flaky {
    inner: MemoryAppStore,
    fail: std::sync::atomic::AtomicBool,
}

impl Flaky {
    fn failing(&self) -> Result<(), store::StoreError> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            Err(store::StoreError::Closed)
        } else {
            Ok(())
        }
    }
}

impl AppStore for Flaky {
    fn audit_append(
        &self,
        e: AuditEntry,
    ) -> futures::future::BoxFuture<'_, Result<i64, store::StoreError>> {
        self.inner.audit_append(e)
    }
    fn audit_page(
        &self,
        q: AuditQuery,
    ) -> futures::future::BoxFuture<'_, Result<Vec<store::AuditRow>, store::StoreError>> {
        match self.failing() {
            Ok(()) => self.inner.audit_page(q),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }
    fn audit_prune(
        &self,
        t: i64,
    ) -> futures::future::BoxFuture<'_, Result<u64, store::StoreError>> {
        self.inner.audit_prune(t)
    }
    fn audit_forget_workspace(
        &self,
        ws: &WorkspaceKey,
    ) -> futures::future::BoxFuture<'_, Result<u64, store::StoreError>> {
        self.inner.audit_forget_workspace(ws)
    }
    fn audit_clear(&self) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        match self.failing() {
            Ok(()) => self.inner.audit_clear(),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }
    fn plugins(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<Vec<store::PluginRow>, store::StoreError>> {
        self.inner.plugins()
    }
    fn upsert_plugin(
        &self,
        row: store::PluginRow,
    ) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        self.inner.upsert_plugin(row)
    }
    fn delete_plugin(
        &self,
        id: &str,
    ) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        self.inner.delete_plugin(id)
    }
    fn grants(
        &self,
        id: &str,
    ) -> futures::future::BoxFuture<'_, Result<Vec<store::GrantRow>, store::StoreError>> {
        self.inner.grants(id)
    }
    fn set_grants(
        &self,
        id: &str,
        g: Vec<store::GrantRow>,
    ) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        self.inner.set_grants(id, g)
    }
    fn write_allow(
        &self,
        id: &str,
    ) -> futures::future::BoxFuture<'_, Result<Vec<store::WriteAllow>, store::StoreError>> {
        self.inner.write_allow(id)
    }
    fn add_write_allow(
        &self,
        id: &str,
        a: store::WriteAllow,
    ) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        self.inner.add_write_allow(id, a)
    }
    fn revoke_write_allow(
        &self,
        id: &str,
        a: &store::WriteAllow,
    ) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        self.inner.revoke_write_allow(id, a)
    }
    fn kv_get(
        &self,
        id: &str,
        s: &store::KvScope,
        k: &str,
    ) -> futures::future::BoxFuture<'_, Result<Option<Vec<u8>>, store::StoreError>> {
        self.inner.kv_get(id, s, k)
    }
    fn kv_set(
        &self,
        id: &str,
        s: &store::KvScope,
        k: &str,
        v: Vec<u8>,
    ) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        self.inner.kv_set(id, s, k, v)
    }
    fn kv_delete(
        &self,
        id: &str,
        s: &store::KvScope,
        k: &str,
    ) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        self.inner.kv_delete(id, s, k)
    }
    fn kv_keys(
        &self,
        id: &str,
        s: &store::KvScope,
        p: &str,
        a: Option<&str>,
        n: usize,
    ) -> futures::future::BoxFuture<'_, Result<Vec<Arc<str>>, store::StoreError>> {
        self.inner.kv_keys(id, s, p, a, n)
    }
    fn kv_clear(
        &self,
        id: &str,
        s: &store::KvScope,
    ) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        self.inner.kv_clear(id, s)
    }
    fn kv_usage(
        &self,
        id: &str,
    ) -> futures::future::BoxFuture<'_, Result<store::KvUsage, store::StoreError>> {
        self.inner.kv_usage(id)
    }
    fn cwasm_hash(
        &self,
        k: &store::CwasmKey,
    ) -> futures::future::BoxFuture<'_, Result<Option<[u8; 32]>, store::StoreError>> {
        self.inner.cwasm_hash(k)
    }
    fn set_cwasm_hash(
        &self,
        k: &store::CwasmKey,
        h: Option<[u8; 32]>,
    ) -> futures::future::BoxFuture<'_, Result<(), store::StoreError>> {
        self.inner.set_cwasm_hash(k, h)
    }
}

fn flaky(n: i64) -> Arc<Flaky> {
    let f = Flaky {
        inner: MemoryAppStore::default(),
        fail: Default::default(),
    };
    fill(&f.inner, n);
    Arc::new(f)
}

fn open_dyn(
    cx: &mut TestAppContext,
    store: Arc<dyn AppStore>,
    page: usize,
) -> (Entity<AuditLogView>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (view, vcx) =
        cx.add_window_view(move |_, cx| AuditLogView::with_page(store, Rc::new(Names), page, cx));
    vcx.run_until_parked();
    (view, vcx)
}

#[gpui::test]
fn failed_page_shows_error_and_retries(cx: &mut TestAppContext) {
    let store = flaky(120);
    let (view, vcx) = open_dyn(cx, store.clone(), 50);
    assert_eq!(ids(&view, vcx).len(), 50);
    store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    view.update(vcx, |v, cx| v.load_more(cx));
    vcx.run_until_parked();
    assert_eq!(ids(&view, vcx).len(), 50, "rows are kept");
    assert!(view.read_with(vcx, |v, _| !v.is_exhausted()));
    assert_eq!(
        view.read_with(vcx, |v, _| v.error()),
        Some(components::AuditError::Load)
    );
    assert!(vcx.debug_bounds("audit-error").is_some());
    store.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    view.update(vcx, |v, cx| v.retry(cx));
    vcx.run_until_parked();
    assert_eq!(ids(&view, vcx).len(), 100);
    assert_eq!(view.read_with(vcx, |v, _| v.error()), None);
}

#[gpui::test]
fn failed_first_page_is_not_empty(cx: &mut TestAppContext) {
    let store = flaky(5);
    store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let (view, vcx) = open_dyn(cx, store, 50);
    assert!(view.read_with(vcx, |v, _| !v.is_exhausted()));
    assert!(view.read_with(vcx, |v, _| v.error().is_some()));
}

#[gpui::test]
fn failed_clear_keeps_rows(cx: &mut TestAppContext) {
    let store = flaky(5);
    let (view, vcx) = open_dyn(cx, store.clone(), 50);
    store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    view.update(vcx, |v, cx| {
        v.request_clear(cx);
        v.confirm_clear(cx)
    });
    vcx.run_until_parked();
    assert_eq!(ids(&view, vcx).len(), 5);
    assert_eq!(
        view.read_with(vcx, |v, _| v.error()),
        Some(components::AuditError::Clear)
    );
}

#[gpui::test]
fn filter_change_does_not_cancel_a_running_clear(cx: &mut TestAppContext) {
    let store = flaky(5);
    let (view, vcx) = open_dyn(cx, store.clone(), 50);
    view.update(vcx, |v, cx| {
        v.request_clear(cx);
        v.confirm_clear(cx);
        v.set_writes_only(true, cx);
    });
    vcx.run_until_parked();
    let left = futures::executor::block_on(store.audit_page(AuditQuery {
        limit: 10,
        ..Default::default()
    }))
    .unwrap();
    assert!(left.is_empty(), "the clear finished");
}
