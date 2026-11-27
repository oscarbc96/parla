//! Shared `AppStore` cases, run on every backend.

use std::sync::Arc;

use settings::WorkspaceKey;

use crate::{AppStore, AuditEntry, AuditOutcome, AuditQuery, WriteDecision};

fn ws(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

fn entry(at_ms: i64, tool: &str, w: Option<&str>) -> AuditEntry {
    AuditEntry {
        at_ms,
        client: Arc::from("client"),
        peer_pid: Some(42),
        tool: Arc::from(tool),
        workspace: w.map(ws),
        target: Some(Arc::from("C1")),
        outcome: AuditOutcome::Ok,
        detail: None,
        decision: None,
        duration_ms: 7,
        content: None,
    }
}

fn page(limit: usize) -> AuditQuery {
    AuditQuery {
        limit,
        ..Default::default()
    }
}

pub async fn append_then_page_newest_first(s: &dyn AppStore) {
    let mut full = entry(10, "read_messages", Some("a"));
    full.outcome = AuditOutcome::Denied;
    full.detail = Some(Arc::from("deny_list"));
    full.decision = Some(WriteDecision::Declined);
    full.content = Some(Arc::from("body"));
    let a = s.audit_append(full.clone()).await.unwrap();
    let b = s
        .audit_append(entry(20, "list_conversations", None))
        .await
        .unwrap();
    assert!(b > a);
    let rows = s.audit_page(page(10)).await.unwrap();
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), [b, a]);
    assert_eq!(rows[1].entry, full);
    assert_eq!(rows[0].workspace, None);
    assert_eq!(s.audit_page(page(1)).await.unwrap().len(), 1);
}

pub async fn page_filters_workspace_tool_and_writes(s: &dyn AppStore) {
    s.audit_append(entry(1, "read_messages", Some("a")))
        .await
        .unwrap();
    s.audit_append(entry(2, "read_messages", Some("b")))
        .await
        .unwrap();
    let mut w = entry(3, "send_message", Some("a"));
    w.decision = Some(WriteDecision::Confirmed);
    s.audit_append(w).await.unwrap();

    let q = AuditQuery {
        workspace: Some(ws("a")),
        ..page(10)
    };
    assert_eq!(s.audit_page(q).await.unwrap().len(), 2);
    let q = AuditQuery {
        tool: Some(Arc::from("read_messages")),
        ..page(10)
    };
    assert_eq!(s.audit_page(q).await.unwrap().len(), 2);
    let q = AuditQuery {
        writes_only: true,
        ..page(10)
    };
    let rows = s.audit_page(q).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(&*rows[0].tool, "send_message");
    let q = AuditQuery {
        workspace: Some(ws("b")),
        writes_only: true,
        ..page(10)
    };
    assert!(s.audit_page(q).await.unwrap().is_empty());
}

pub async fn page_continues_before_id(s: &dyn AppStore) {
    for i in 0..5 {
        s.audit_append(entry(i, "t", None)).await.unwrap();
    }
    let first = s.audit_page(page(2)).await.unwrap();
    let second = s
        .audit_page(AuditQuery {
            before_id: Some(first[1].id),
            ..page(2)
        })
        .await
        .unwrap();
    let third = s
        .audit_page(AuditQuery {
            before_id: Some(second[1].id),
            ..page(2)
        })
        .await
        .unwrap();
    let ats: Vec<i64> = first
        .iter()
        .chain(&second)
        .chain(&third)
        .map(|r| r.at_ms)
        .collect();
    assert_eq!(ats, [4, 3, 2, 1, 0]);
}

pub async fn prune_drops_only_older(s: &dyn AppStore) {
    for at in [10, 20, 30] {
        s.audit_append(entry(at, "t", None)).await.unwrap();
    }
    assert_eq!(s.audit_prune(20).await.unwrap(), 1);
    let ats: Vec<i64> = s
        .audit_page(page(10))
        .await
        .unwrap()
        .iter()
        .map(|r| r.at_ms)
        .collect();
    assert_eq!(ats, [30, 20]);
    assert_eq!(s.audit_prune(20).await.unwrap(), 0);
}

