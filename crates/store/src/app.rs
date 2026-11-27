//! Cross-workspace storage (`app.db`): the MCP audit log and plugin state.
//! Neutral like the rest of `store`: a workspace is the
//! text `WorkspaceKey`, no provider is named.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use settings::WorkspaceKey;

use crate::StoreError;

/// How a tool call ended. The integer codes are stored on disk and never
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    Ok = 0,
    Denied = 1,
    NotAvailable = 2,
    RateLimited = 3,
    Declined = 4,
    TimedOut = 5,
    Failed = 6,
}

impl AuditOutcome {
    pub fn code(self) -> i64 {
        self as i64
    }

    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => Self::Ok,
            1 => Self::Denied,
            2 => Self::NotAvailable,
            3 => Self::RateLimited,
            4 => Self::Declined,
            5 => Self::TimedOut,
            6 => Self::Failed,
            _ => return None,
        })
    }
}

/// How a write was settled. The integer codes are stored on disk and never
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteDecision {
    Confirmed = 0,
    Auto = 1,
    Declined = 2,
    TimedOut = 3,
    /// Sent without a prompt: the user chose "Always for this session" for
    /// this tool and destination earlier in the session.
    SessionAllowed = 4,
    /// The user answered "Always for this session": this write was confirmed
    /// and later writes of its tool and destination go without a prompt.
    ConfirmedForSession = 5,
}

impl WriteDecision {
    pub fn code(self) -> i64 {
        self as i64
    }

    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => Self::Confirmed,
            1 => Self::Auto,
            2 => Self::Declined,
            3 => Self::TimedOut,
            4 => Self::SessionAllowed,
            5 => Self::ConfirmedForSession,
            _ => return None,
        })
    }
}

/// One audited tool call. `detail` is a fixed, content-free reason such as
/// `deny_list`, `dms_excluded`, `writes_off` or `provider_error`; `content`
/// is only filled when the owner turned content logging on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub at_ms: i64,
    pub client: Arc<str>,
    pub peer_pid: Option<u32>,
    pub tool: Arc<str>,
    pub workspace: Option<WorkspaceKey>,
    pub target: Option<Arc<str>>,
    pub outcome: AuditOutcome,
    pub detail: Option<Arc<str>>,
    pub decision: Option<WriteDecision>,
    pub duration_ms: u32,
    pub content: Option<Arc<str>>,
}

/// An [`AuditEntry`] as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub id: i64,
    pub entry: AuditEntry,
}

impl std::ops::Deref for AuditRow {
    type Target = AuditEntry;
    fn deref(&self) -> &AuditEntry {
        &self.entry
    }
}

/// A page of the audit log, newest first. `before_id` continues after the
/// last row of the previous page; `writes_only` keeps rows that carry a
/// write decision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditQuery {
    pub before_id: Option<i64>,
    pub limit: usize,
    pub workspace: Option<WorkspaceKey>,
    pub tool: Option<Arc<str>>,
    pub writes_only: bool,
}

/// One installed plugin. `source` (`registry`, `file`,
/// `dev`) and `state` are the plugin
/// host's names, kept as text so the store needs no plugin types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    pub id: Arc<str>,
    pub version: Arc<str>,
    pub source: Arc<str>,
    /// Hex SHA-256 of the package.
    pub sha256: Arc<str>,
    pub installed_at_ms: i64,
    pub state: Arc<str>,
    pub previous_version: Option<Arc<str>>,
    /// Hex SHA-256 of the installed files, checked before the plugin loads.
    pub files_digest: Option<Arc<str>>,
    /// The manifest's authors, one per line: with `source`, who a kept row's
    /// data belongs to.
    pub authors: Option<Arc<str>>,
}

/// One accepted capability of a plugin's grant. `value` is the capability's
/// argument (a network host, an area, an argv shape) or empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRow {
    pub capability: Arc<str>,
    pub value: Arc<str>,
    pub granted_at_ms: i64,
}

