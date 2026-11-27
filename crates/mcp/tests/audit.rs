use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use mcp::{
    audit::{
        Auditor, ClientInfo, sanitize_client_name, target_conversation, target_message, target_user,
    },
    limits::ClientId,
    policy::SettingsSource,
};
use settings::{McpSettings, WorkspaceKey};
use store::{
    AppStore, AuditEntry, AuditOutcome, AuditQuery, AuditRow, MemoryAppStore, StoreError,
    WriteDecision,
};

struct Src(Mutex<bool>);

impl SettingsSource for Src {
    fn mcp(&self) -> Arc<McpSettings> {
        Arc::new(McpSettings::default())
    }
    fn include_content(&self) -> bool {
        *self.0.lock().unwrap()
    }
}

fn client() -> ClientInfo {
    ClientInfo {
        id: ClientId(1),
        name: Arc::from("tester"),
        peer_pid: Some(42),
    }
}

fn ws() -> WorkspaceKey {
    WorkspaceKey::new("fake", "W1")
}

fn setup(content: bool) -> (Arc<MemoryAppStore>, Auditor) {
    let store = Arc::new(MemoryAppStore::default());
    let a = Auditor::new(store.clone(), Arc::new(Src(Mutex::new(content))), || 1_000);
    (store, a)
}

async fn rows(store: &MemoryAppStore) -> Vec<AuditRow> {
    store
        .audit_page(AuditQuery {
            limit: 100,
            ..AuditQuery::default()
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn every_outcome_is_audited() {
    let (store, a) = setup(false);
    let outcomes = [
        AuditOutcome::Ok,
        AuditOutcome::Denied,
        AuditOutcome::NotAvailable,
        AuditOutcome::RateLimited,
        AuditOutcome::Declined,
        AuditOutcome::TimedOut,
        AuditOutcome::Failed,
    ];
    for o in outcomes {
        let mut s = a.start(&client(), "read_channel");
        s.workspace(ws());
        s.target(target_conversation("C1"));
        s.decision(WriteDecision::Auto);
        s.finish(o, Some("why"));
    }
    a.flush().await;
    let got = rows(&store).await;
    assert_eq!(got.len(), outcomes.len());
    let mut seen: Vec<i64> = got.iter().map(|r| r.outcome.code()).collect();
    seen.sort();
    assert_eq!(seen, vec![0, 1, 2, 3, 4, 5, 6]);
    let r = &got[0];
    assert_eq!(&*r.client, "tester");
    assert_eq!(r.peer_pid, Some(42));
    assert_eq!(&*r.tool, "read_channel");
    assert_eq!(r.workspace, Some(ws()));
    assert_eq!(r.target.as_deref(), Some("C1"));
    assert_eq!(r.detail.as_deref(), Some("why"));
    assert_eq!(r.decision, Some(WriteDecision::Auto));
    assert_eq!(r.at_ms, 1_000);
    assert_eq!(a.dropped(), 0);
}

#[tokio::test]
async fn no_content_without_include_content() {
    let (store, a) = setup(false);
    let mut s = a.start(&client(), "send_message");
    s.content(|| panic!("content closure must not run"));
    s.finish(AuditOutcome::Ok, None);
    a.flush().await;
    assert_eq!(rows(&store).await[0].content, None);
}

#[tokio::test]
async fn content_recorded_when_configured() {
    let (store, a) = setup(true);
    let mut s = a.start(&client(), "send_message");
    s.content(|| "hello".to_string());
    s.finish(AuditOutcome::Ok, None);
    a.flush().await;
    assert_eq!(rows(&store).await[0].content.as_deref(), Some("hello"));
}

#[tokio::test]
async fn dropped_span_is_recorded_as_failed() {
    let (store, a) = setup(false);
    drop(a.start(&client(), "get_unreads"));
    a.flush().await;
    let got = rows(&store).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].outcome, AuditOutcome::Failed);
    assert_eq!(got[0].detail.as_deref(), Some("dropped"));
}

struct Broken;

impl AppStore for Broken {
    fn audit_append(&self, _e: AuditEntry) -> BoxFuture<'_, Result<i64, StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn audit_page(&self, _q: AuditQuery) -> BoxFuture<'_, Result<Vec<AuditRow>, StoreError>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn audit_prune(&self, _o: i64) -> BoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async { Ok(0) })
    }
    fn audit_forget_workspace(&self, _w: &WorkspaceKey) -> BoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async { Ok(0) })
    }
    fn audit_clear(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Ok(()) })
    }
    fn plugins(&self) -> BoxFuture<'_, Result<Vec<store::PluginRow>, StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn upsert_plugin(&self, _row: store::PluginRow) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn delete_plugin(&self, _id: &str) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn grants(&self, _id: &str) -> BoxFuture<'_, Result<Vec<store::GrantRow>, StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn set_grants(
        &self,
        _id: &str,
        _g: Vec<store::GrantRow>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn write_allow(&self, _id: &str) -> BoxFuture<'_, Result<Vec<store::WriteAllow>, StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn add_write_allow(
        &self,
        _id: &str,
        _a: store::WriteAllow,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn revoke_write_allow(
        &self,
        _id: &str,
        _a: &store::WriteAllow,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn kv_get(
        &self,
        _id: &str,
        _s: &store::KvScope,
        _k: &str,
    ) -> BoxFuture<'_, Result<Option<Vec<u8>>, StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn kv_set(
        &self,
        _id: &str,
        _s: &store::KvScope,
        _k: &str,
        _v: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn kv_delete(
        &self,
        _id: &str,
        _s: &store::KvScope,
        _k: &str,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn kv_keys(
        &self,
        _id: &str,
        _s: &store::KvScope,
        _p: &str,
        _a: Option<&str>,
        _n: usize,
    ) -> BoxFuture<'_, Result<Vec<Arc<str>>, StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn kv_clear(&self, _id: &str, _s: &store::KvScope) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn kv_usage(&self, _id: &str) -> BoxFuture<'_, Result<store::KvUsage, StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn cwasm_hash(
        &self,
        _k: &store::CwasmKey,
    ) -> BoxFuture<'_, Result<Option<[u8; 32]>, StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
    fn set_cwasm_hash(
        &self,
        _k: &store::CwasmKey,
        _h: Option<[u8; 32]>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Err(StoreError::Closed) })
    }
}

#[tokio::test]
async fn store_failure_does_not_fail_the_call() {
    let a = Auditor::new(Arc::new(Broken), Arc::new(Src(Mutex::new(false))), || 1);
    let s = a.start(&client(), "list_channels");
    s.finish(AuditOutcome::Ok, None);
    a.flush().await;
    assert_eq!(a.dropped(), 1);
}

#[test]
fn client_name_is_sanitized() {
    assert_eq!(
        &*sanitize_client_name(Some("\u{1b}[31mevil\nname"), None),
        "[31mevilname"
    );
    assert_eq!(
        &*sanitize_client_name(Some(&"x".repeat(200)), None),
        &*"x".repeat(64)
    );
    assert_eq!(&*sanitize_client_name(None, None), "unknown");
    assert_eq!(&*sanitize_client_name(Some("\u{1b}\n"), None), "unknown");
    assert_eq!(&*sanitize_client_name(Some("zed"), Some("1.2")), "zed 1.2");
}

#[test]
fn target_never_contains_text() {
    // `target()` only takes the opaque value the helpers build from ids.
    assert_eq!(target_conversation("C1").to_string(), "C1");
    assert_eq!(
        target_message("C1", "1700000000.000100").to_string(),
        "C1/1700000000.000100"
    );
    assert_eq!(target_user("U1").to_string(), "U1");
}