pub async fn forget_workspace_drops_only_its_rows(s: &dyn AppStore) {
    s.audit_append(entry(1, "t", Some("a"))).await.unwrap();
    s.audit_append(entry(2, "t", Some("b"))).await.unwrap();
    s.audit_append(entry(3, "t", None)).await.unwrap();
    assert_eq!(s.audit_forget_workspace(&ws("a")).await.unwrap(), 1);
    let rows = s.audit_page(page(10)).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.workspace != Some(ws("a"))));
    assert_eq!(s.audit_forget_workspace(&ws("a")).await.unwrap(), 0);
}

pub async fn clear_empties(s: &dyn AppStore) {
    s.audit_append(entry(1, "t", Some("a"))).await.unwrap();
    s.audit_append(entry(2, "t", None)).await.unwrap();
    s.audit_clear().await.unwrap();
    assert!(s.audit_page(page(10)).await.unwrap().is_empty());
}

// Plugin state. A plugin's rows need its `plugins` row.

use crate::{
    CwasmKey, GrantRow, KV_BYTES_MAX, KV_KEY_MAX, KV_KEYS_MAX, KV_VALUE_MAX, KvScope, KvUsage,
    PluginRow, StoreError, WriteAllow,
};

fn plugin(id: &str, version: &str) -> PluginRow {
    PluginRow {
        id: Arc::from(id),
        version: Arc::from(version),
        source: Arc::from("registry"),
        sha256: Arc::from("ab".repeat(32).as_str()),
        installed_at_ms: 1_700_000_000_000,
        state: Arc::from("installed"),
        previous_version: None,
        files_digest: None,
        authors: None,
    }
}

fn grant(capability: &str, value: &str) -> GrantRow {
    GrantRow {
        capability: Arc::from(capability),
        value: Arc::from(value),
        granted_at_ms: 5,
    }
}

fn allow(w: &str, area: &str, conversation: Option<&str>) -> WriteAllow {
    WriteAllow {
        workspace: ws(w),
        area: Arc::from(area),
        conversation: conversation.map(Arc::from),
    }
}

fn cwasm(id: &str) -> CwasmKey {
    CwasmKey {
        plugin_id: Arc::from(id),
        version: Arc::from("1.0.0"),
        engine: Arc::from("e1"),
    }
}

fn is_limit<T: std::fmt::Debug>(r: Result<T, StoreError>) -> bool {
    matches!(r, Err(StoreError::Limit(_)))
}

pub async fn plugin_row_round_trips(s: &dyn AppStore) {
    assert!(s.plugins().await.unwrap().is_empty());
    let mut b = plugin("b-plugin", "2.0.0");
    b.previous_version = Some(Arc::from("1.9.0"));
    b.source = Arc::from("dev");
    s.upsert_plugin(b.clone()).await.unwrap();
    s.upsert_plugin(plugin("a-plugin", "1.0.0")).await.unwrap();
    assert_eq!(
        s.plugins().await.unwrap(),
        [plugin("a-plugin", "1.0.0"), b.clone()],
        "sorted by id"
    );
    // Upsert updates in place and keeps the plugin's other rows.
    s.set_grants("b-plugin", vec![grant("storage", "")])
        .await
        .unwrap();
    s.kv_set("b-plugin", &KvScope::Global, "k", b"v".to_vec())
        .await
        .unwrap();
    b.state = Arc::from("quarantined");
    b.version = Arc::from("2.0.1");
    s.upsert_plugin(b.clone()).await.unwrap();
    assert_eq!(s.plugins().await.unwrap()[1], b);
    assert_eq!(s.grants("b-plugin").await.unwrap().len(), 1);
    assert_eq!(
        s.kv_get("b-plugin", &KvScope::Global, "k").await.unwrap(),
        Some(b"v".to_vec())
    );
}