/// A remembered "allow without asking" for a plugin's writes: per
/// conversation when `conversation` is set, else for the whole area in that
/// workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteAllow {
    pub workspace: WorkspaceKey,
    pub area: Arc<str>,
    pub conversation: Option<Arc<str>>,
}

/// Where a plugin's key-value entry lives.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum KvScope {
    Global,
    Workspace(WorkspaceKey),
}

impl KvScope {
    /// The text stored on disk: `global` or the workspace key (which always
    /// holds a `/`, so the two never collide).
    pub fn as_text(&self) -> String {
        match self {
            KvScope::Global => "global".to_owned(),
            KvScope::Workspace(w) => w.to_string(),
        }
    }
}

/// What a plugin's key-value entries use, across every scope. `bytes` counts
/// key and value bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvUsage {
    pub keys: u64,
    pub bytes: u64,
}

/// Longest key, in bytes.
pub const KV_KEY_MAX: usize = 256;
/// Largest value, in bytes.
pub const KV_VALUE_MAX: usize = 256 * 1024;
/// Key and value bytes per plugin.
pub const KV_BYTES_MAX: usize = 5 * 1024 * 1024;
/// Keys per plugin.
pub const KV_KEYS_MAX: usize = 2000;

/// Checks one `kv_set` against the limits. `usage` is the plugin's usage
/// before the write and `old` the size (key plus value) of the entry it
/// replaces. Backends share it so they refuse alike.
pub fn kv_check(
    usage: KvUsage,
    old: Option<u64>,
    key: &str,
    value: &[u8],
) -> Result<(), StoreError> {
    if key.is_empty() {
        return Err(StoreError::Limit("storage key empty"));
    }
    if key.len() > KV_KEY_MAX {
        return Err(StoreError::Limit("storage key too long"));
    }
    if value.len() > KV_VALUE_MAX {
        return Err(StoreError::Limit("storage value too large"));
    }
    let keys = usage.keys + u64::from(old.is_none());
    if keys > KV_KEYS_MAX as u64 {
        return Err(StoreError::Limit("too many storage keys"));
    }
    let bytes = usage.bytes - old.unwrap_or(0) + (key.len() + value.len()) as u64;
    if bytes > KV_BYTES_MAX as u64 {
        return Err(StoreError::Limit("storage full"));
    }
    Ok(())
}

/// Identifies one precompiled `.cwasm`: plugin, version and engine tag.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CwasmKey {
    pub plugin_id: Arc<str>,
    pub version: Arc<str>,
    pub engine: Arc<str>,
}

