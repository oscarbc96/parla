//! `app.db`: cross-workspace data: the MCP audit log and plugin state.

use std::sync::Arc;

use futures::future::BoxFuture;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use settings::WorkspaceKey;
use store::{
    AppStore, AuditEntry, AuditOutcome, AuditQuery, AuditRow, CwasmKey, GrantRow, KvScope, KvUsage,
    PluginRow, StoreError, WriteAllow, WriteDecision, kv_check,
};

use crate::err::other as db;
use crate::key::DbId;
use crate::thread::SqliteStore;

impl SqliteStore {
    /// The [`AppStore`] view of `app.db`. Calls fail with `StoreError::Closed`
    /// until [`open_app`](SqliteStore::open_app) succeeded.
    pub fn app(&self) -> Arc<dyn AppStore> {
        Arc::new(AppDb(self.clone()))
    }
}

struct AppDb(SqliteStore);

fn insert(c: &Connection, e: &AuditEntry) -> Result<i64, StoreError> {
    c.execute(
        "INSERT INTO mcp_audit (at_ms, client, peer_pid, tool, workspace, target, outcome, \
         detail, decision, duration_ms, content) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            e.at_ms,
            &*e.client,
            e.peer_pid,
            &*e.tool,
            e.workspace.as_ref().map(ToString::to_string),
            e.target.as_deref(),
            e.outcome.code(),
            e.detail.as_deref(),
            e.decision.map(WriteDecision::code),
            e.duration_ms,
            e.content.as_deref(),
        ],
    )
    .map_err(db)?;
    Ok(c.last_insert_rowid())
}

fn page(c: &Connection, q: &AuditQuery) -> Result<Vec<AuditRow>, StoreError> {
    let mut sql = String::from(
        "SELECT id, at_ms, client, peer_pid, tool, workspace, target, outcome, detail, \
         decision, duration_ms, content FROM mcp_audit WHERE 1=1",
    );
    let mut args: Vec<Value> = Vec::new();
    if let Some(b) = q.before_id {
        args.push(b.into());
        sql.push_str(&format!(" AND id < ?{}", args.len()));
    }
    if let Some(w) = &q.workspace {
        args.push(w.to_string().into());
        sql.push_str(&format!(" AND workspace = ?{}", args.len()));
    }
    if let Some(t) = &q.tool {
        args.push(t.to_string().into());
        sql.push_str(&format!(" AND tool = ?{}", args.len()));
    }
    if q.writes_only {
        sql.push_str(" AND decision IS NOT NULL");
    }
    args.push(i64::try_from(q.limit).unwrap_or(i64::MAX).into());
    sql.push_str(&format!(" ORDER BY id DESC LIMIT ?{}", args.len()));
    let mut st = c.prepare_cached(&sql).map_err(db)?;
    let rows = st
        .query_map(params_from_iter(args), |r| {
            let text = |i: usize| r.get::<_, Option<String>>(i).map(|o| o.map(Arc::from));
            Ok(AuditRow {
                id: r.get(0)?,
                entry: AuditEntry {
                    at_ms: r.get(1)?,
                    client: Arc::from(r.get::<_, String>(2)?),
                    peer_pid: r.get(3)?,
                    tool: Arc::from(r.get::<_, String>(4)?),
                    workspace: r
                        .get::<_, Option<String>>(5)?
                        .and_then(|s| s.parse::<WorkspaceKey>().ok()),
                    target: text(6)?,
                    outcome: AuditOutcome::from_code(r.get(7)?).unwrap_or(AuditOutcome::Failed),
                    detail: text(8)?,
                    decision: r
                        .get::<_, Option<i64>>(9)?
                        .and_then(WriteDecision::from_code),
                    duration_ms: r.get(10)?,
                    content: text(11)?,
                },
            })
        })
        .map_err(db)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db)
}

/// Frees the pages a delete left on the freelist (`auto_vacuum=INCREMENTAL`). A
/// separate job: inside the delete's transaction it leaves pages behind.
fn vacuum(db_: &SqliteStore) -> BoxFuture<'_, Result<(), StoreError>> {
    db_.write_db(&DbId::App, |c, _| {
        let mut st = c.prepare("PRAGMA incremental_vacuum").map_err(db)?;
        let mut rows = st.query([]).map_err(db)?;
        while rows.next().map_err(db)?.is_some() {}
        Ok(())
    })
}

/// Fails when the plugin has no row, as the memory store does (the foreign
/// keys would refuse the write too, with a less useful error).
fn need_plugin(c: &Connection, id: &str) -> Result<(), StoreError> {
    let found: Option<i64> = c
        .prepare_cached("SELECT 1 FROM plugins WHERE id = ?1")
        .and_then(|mut st| st.query_row([id], |r| r.get(0)).optional())
        .map_err(db)?;
    match found {
        Some(_) => Ok(()),
        None => Err(StoreError::Other(format!("no plugin {id}"))),
    }
}