pub async fn writes_need_a_plugin_row(s: &dyn AppStore) {
    assert!(
        s.set_grants("nope", vec![grant("storage", "")])
            .await
            .is_err()
    );
    assert!(
        s.kv_set("nope", &KvScope::Global, "k", vec![1])
            .await
            .is_err()
    );
    assert!(
        s.add_write_allow("nope", allow("a", "messages:write", None))
            .await
            .is_err()
    );
    assert!(
        s.set_cwasm_hash(&cwasm("nope"), Some([1; 32]))
            .await
            .is_err()
    );
    // Reads of an unknown plugin are empty, not errors.
    assert!(s.grants("nope").await.unwrap().is_empty());
    assert!(s.write_allow("nope").await.unwrap().is_empty());
    assert_eq!(s.kv_usage("nope").await.unwrap(), KvUsage::default());
    assert_eq!(s.cwasm_hash(&cwasm("nope")).await.unwrap(), None);
    // Inserts fail; removes are no-ops.
    s.delete_plugin("nope").await.unwrap();
    s.set_cwasm_hash(&cwasm("nope"), None).await.unwrap();
    s.revoke_write_allow("nope", &allow("a", "messages:write", None))
        .await
        .unwrap();
    s.kv_delete("nope", &KvScope::Global, "k").await.unwrap();
    s.kv_clear("nope", &KvScope::Global).await.unwrap();
}