pub trait AppStore: Send + Sync + 'static {
    /// Appends a row and returns its id.
    fn audit_append(&self, e: AuditEntry) -> BoxFuture<'_, Result<i64, StoreError>>;
    /// Newest first.
    fn audit_page(&self, q: AuditQuery) -> BoxFuture<'_, Result<Vec<AuditRow>, StoreError>>;
    /// Deletes rows older than `older_than_ms`; returns how many.
    fn audit_prune(&self, older_than_ms: i64) -> BoxFuture<'_, Result<u64, StoreError>>;
    /// Deletes every row of one workspace; returns how many.
    fn audit_forget_workspace(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<u64, StoreError>>;
    fn audit_clear(&self) -> BoxFuture<'_, Result<(), StoreError>>;

    // Plugin state. For an id without a `plugins` row inserts fail, removes
    // are no-ops and reads are empty.

    /// Every installed plugin, sorted by id.
    fn plugins(&self) -> BoxFuture<'_, Result<Vec<PluginRow>, StoreError>>;
    /// Inserts or updates in place; the plugin's other rows are kept.
    fn upsert_plugin(&self, row: PluginRow) -> BoxFuture<'_, Result<(), StoreError>>;
    /// Removes the plugin with its grants, write allowances, key-value
    /// entries, secret key index and `.cwasm` hashes.
    fn delete_plugin(&self, id: &str) -> BoxFuture<'_, Result<(), StoreError>>;
    /// Sorted by capability, then value (byte order).
    fn grants(&self, id: &str) -> BoxFuture<'_, Result<Vec<GrantRow>, StoreError>>;
    /// Replaces the whole grant. A capability and value given twice keeps
    /// the last one.
    fn set_grants(&self, id: &str, grants: Vec<GrantRow>) -> BoxFuture<'_, Result<(), StoreError>>;
    /// Sorted by workspace key text, area, then conversation, a whole-area
    /// allowance (`conversation: None`) first; each allowance once.
    fn write_allow(&self, id: &str) -> BoxFuture<'_, Result<Vec<WriteAllow>, StoreError>>;
    /// Idempotent.
    fn add_write_allow(&self, id: &str, a: WriteAllow) -> BoxFuture<'_, Result<(), StoreError>>;
    fn revoke_write_allow(&self, id: &str, a: &WriteAllow)
    -> BoxFuture<'_, Result<(), StoreError>>;
    fn kv_get(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<Vec<u8>>, StoreError>>;
    /// Fails with [`StoreError::Limit`] for an empty key or past [`KV_KEY_MAX`], [`KV_VALUE_MAX`],
    /// [`KV_BYTES_MAX`] or [`KV_KEYS_MAX`]; nothing is written then.
    fn kv_set(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
        value: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), StoreError>>;
    fn kv_delete(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
    ) -> BoxFuture<'_, Result<(), StoreError>>;
    /// Keys starting with `prefix` (byte for byte, so case-sensitive),
    /// sorted in byte order, after `after` when given, at most `limit`.
    fn kv_keys(
        &self,
        id: &str,
        scope: &KvScope,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Arc<str>>, StoreError>>;
    fn kv_clear(&self, id: &str, scope: &KvScope) -> BoxFuture<'_, Result<(), StoreError>>;
    fn kv_usage(&self, id: &str) -> BoxFuture<'_, Result<KvUsage, StoreError>>;
    fn cwasm_hash(&self, key: &CwasmKey) -> BoxFuture<'_, Result<Option<[u8; 32]>, StoreError>>;
    /// `None` removes the hash.
    fn set_cwasm_hash(
        &self,
        key: &CwasmKey,
        sha256: Option<[u8; 32]>,
    ) -> BoxFuture<'_, Result<(), StoreError>>;
}

/// In-memory [`AppStore`] for tests and for running without the encrypted
/// store.
#[derive(Default)]
pub struct MemoryAppStore(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    next: i64,
    rows: Vec<AuditRow>,
    plugins: BTreeMap<Arc<str>, PluginState>,
}

/// One plugin's rows. Bounded: the store refuses entries past the limits.
#[derive(Default)]
struct PluginState {
    row: Option<PluginRow>,
    grants: Vec<GrantRow>,
    allow: Vec<WriteAllow>,
    kv: BTreeMap<(KvScope, Arc<str>), Vec<u8>>,
    cwasm: BTreeMap<(Arc<str>, Arc<str>), [u8; 32]>,
}

fn unknown(id: &str) -> StoreError {
    StoreError::Other(format!("no plugin {id}"))
}

fn ready<T: Send + 'static>(r: Result<T, StoreError>) -> BoxFuture<'static, Result<T, StoreError>> {
    Box::pin(async move { r })
}

impl MemoryAppStore {
    fn with_plugin<T: Send + 'static>(
        &self,
        id: &str,
        f: impl FnOnce(&mut PluginState) -> Result<T, StoreError>,
    ) -> BoxFuture<'static, Result<T, StoreError>> {
        let mut g = self.0.lock().unwrap();
        ready(match g.plugins.get_mut(id) {
            Some(p) => f(p),
            None => Err(unknown(id)),
        })
    }

    fn read_plugin<T: Send + 'static>(
        &self,
        id: &str,
        empty: T,
        f: impl FnOnce(&PluginState) -> T,
    ) -> BoxFuture<'static, Result<T, StoreError>> {
        let g = self.0.lock().unwrap();
        ready(Ok(g.plugins.get(id).map_or(empty, f)))
    }
}

