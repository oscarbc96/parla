//! `app.db` plugin tables: the shared contract on SQLite,
//! encryption on disk and the secret key index cascade.

use std::path::Path;
use std::sync::Arc;

use store::{AppStore, GrantRow, KvScope, PluginRow, app_contract};
use store_sqlite::{DbId, KeySource, MemoryKeySource, SqliteConfig, SqliteStore};

fn start(dir: &Path) -> (SqliteStore, Arc<MemoryKeySource>) {
    let keys = Arc::new(MemoryKeySource::default());
    let s = SqliteStore::start(
        SqliteConfig {
            dir: dir.to_path_buf(),
            cache_mb: 8,
        },
        keys.clone(),
    );
    (s, keys)
}

struct Fixture {
    _store: SqliteStore,
    app: Arc<dyn AppStore>,
    _dir: tempfile::TempDir,
}

async fn sqlite() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = start(dir.path());
    store.open_app().await.unwrap();
    let app = store.app();
    Fixture {
        _store: store,
        app,
        _dir: dir,
    }
}

macro_rules! case {
    ($($name:ident),* $(,)?) => {$(
        #[tokio::test]
        async fn $name() {
            let f = sqlite().await;
            app_contract::$name(&*f.app).await;
        }
    )*};
}

case!(
    plugin_row_round_trips,
    writes_need_a_plugin_row,
    grants_replace_whole_set,
    write_allow_add_and_revoke,
    kv_scopes_keys_and_clear,
    kv_quota_5mb_2000_keys,
    kv_value_max_256kb,
    cwasm_hash_round_trips,
    delete_plugin_cascades,
    kv_keys_prefix_is_case_sensitive,
    kv_empty_key_refused,
    files_digest_survives_set_grants,
);

fn row(id: &str, version: &str) -> PluginRow {
    PluginRow {
        id: Arc::from(id),
        version: Arc::from(version),
        source: Arc::from("file"),
        sha256: Arc::from("cd".repeat(32).as_str()),
        installed_at_ms: 1,
        state: Arc::from("installed"),
        previous_version: None,
        files_digest: None,
        authors: None,
    }
}

#[tokio::test]
async fn plugin_rows_unreadable_on_disk() {
    let d = tempfile::tempdir().unwrap();
    let (s, _) = start(d.path());
    s.open_app().await.unwrap();
    let app = s.app();
    let id = "Hq3Nv8Tc5Yr1Bm6P";
    let host = "Wx4Kd9Lf2Gs7Jn5Q";
    let value = "Ub8Ce3Ra6Zt1Mv4Y";
    app.upsert_plugin(row(id, "1.0.0")).await.unwrap();
    app.set_grants(
        id,
        vec![GrantRow {
            capability: Arc::from("network"),
            value: Arc::from(host),
            granted_at_ms: 1,
        }],
    )
    .await
    .unwrap();
    app.kv_set(id, &KvScope::Global, "k", value.as_bytes().to_vec())
        .await
        .unwrap();
    s.close_app().await.unwrap(); // last connection closed: WAL checkpointed
    let mut raw = std::fs::read(s.app_path()).unwrap();
    if let Ok(w) = std::fs::read(d.path().join("app.db-wal")) {
        raw.extend(w);
    }
    for marker in [id, host, value] {
        assert!(
            !raw.windows(marker.len()).any(|w| w == marker.as_bytes()),
            "{marker} readable on disk"
        );
    }
}

fn raw_conn(s: &SqliteStore, hex: &str) -> rusqlite::Connection {
    let c = rusqlite::Connection::open(s.app_path()).unwrap();
    c.execute_batch(&format!(
        "PRAGMA key = \"x'{hex}'\"; PRAGMA foreign_keys = ON;"
    ))
    .unwrap();
    c
}

#[tokio::test]
async fn delete_plugin_drops_secret_key_index() {
    let d = tempfile::tempdir().unwrap();
    let (s, keys) = start(d.path());
    s.open_app().await.unwrap();
    s.app().upsert_plugin(row("p", "1.0.0")).await.unwrap();
    s.app().upsert_plugin(row("q", "1.0.0")).await.unwrap();
    s.close_app().await.unwrap();
    let hex = keys.get_or_create(&DbId::App).await.unwrap().to_hex();
    {
        let c = raw_conn(&s, hex.as_str());
        c.execute_batch(
            "INSERT INTO plugin_secret_keys (plugin_id, key) VALUES ('p', 'token'), ('q', 'token')",
        )
        .unwrap();
    }
    s.open_app().await.unwrap();
    s.app().delete_plugin("p").await.unwrap();
    s.close_app().await.unwrap();
    let c = raw_conn(&s, hex.as_str());
    let left: Vec<String> = c
        .prepare("SELECT plugin_id FROM plugin_secret_keys ORDER BY plugin_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(left, ["q"]);
}