pub async fn grants_replace_whole_set(s: &dyn AppStore) {
    s.upsert_plugin(plugin("p", "1.0.0")).await.unwrap();
    // Read back sorted by capability, then value; a repeated pair keeps the
    // last one given.
    let mut late = grant("network", "api.example.org");
    late.granted_at_ms = 9;
    s.set_grants(
        "p",
        vec![
            grant("storage", ""),
            grant("network", "api.example.org"),
            grant("network", "Api.example.org"),
            late.clone(),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        s.grants("p").await.unwrap(),
        [
            grant("network", "Api.example.org"),
            late,
            grant("storage", "")
        ]
    );
    s.set_grants("p", vec![grant("network", "cdn.example.org")])
        .await
        .unwrap();
    assert_eq!(
        s.grants("p").await.unwrap(),
        [grant("network", "cdn.example.org")]
    );
    s.set_grants("p", vec![]).await.unwrap();
    assert!(s.grants("p").await.unwrap().is_empty());
}

pub async fn write_allow_add_and_revoke(s: &dyn AppStore) {
    s.upsert_plugin(plugin("p", "1.0.0")).await.unwrap();
    let in_conv = allow("a", "messages:write", Some("C1"));
    let per_plugin = allow("a", "reminders:write", None);
    let whole_area = allow("a", "messages:write", None);
    let other_ws = allow("B", "messages:write", Some("C1"));
    s.add_write_allow("p", per_plugin.clone()).await.unwrap();
    s.add_write_allow("p", in_conv.clone()).await.unwrap();
    s.add_write_allow("p", in_conv.clone()).await.unwrap(); // idempotent
    s.add_write_allow("p", other_ws.clone()).await.unwrap();
    s.add_write_allow("p", whole_area.clone()).await.unwrap();
    // Sorted by workspace key text, area, then conversation (a whole area
    // first); each allowance once.
    assert_eq!(
        s.write_allow("p").await.unwrap(),
        [
            other_ws.clone(),
            whole_area.clone(),
            in_conv.clone(),
            per_plugin.clone()
        ]
    );
    s.revoke_write_allow("p", &whole_area).await.unwrap();
    s.revoke_write_allow("p", &other_ws).await.unwrap();
    s.revoke_write_allow("p", &in_conv).await.unwrap();
    assert_eq!(
        s.write_allow("p").await.unwrap(),
        std::slice::from_ref(&per_plugin)
    );
    s.revoke_write_allow("p", &per_plugin).await.unwrap();
    assert!(s.write_allow("p").await.unwrap().is_empty());
}

pub async fn kv_scopes_keys_and_clear(s: &dyn AppStore) {
    s.upsert_plugin(plugin("p", "1.0.0")).await.unwrap();
    s.upsert_plugin(plugin("q", "1.0.0")).await.unwrap();
    let w = KvScope::Workspace(ws("a"));
    let g = KvScope::Global;
    for k in ["b/2", "a/1", "b/1", "c"] {
        s.kv_set("p", &g, k, k.as_bytes().to_vec()).await.unwrap();
    }
    s.kv_set("p", &w, "a/1", b"ws".to_vec()).await.unwrap();
    s.kv_set("q", &g, "a/1", b"other".to_vec()).await.unwrap();
    // Scopes and plugins are separate.
    assert_eq!(
        s.kv_get("p", &g, "a/1").await.unwrap(),
        Some(b"a/1".to_vec())
    );
    assert_eq!(
        s.kv_get("p", &w, "a/1").await.unwrap(),
        Some(b"ws".to_vec())
    );
    assert_eq!(
        s.kv_get("q", &g, "a/1").await.unwrap(),
        Some(b"other".to_vec())
    );
    assert_eq!(s.kv_get("p", &w, "c").await.unwrap(), None);
    // Keys: sorted, prefix filtered, paged after a cursor.
    let keys = |v: Vec<Arc<str>>| v.iter().map(|k| k.to_string()).collect::<Vec<_>>();
    assert_eq!(
        keys(s.kv_keys("p", &g, "", None, 10).await.unwrap()),
        ["a/1", "b/1", "b/2", "c"]
    );
    assert_eq!(
        keys(s.kv_keys("p", &g, "b/", None, 10).await.unwrap()),
        ["b/1", "b/2"]
    );
    assert_eq!(
        keys(s.kv_keys("p", &g, "", None, 2).await.unwrap()),
        ["a/1", "b/1"]
    );
    assert_eq!(
        keys(s.kv_keys("p", &g, "", Some("b/1"), 2).await.unwrap()),
        ["b/2", "c"]
    );
    assert_eq!(
        keys(s.kv_keys("p", &g, "b/", Some("b/2"), 2).await.unwrap()),
        Vec::<String>::new()
    );
    // A prefix with LIKE metacharacters matches literally.
    s.kv_set("p", &g, "%_x", vec![]).await.unwrap();
    assert_eq!(
        keys(s.kv_keys("p", &g, "%_", None, 10).await.unwrap()),
        ["%_x"]
    );
    assert_eq!(
        keys(s.kv_keys("p", &g, "_", None, 10).await.unwrap()),
        Vec::<String>::new()
    );
    // Usage counts key and value bytes across every scope.
    let u = s.kv_usage("p").await.unwrap();
    assert_eq!(u.keys, 6);
    assert_eq!(u.bytes, (3 + 3) * 3 + 2 + (3 + 2) + 3);
    s.kv_delete("p", &g, "c").await.unwrap();
    s.kv_delete("p", &g, "missing").await.unwrap();
    assert_eq!(s.kv_get("p", &g, "c").await.unwrap(), None);
    s.kv_clear("p", &g).await.unwrap();
    assert!(s.kv_keys("p", &g, "", None, 10).await.unwrap().is_empty());
    assert_eq!(
        s.kv_get("p", &w, "a/1").await.unwrap(),
        Some(b"ws".to_vec())
    );
    assert_eq!(
        s.kv_get("q", &g, "a/1").await.unwrap(),
        Some(b"other".to_vec())
    );
    assert_eq!(
        s.kv_usage("p").await.unwrap(),
        KvUsage { keys: 1, bytes: 5 }
    );
}

pub async fn kv_quota_5mb_2000_keys(s: &dyn AppStore) {
    s.upsert_plugin(plugin("p", "1.0.0")).await.unwrap();
    s.upsert_plugin(plugin("q", "1.0.0")).await.unwrap();
    let g = KvScope::Global;
    // Bytes: 19 full values fit, the 20th (with the key bytes) does not.
    let big = vec![7u8; KV_VALUE_MAX];
    for i in 0..19 {
        s.kv_set("p", &g, &format!("k{i:02}"), big.clone())
            .await
            .unwrap();
    }
    const { assert!(KV_BYTES_MAX < 20 * (KV_VALUE_MAX + 3)) };
    assert!(is_limit(s.kv_set("p", &g, "k19", big.clone()).await));
    assert_eq!(s.kv_get("p", &g, "k19").await.unwrap(), None);
    // Replacing a value counts the new size, not old plus new.
    s.kv_set("p", &g, "k00", big.clone()).await.unwrap();
    // The quota is per plugin, across scopes.
    assert!(is_limit(
        s.kv_set("p", &KvScope::Workspace(ws("a")), "k19", big.clone())
            .await
    ));
    s.kv_set("q", &g, "k19", big.clone()).await.unwrap();
    s.kv_clear("p", &g).await.unwrap();
    // Keys: 2,000 fit, the 2,001st does not; overwriting one still does.
    for i in 0..KV_KEYS_MAX {
        s.kv_set("p", &g, &format!("n{i}"), vec![]).await.unwrap();
    }
    assert!(is_limit(s.kv_set("p", &g, "extra", vec![]).await));
    s.kv_set("p", &g, "n0", vec![1]).await.unwrap();
    assert_eq!(s.kv_usage("p").await.unwrap().keys, KV_KEYS_MAX as u64);
}

pub async fn kv_value_max_256kb(s: &dyn AppStore) {
    s.upsert_plugin(plugin("p", "1.0.0")).await.unwrap();
    let g = KvScope::Global;
    s.kv_set("p", &g, "v", vec![0; KV_VALUE_MAX]).await.unwrap();
    assert!(is_limit(
        s.kv_set("p", &g, "v", vec![0; KV_VALUE_MAX + 1]).await
    ));
    assert_eq!(
        s.kv_get("p", &g, "v").await.unwrap().unwrap().len(),
        KV_VALUE_MAX
    );
    let key = "k".repeat(KV_KEY_MAX);
    s.kv_set("p", &g, &key, vec![]).await.unwrap();
    assert!(is_limit(
        s.kv_set("p", &g, &format!("{key}k"), vec![]).await
    ));
}

pub async fn cwasm_hash_round_trips(s: &dyn AppStore) {
    s.upsert_plugin(plugin("p", "1.0.0")).await.unwrap();
    let k = cwasm("p");
    assert_eq!(s.cwasm_hash(&k).await.unwrap(), None);
    s.set_cwasm_hash(&k, Some([3; 32])).await.unwrap();
    s.set_cwasm_hash(&k, Some([4; 32])).await.unwrap();
    assert_eq!(s.cwasm_hash(&k).await.unwrap(), Some([4; 32]));
    let other = CwasmKey {
        engine: Arc::from("e2"),
        ..k.clone()
    };
    assert_eq!(s.cwasm_hash(&other).await.unwrap(), None);
    s.set_cwasm_hash(&k, None).await.unwrap();
    assert_eq!(s.cwasm_hash(&k).await.unwrap(), None);
}

pub async fn delete_plugin_cascades(s: &dyn AppStore) {
    for id in ["p", "q"] {
        s.upsert_plugin(plugin(id, "1.0.0")).await.unwrap();
        s.set_grants(id, vec![grant("storage", "")]).await.unwrap();
        s.add_write_allow(id, allow("a", "messages:write", Some("C1")))
            .await
            .unwrap();
        s.kv_set(id, &KvScope::Global, "k", vec![1]).await.unwrap();
        s.kv_set(id, &KvScope::Workspace(ws("a")), "k", vec![1])
            .await
            .unwrap();
        s.set_cwasm_hash(&cwasm(id), Some([9; 32])).await.unwrap();
    }
    s.delete_plugin("p").await.unwrap();
    assert_eq!(s.plugins().await.unwrap(), [plugin("q", "1.0.0")]);
    assert!(s.grants("p").await.unwrap().is_empty());
    assert!(s.write_allow("p").await.unwrap().is_empty());
    assert_eq!(s.kv_usage("p").await.unwrap(), KvUsage::default());
    assert_eq!(s.cwasm_hash(&cwasm("p")).await.unwrap(), None);
    // Reinstalling starts clean.
    s.upsert_plugin(plugin("p", "1.0.0")).await.unwrap();
    assert_eq!(s.kv_get("p", &KvScope::Global, "k").await.unwrap(), None);
    // The other plugin is untouched.
    assert_eq!(s.grants("q").await.unwrap().len(), 1);
    assert_eq!(s.write_allow("q").await.unwrap().len(), 1);
    assert_eq!(s.kv_usage("q").await.unwrap().keys, 2);
    assert_eq!(s.cwasm_hash(&cwasm("q")).await.unwrap(), Some([9; 32]));
}

pub async fn kv_keys_prefix_is_case_sensitive(s: &dyn AppStore) {
    s.upsert_plugin(plugin("p", "1.0.0")).await.unwrap();
    let g = KvScope::Global;
    for k in [
        "Abc",
        "abc",
        "aBc",
        "ab",
        "b",
        "\u{e9}t\u{e9}",
        "\u{c9}t\u{e9}",
    ] {
        s.kv_set("p", &g, k, vec![]).await.unwrap();
    }
    let keys = |v: Vec<Arc<str>>| v.iter().map(|k| k.to_string()).collect::<Vec<_>>();
    // Byte order, prefix matched byte for byte.
    assert_eq!(
        keys(s.kv_keys("p", &g, "", None, 10).await.unwrap()),
        [
            "Abc",
            "aBc",
            "ab",
            "abc",
            "b",
            "\u{c9}t\u{e9}",
            "\u{e9}t\u{e9}"
        ]
    );
    assert_eq!(
        keys(s.kv_keys("p", &g, "ab", None, 10).await.unwrap()),
        ["ab", "abc"]
    );
    assert_eq!(
        keys(s.kv_keys("p", &g, "A", None, 10).await.unwrap()),
        ["Abc"]
    );
    assert_eq!(
        keys(s.kv_keys("p", &g, "\u{e9}", None, 10).await.unwrap()),
        ["\u{e9}t\u{e9}"]
    );
    assert_eq!(
        keys(s.kv_keys("p", &g, "a", Some("aBc"), 10).await.unwrap()),
        ["ab", "abc"]
    );
}

pub async fn kv_empty_key_refused(s: &dyn AppStore) {
    s.upsert_plugin(plugin("p", "1.0.0")).await.unwrap();
    let g = KvScope::Global;
    assert!(is_limit(s.kv_set("p", &g, "", vec![1]).await));
    assert_eq!(s.kv_get("p", &g, "").await.unwrap(), None);
    assert_eq!(s.kv_usage("p").await.unwrap(), KvUsage::default());
}

/// The installed files' digest and the authors live on the plugin row, so
/// replacing or clearing the grant never drops them.
pub async fn files_digest_survives_set_grants(s: &dyn AppStore) {
    let mut p = plugin("a-plugin", "1.0.0");
    p.files_digest = Some(Arc::from("ef".repeat(32).as_str()));
    p.authors = Some(Arc::from("Ada\nGrace"));
    s.upsert_plugin(p.clone()).await.unwrap();
    s.set_grants("a-plugin", vec![grant("storage", "")])
        .await
        .unwrap();
    s.set_grants("a-plugin", Vec::new()).await.unwrap();
    assert_eq!(s.plugins().await.unwrap(), [p.clone()]);
    p.files_digest = None;
    s.upsert_plugin(p.clone()).await.unwrap();
    assert_eq!(s.plugins().await.unwrap(), [p]);
}