/// The order [`AppStore::write_allow`] returns, the one `app.db` sorts by.
fn allow_order(a: &WriteAllow) -> (String, &str, &str) {
    (
        a.workspace.to_string(),
        &a.area,
        a.conversation.as_deref().unwrap_or(""),
    )
}

fn usage(p: &PluginState) -> KvUsage {
    KvUsage {
        keys: p.kv.len() as u64,
        bytes: p
            .kv
            .iter()
            .map(|((_, k), v)| (k.len() + v.len()) as u64)
            .sum(),
    }
}

impl AppStore for MemoryAppStore {
    fn audit_append(&self, e: AuditEntry) -> BoxFuture<'_, Result<i64, StoreError>> {
        let mut g = self.0.lock().unwrap();
        g.next += 1;
        let id = g.next;
        g.rows.push(AuditRow { id, entry: e });
        Box::pin(async move { Ok(id) })
    }

    fn audit_page(&self, q: AuditQuery) -> BoxFuture<'_, Result<Vec<AuditRow>, StoreError>> {
        let g = self.0.lock().unwrap();
        let rows: Vec<AuditRow> = g
            .rows
            .iter()
            .rev()
            .filter(|r| q.before_id.is_none_or(|b| r.id < b))
            .filter(|r| {
                q.workspace
                    .as_ref()
                    .is_none_or(|w| r.workspace.as_ref() == Some(w))
            })
            .filter(|r| q.tool.as_ref().is_none_or(|t| r.tool == *t))
            .filter(|r| !q.writes_only || r.decision.is_some())
            .take(q.limit)
            .cloned()
            .collect();
        Box::pin(async move { Ok(rows) })
    }

    fn audit_prune(&self, older_than_ms: i64) -> BoxFuture<'_, Result<u64, StoreError>> {
        let mut g = self.0.lock().unwrap();
        let before = g.rows.len();
        g.rows.retain(|r| r.at_ms >= older_than_ms);
        let n = (before - g.rows.len()) as u64;
        Box::pin(async move { Ok(n) })
    }

    fn audit_forget_workspace(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<u64, StoreError>> {
        let mut g = self.0.lock().unwrap();
        let before = g.rows.len();
        g.rows.retain(|r| r.workspace.as_ref() != Some(ws));
        let n = (before - g.rows.len()) as u64;
        Box::pin(async move { Ok(n) })
    }

    fn audit_clear(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.0.lock().unwrap().rows.clear();
        Box::pin(async { Ok(()) })
    }

    fn plugins(&self) -> BoxFuture<'_, Result<Vec<PluginRow>, StoreError>> {
        let g = self.0.lock().unwrap();
        ready(Ok(g
            .plugins
            .values()
            .filter_map(|p| p.row.clone())
            .collect()))
    }

    fn upsert_plugin(&self, row: PluginRow) -> BoxFuture<'_, Result<(), StoreError>> {
        let mut g = self.0.lock().unwrap();
        let id = row.id.clone();
        g.plugins.entry(id).or_default().row = Some(row);
        ready(Ok(()))
    }

    fn delete_plugin(&self, id: &str) -> BoxFuture<'_, Result<(), StoreError>> {
        self.0.lock().unwrap().plugins.remove(id);
        ready(Ok(()))
    }

    fn grants(&self, id: &str) -> BoxFuture<'_, Result<Vec<GrantRow>, StoreError>> {
        self.read_plugin(id, Vec::new(), |p| p.grants.clone())
    }

    fn set_grants(&self, id: &str, grants: Vec<GrantRow>) -> BoxFuture<'_, Result<(), StoreError>> {
        self.with_plugin(id, |p| {
            // Keyed by capability and value: sorted, and the last one wins.
            let by: BTreeMap<_, _> = grants
                .into_iter()
                .map(|g| ((g.capability.clone(), g.value.clone()), g))
                .collect();
            p.grants = by.into_values().collect();
            Ok(())
        })
    }

    fn write_allow(&self, id: &str) -> BoxFuture<'_, Result<Vec<WriteAllow>, StoreError>> {
        self.read_plugin(id, Vec::new(), |p| p.allow.clone())
    }

    fn add_write_allow(&self, id: &str, a: WriteAllow) -> BoxFuture<'_, Result<(), StoreError>> {
        self.with_plugin(id, |p| {
            if let Err(i) = p
                .allow
                .binary_search_by(|x| allow_order(x).cmp(&allow_order(&a)))
            {
                p.allow.insert(i, a);
            }
            Ok(())
        })
    }

    fn revoke_write_allow(
        &self,
        id: &str,
        a: &WriteAllow,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let mut g = self.0.lock().unwrap();
        if let Some(p) = g.plugins.get_mut(id) {
            p.allow.retain(|x| x != a);
        }
        ready(Ok(()))
    }

    fn kv_get(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
    ) -> BoxFuture<'_, Result<Option<Vec<u8>>, StoreError>> {
        let k = (scope.clone(), Arc::from(key));
        self.read_plugin(id, None, |p| p.kv.get(&k).cloned())
    }

    fn kv_set(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
        value: Vec<u8>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let k = (scope.clone(), Arc::<str>::from(key));
        self.with_plugin(id, |p| {
            let old = p.kv.get(&k).map(|v| (key.len() + v.len()) as u64);
            kv_check(usage(p), old, key, &value)?;
            p.kv.insert(k, value);
            Ok(())
        })
    }

    fn kv_delete(
        &self,
        id: &str,
        scope: &KvScope,
        key: &str,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let mut g = self.0.lock().unwrap();
        if let Some(p) = g.plugins.get_mut(id) {
            p.kv.remove(&(scope.clone(), Arc::from(key)));
        }
        ready(Ok(()))
    }

    fn kv_keys(
        &self,
        id: &str,
        scope: &KvScope,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Arc<str>>, StoreError>> {
        self.read_plugin(id, Vec::new(), |p| {
            p.kv.keys()
                .filter(|(s, k)| s == scope && k.starts_with(prefix))
                .filter(|(_, k)| after.is_none_or(|a| &**k > a))
                .take(limit)
                .map(|(_, k)| k.clone())
                .collect()
        })
    }

    fn kv_clear(&self, id: &str, scope: &KvScope) -> BoxFuture<'_, Result<(), StoreError>> {
        let mut g = self.0.lock().unwrap();
        if let Some(p) = g.plugins.get_mut(id) {
            p.kv.retain(|(s, _), _| s != scope);
        }
        ready(Ok(()))
    }

    fn kv_usage(&self, id: &str) -> BoxFuture<'_, Result<KvUsage, StoreError>> {
        self.read_plugin(id, KvUsage::default(), usage)
    }

    fn cwasm_hash(&self, key: &CwasmKey) -> BoxFuture<'_, Result<Option<[u8; 32]>, StoreError>> {
        let k = (key.version.clone(), key.engine.clone());
        self.read_plugin(&key.plugin_id, None, |p| p.cwasm.get(&k).copied())
    }

    fn set_cwasm_hash(
        &self,
        key: &CwasmKey,
        sha256: Option<[u8; 32]>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let k = (key.version.clone(), key.engine.clone());
        match sha256 {
            Some(h) => self.with_plugin(&key.plugin_id, |p| {
                p.cwasm.insert(k, h);
                Ok(())
            }),
            None => {
                let mut g = self.0.lock().unwrap();
                if let Some(p) = g.plugins.get_mut(&*key.plugin_id) {
                    p.cwasm.remove(&k);
                }
                ready(Ok(()))
            }
        }
    }
}
