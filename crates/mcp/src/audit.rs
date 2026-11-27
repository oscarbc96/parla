//! Audit spans (spec 8a Safety, section 12): one row per tool call, no
//! message text unless `log.include_content` is on. Appends go through a
//! bounded channel so a slow or failing store never delays or fails a call.

use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use settings::WorkspaceKey;
use store::{AppStore, AuditEntry, AuditOutcome, WriteDecision};
use tokio::sync::{mpsc, oneshot};

use crate::policy::SettingsSource;

const CHANNEL_CAPACITY: usize = 1_024;
const MAX_NAME: usize = 64;
const WARN_EVERY: Duration = Duration::from_secs(60);

/// The MCP client making a call (shared with the write confirmation).
pub use write_confirm::ClientInfo;

/// Printable ASCII only, at most 64 chars, `unknown` when nothing is left.
pub fn sanitize_client_name(raw: Option<&str>, version: Option<&str>) -> Arc<str> {
    let clean = |s: &str| -> String {
        s.chars()
            .filter(|c| (' '..='~').contains(c))
            .collect::<String>()
            .trim()
            .to_string()
    };
    let name = clean(raw.unwrap_or(""));
    if name.is_empty() {
        return Arc::from("unknown");
    }
    let version = clean(version.unwrap_or(""));
    let full = if version.is_empty() {
        name
    } else {
        format!("{name} {version}")
    };
    Arc::from(full.chars().take(MAX_NAME).collect::<String>().trim_end())
}

/// What a call touched, as ids only. Built by the `target_*` helpers so
/// message text cannot end up here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target(String);

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Longest id kept in a target: arguments are client-supplied, ids are short.
const MAX_TARGET_ID: usize = 128;

fn capped(id: &str) -> String {
    id.chars().take(MAX_TARGET_ID).collect()
}

pub fn target_conversation(conversation: &str) -> Target {
    Target(capped(conversation))
}

pub fn target_message(conversation: &str, ts: &str) -> Target {
    Target(format!("{}/{}", capped(conversation), capped(ts)))
}

pub fn target_user(user: &str) -> Target {
    Target(capped(user))
}

enum Msg {
    Entry(Box<AuditEntry>),
    Flush(oneshot::Sender<()>),
}

struct Inner {
    tx: mpsc::Sender<Msg>,
    settings: Arc<dyn SettingsSource>,
    now: fn() -> i64,
    dropped: AtomicU64,
    last_warn: Mutex<Option<Instant>>,
}

impl Inner {
    fn note_dropped(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        let mut last = self.last_warn.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_none_or(|t| t.elapsed() >= WARN_EVERY) {
            *last = Some(Instant::now());
            tracing::warn!("mcp audit row dropped; the audit log is behind or unavailable");
        }
    }
}

/// Records one row per tool call. Needs a Tokio runtime at construction.
#[derive(Clone)]
pub struct Auditor {
    inner: Arc<Inner>,
}

impl Auditor {
    pub fn new(
        store: Arc<dyn AppStore>,
        settings: Arc<dyn SettingsSource>,
        now: fn() -> i64,
    ) -> Auditor {
        let (tx, mut rx) = mpsc::channel::<Msg>(CHANNEL_CAPACITY);
        let inner = Arc::new(Inner {
            tx,
            settings,
            now,
            dropped: AtomicU64::new(0),
            last_warn: Mutex::new(None),
        });
        // The drain task only holds a weak handle so dropping every Auditor
        // closes the channel and ends the task.
        let weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    Msg::Entry(e) => {
                        if store.audit_append(*e).await.is_err()
                            && let Some(i) = weak.upgrade()
                        {
                            i.note_dropped();
                        }
                    }
                    Msg::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        Auditor { inner }
    }

    /// Starts a span; call it before anything else in the tool.
    pub fn start(&self, client: &ClientInfo, tool: &'static str) -> AuditSpan {
        AuditSpan {
            inner: self.inner.clone(),
            started: Instant::now(),
            entry: Some(AuditEntry {
                at_ms: (self.inner.now)(),
                client: client.name.clone(),
                peer_pid: client.peer_pid,
                tool: Arc::from(tool),
                workspace: None,
                target: None,
                outcome: AuditOutcome::Failed,
                detail: None,
                decision: None,
                duration_ms: 0,
                content: None,
            }),
        }
    }

    /// Rows lost to a full channel or a store error.
    #[cfg(feature = "test-support")]
    pub fn dropped(&self) -> u64 {
        self.inner.dropped.load(Ordering::Relaxed)
    }

    /// Resolves once every row sent so far has been handed to the store.
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        if self.inner.tx.send(Msg::Flush(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }
}

/// One call in flight. Dropping it without `finish` records `Failed` with
/// detail `dropped`, so a cancelled call is still audited.
pub struct AuditSpan {
    inner: Arc<Inner>,
    started: Instant,
    entry: Option<AuditEntry>,
}

impl AuditSpan {
    fn entry(&mut self) -> &mut AuditEntry {
        self.entry.as_mut().expect("span not finished")
    }

    pub fn workspace(&mut self, ws: WorkspaceKey) {
        self.entry().workspace = Some(ws);
    }

    pub fn target(&mut self, t: Target) {
        self.entry().target = Some(Arc::from(t.0));
    }

    /// The closure only runs when content logging is on.
    pub fn content(&mut self, f: impl FnOnce() -> String) {
        if self.inner.settings.include_content() {
            self.entry().content = Some(Arc::from(f()));
        }
    }

    /// A fixed, content-free reason recorded when the call finishes without
    /// its own (for example `idempotent_replay`).
    pub fn note(&mut self, detail: &'static str) {
        self.entry().detail = Some(Arc::from(detail));
    }

    pub fn decision(&mut self, d: WriteDecision) {
        self.entry().decision = Some(d);
    }

    pub fn finish(mut self, outcome: AuditOutcome, detail: Option<&'static str>) {
        self.send(outcome, detail);
    }

    fn send(&mut self, outcome: AuditOutcome, detail: Option<&'static str>) {
        let Some(mut e) = self.entry.take() else {
            return;
        };
        e.outcome = outcome;
        if let Some(d) = detail {
            e.detail = Some(Arc::from(d));
        }
        e.duration_ms = u32::try_from(self.started.elapsed().as_millis()).unwrap_or(u32::MAX);
        if self.inner.tx.try_send(Msg::Entry(Box::new(e))).is_err() {
            self.inner.note_dropped();
        }
    }
}

impl Drop for AuditSpan {
    fn drop(&mut self) {
        self.send(AuditOutcome::Failed, Some("dropped"));
    }
}