fn usage(c: &Connection, id: &str) -> Result<KvUsage, StoreError> {
    c.prepare_cached(
        "SELECT count(*), coalesce(sum(length(CAST(key AS BLOB)) + length(value)), 0) \
         FROM plugin_kv WHERE plugin_id = ?1",
    )
    .and_then(|mut st| {
        st.query_row([id], |r| {
            Ok(KvUsage {
                keys: r.get::<_, i64>(0)? as u64,
                bytes: r.get::<_, i64>(1)? as u64,
            })
        })
    })
    .map_err(db)
}

fn plugin_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PluginRow> {
    let text = |i: usize| r.get::<_, String>(i).map(Arc::from);
    Ok(PluginRow {
        id: text(0)?,
        version: text(1)?,
        source: text(2)?,
        sha256: text(3)?,
        installed_at_ms: r.get(4)?,
        state: text(5)?,
        previous_version: r.get::<_, Option<String>>(6)?.map(Arc::from),
        files_digest: r.get::<_, Option<String>>(7)?.map(Arc::from),
        authors: r.get::<_, Option<String>>(8)?.map(Arc::from),
    })
}

impl AppStore for AppDb {
    fn audit_append(&self, e: AuditEntry) -> BoxFuture<'_, Result<i64, StoreError>> {
        self.0.write_db(&DbId::App, move |c, _| insert(c, &e))
    }

    fn audit_page(&self, q: AuditQuery) -> BoxFuture<'_, Result<Vec<AuditRow>, StoreError>> {
        self.0.read_db(&DbId::App, move |c, _| page(c, &q))
    }

    fn audit_prune(&self, older_than_ms: i64) -> BoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async move {
            let n = self
                .0
                .write_db(&DbId::App, move |c, _| {
                    c.execute("DELETE FROM mcp_audit WHERE at_ms < ?1", [older_than_ms])
                        .map(|n| n as u64)
                        .map_err(db)
                })
                .await?;
            vacuum(&self.0).await?;
            Ok(n)
        })
    }

    fn audit_forget_workspace(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<u64, StoreError>> {
        let ws = ws.to_string();
        self.0.write_db(&DbId::App, move |c, _| {
            c.execute("DELETE FROM mcp_audit WHERE workspace = ?1", [ws])
                .map(|n| n as u64)
                .map_err(db)
        })
    }

    fn audit_clear(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            self.0
                .write_db(&DbId::App, |c, _| {
                    c.execute("DELETE FROM mcp_audit", [])
                        .map(|_| ())
                        .map_err(db)
                })
                .await?;
            vacuum(&self.0).await
        })
    }
    fn plugins(&self) -> BoxFuture<'_, Result<Vec<PluginRow>, StoreError>> {
        self.0.read_db(&DbId::App, |c, _| {
            let mut st = c
                .prepare_cached(
                    "SELECT id, version, source, sha256, installed_at, state, previous_version, \
                 files_digest, authors \
                     FROM plugins ORDER BY id",
                )
                .map_err(db)?;
            let rows = st.query_map([], plugin_row).map_err(db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db)
        })
    }

    fn upsert_plugin(&self, row: PluginRow) -> BoxFuture<'_, Result<(), StoreError>> {
        // An upsert, never `REPLACE`: that deletes the row and cascades.
        self.0.write_db(&DbId::App, move |c, _| {
            c.prepare_cached(
                "INSERT INTO plugins (id, version, source, sha256, installed_at, state, \
                 previous_version, files_digest, authors) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT (id) DO UPDATE SET \
                 version = excluded.version, source = excluded.source, sha256 = excluded.sha256, \
                 installed_at = excluded.installed_at, state = excluded.state, \
                 previous_version = excluded.previous_version, \
                 files_digest = excluded.files_digest, authors = excluded.authors",
            )
            .and_then(|mut st| {
                st.execute(params![
                    &*row.id,
                    &*row.version,
                    &*row.source,
                    &*row.sha256,
                    row.installed_at_ms,
                    &*row.state,
                    row.previous_version.as_deref(),
                    row.files_digest.as_deref(),
                    row.authors.as_deref(),
                ])
            })
            .map(|_| ())
            .map_err(db)
        })
    }

    fn delete_plugin(&self, id: &str) -> BoxFuture<'_, Result<(), StoreError>> {
        let id = id.to_owned();
        Box::pin(async move {
            // The other plugin tables cascade (foreign keys are on).
            let n = self
                .0
                .write_db(&DbId::App, move |c, _| {
                    c.execute("DELETE FROM plugins WHERE id = ?1", [id])
                        .map_err(db)
                })
                .await?;
            if n > 0 {
                vacuum(&self.0).await?;
            }
            Ok(())
        })
    }

    fn grants(&self, id: &str) -> BoxFuture<'_, Result<Vec<GrantRow>, StoreError>> {
        let id = id.to_owned();
        self.0.read_db(&DbId::App, move |c, _| {
            let mut st = c
                .prepare_cached(
                    "SELECT capability, value, granted_at FROM plugin_grants \
                     WHERE plugin_id = ?1 ORDER BY capability, value",
                )
                .map_err(db)?;
            let rows = st
                .query_map([id], |r| {
                    Ok(GrantRow {
                        capability: Arc::from(r.get::<_, String>(0)?),
                        value: Arc::from(r.get::<_, String>(1)?),
                        granted_at_ms: r.get(2)?,
                    })
                })
                .map_err(db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db)
        })
    }

    fn set_grants(&self, id: &str, grants: Vec<GrantRow>) -> BoxFuture<'_, Result<(), StoreError>> {
        let id = id.to_owned();
        self.0.write_db(&DbId::App, move |c, _| {
            need_plugin(c, &id)?;
            c.execute("DELETE FROM plugin_grants WHERE plugin_id = ?1", [&id])
                .map_err(db)?;
            let mut st = c
                .prepare_cached(
                    "INSERT OR REPLACE INTO plugin_grants (plugin_id, capability, value, granted_at) \
                     VALUES (?1,?2,?3,?4)",
                )
                .map_err(db)?;
            for g in &grants {
                st.execute(params![&id, &*g.capability, &*g.value, g.granted_at_ms])
                    .map_err(db)?;
            }
            Ok(())
        })
    }

    fn write_allow(&self, id: &str) -> BoxFuture<'_, Result<Vec<WriteAllow>, StoreError>> {
        let id = id.to_owned();
        self.0.read_db(&DbId::App, move |c, _| {
            let mut st = c
                .prepare_cached(
                    "SELECT workspace, area, conversation FROM plugin_write_allow \
                     WHERE plugin_id = ?1 ORDER BY workspace, area, conversation",
                )
                .map_err(db)?;
            let rows = st
                .query_map([id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(db)?;
            let mut out = Vec::new();
            for row in rows {
                let (w, area, conv) = row.map_err(db)?;
                // Rows are written from a `WorkspaceKey`, so this only skips a
                // hand-edited or corrupt row; an allowance never reached
                // means the plugin asks again, the safe side.
                let Ok(workspace) = w.parse::<WorkspaceKey>() else {
                    continue;
                };
                out.push(WriteAllow {
                    workspace,
                    area: Arc::from(area),
                    conversation: (!conv.is_empty()).then(|| Arc::from(conv)),
                });
            }
            Ok(out)
        })
    }

    fn add_write_allow(&self, id: &str, a: WriteAllow) -> BoxFuture<'_, Result<(), StoreError>> {
        let id = id.to_owned();
        self.0.write_db(&DbId::App, move |c, _| {
            need_plugin(c, &id)?;
            c.prepare_cached(
                "INSERT OR IGNORE INTO plugin_write_allow (plugin_id, workspace, area, conversation) \
                 VALUES (?1,?2,?3,?4)",
            )
            .and_then(|mut st| {
                st.execute(params![
                    &id,
                    a.workspace.to_string(),
                    &*a.area,
                    a.conversation.as_deref().unwrap_or(""),
                ])
            })
            .map(|_| ())
            .map_err(db)
        })
    }

    fn revoke_write_allow(
        &self,
        id: &str,
        a: &WriteAllow,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let id = id.to_owned();
        let a = a.clone();
        self.0.write_db(&DbId::App, move |c, _| {
            c.execute(
                "DELETE FROM plugin_write_allow WHERE plugin_id = ?1 AND workspace = ?2 \
                 AND area = ?3 AND conversation = ?4",
                params![
                    &id,
                    a.workspace.to_string(),
                    &*a.area,
                    a.conversation.as_deref().unwrap_or(""),
                ],
            )
            .map(|_| ())
            .map_err(db)
        })
    }

    fn kv_get(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<Vec<u8>>, StoreError>> {
        let (id, scope, key) = (id.to_owned(), scope.as_text(), key.to_owned());
        self.0.read_db(&DbId::App, move |c, _| {
            c.prepare_cached(
                "SELECT value FROM plugin_kv WHERE plugin_id = ?1 AND scope = ?2 AND key = ?3",
            )
            .and_then(|mut st| st.query_row([id, scope, key], |r| r.get(0)).optional())
            .map_err(db)
        })
    }

    fn kv_set(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
        value: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let (id, scope, key) = (id.to_owned(), scope.as_text(), key.to_owned());
        // One job on the single writer: the check and the write cannot race.
        self.0.write_db(&DbId::App, move |c, _| {
            need_plugin(c, &id)?;
            let old: Option<i64> = c
                .prepare_cached(
                    "SELECT length(CAST(key AS BLOB)) + length(value) FROM plugin_kv \
                     WHERE plugin_id = ?1 AND scope = ?2 AND key = ?3",
                )
                .and_then(|mut st| st.query_row([&id, &scope, &key], |r| r.get(0)).optional())
                .map_err(db)?;
            kv_check(usage(c, &id)?, old.map(|n| n as u64), &key, &value)?;
            c.prepare_cached(
                "INSERT INTO plugin_kv (plugin_id, scope, key, value) VALUES (?1,?2,?3,?4) \
                 ON CONFLICT (plugin_id, scope, key) DO UPDATE SET value = excluded.value",
            )
            .and_then(|mut st| st.execute(params![&id, &scope, &key, &value]))
            .map(|_| ())
            .map_err(db)
        })
    }

    fn kv_delete(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let (id, scope, key) = (id.to_owned(), scope.as_text(), key.to_owned());
        self.0.write_db(&DbId::App, move |c, _| {
            c.execute(
                "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND scope = ?2 AND key = ?3",
                [id, scope, key],
            )
            .map(|_| ())
            .map_err(db)
        })
    }

    fn kv_keys(
        &self,
        id: &str,
        scope: &KvScope,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Arc<str>>, StoreError>> {
        let (id, scope, prefix) = (id.to_owned(), scope.as_text(), prefix.as_bytes().to_vec());
        let after = after.unwrap_or("").to_owned();
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.0.read_db(&DbId::App, move |c, _| {
            let mut st = c
                .prepare_cached(
                    "SELECT key FROM plugin_kv WHERE plugin_id = ?1 AND scope = ?2 \
                     AND substr(CAST(key AS BLOB), 1, length(?3)) = ?3 AND key > ?4 \
                     ORDER BY key LIMIT ?5",
                )
                .map_err(db)?;
            let rows = st
                // `LIKE` folds ASCII case; a byte compare keeps the prefix
                // exact. Text compares are BINARY, so order is byte order.
                .query_map(params![id, scope, prefix, after, limit], |r| {
                    r.get::<_, String>(0).map(Arc::from)
                })
                .map_err(db)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db)
        })
    }

    fn kv_clear(&self, id: &str, scope: &KvScope) -> BoxFuture<'_, Result<(), StoreError>> {
        let (id, scope) = (id.to_owned(), scope.as_text());
        Box::pin(async move {
            let n = self
                .0
                .write_db(&DbId::App, move |c, _| {
                    c.execute(
                        "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND scope = ?2",
                        [id, scope],
                    )
                    .map_err(db)
                })
                .await?;
            if n > 0 {
                vacuum(&self.0).await?;
            }
            Ok(())
        })
    }

    fn kv_usage(&self, id: &str) -> BoxFuture<'_, Result<KvUsage, StoreError>> {
        let id = id.to_owned();
        self.0.read_db(&DbId::App, move |c, _| usage(c, &id))
    }

    fn cwasm_hash(&self, key: &CwasmKey) -> BoxFuture<'_, Result<Option<[u8; 32]>, StoreError>> {
        let k = key.clone();
        self.0.read_db(&DbId::App, move |c, _| {
            let h: Option<Vec<u8>> = c
                .prepare_cached(
                    "SELECT sha256 FROM plugin_cwasm WHERE plugin_id = ?1 AND version = ?2 \
                     AND engine = ?3",
                )
                .and_then(|mut st| {
                    st.query_row([&*k.plugin_id, &*k.version, &*k.engine], |r| r.get(0))
                        .optional()
                })
                .map_err(db)?;
            // A malformed hash reads as none: the cache then recompiles.
            Ok(h.and_then(|v| <[u8; 32]>::try_from(v).ok()))
        })
    }

    fn set_cwasm_hash(
        &self,
        key: &CwasmKey,
        sha256: Option<[u8; 32]>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let k = key.clone();
        self.0.write_db(&DbId::App, move |c, _| {
            let ids = params![&*k.plugin_id, &*k.version, &*k.engine];
            match sha256 {
                Some(h) => {
                    need_plugin(c, &k.plugin_id)?;
                    c.execute(
                        "INSERT INTO plugin_cwasm (plugin_id, version, engine, sha256) \
                         VALUES (?1,?2,?3,?4) ON CONFLICT (plugin_id, version, engine) \
                         DO UPDATE SET sha256 = excluded.sha256",
                        params![&*k.plugin_id, &*k.version, &*k.engine, &h[..]],
                    )
                }
                None => c.execute(
                    "DELETE FROM plugin_cwasm WHERE plugin_id = ?1 AND version = ?2 \
                     AND engine = ?3",
                    ids,
                ),
            }
            .map(|_| ())
            .map_err(db)
        })
    }
}
