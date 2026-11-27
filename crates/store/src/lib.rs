//! Persistence trait for the provider-neutral chat model, plus an in-memory
//! backend. A SQLite backend can implement `Store` unchanged.

mod app;
#[cfg(feature = "test-support")]
pub mod app_contract;
#[cfg(feature = "test-support")]
pub mod contract;
mod memory;

use std::collections::HashMap;

use domain::{
    ClientMsgId, ComposedText, Conversation, ConversationId, ConversationPrefs, DraftKey, EmojiRef,
    Gap, LocalSchedule, Member, Message, MessageTs, NotifyExtras, OutboxItem, PersistOp, SearchHit,
    SearchQuery,
};
use futures::future::BoxFuture;
use settings::WorkspaceKey;

pub use app::{
    AppStore, AuditEntry, AuditOutcome, AuditQuery, AuditRow, CwasmKey, GrantRow, KV_BYTES_MAX,
    KV_KEY_MAX, KV_KEYS_MAX, KV_VALUE_MAX, KvScope, KvUsage, MemoryAppStore, PluginRow, WriteAllow,
    WriteDecision, kv_check,
};
pub use memory::{MemoryStore, WriteFault};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("store closed")]
    Closed,
    /// Wrong key or corrupt file (SQLCipher cannot tell them apart).
    #[error("store unreadable")]
    Unreadable,
    #[error("migration failed: {0}")]
    Migration(String),
    /// A size or count limit refused the write; nothing was written.
    #[error("limit reached: {0}")]
    Limit(&'static str),
    #[error("{0}")]
    Other(String),
}

/// Most messages one [`Store::conversation_files`] page reads.
pub const FILES_PAGE_MAX: usize = 100;

/// Builds a [`Store::conversation_files`] page from the messages a backend
/// read: those with files, newest first, at most `limit` of them (already
/// clamped). Backends share it so they page alike.
pub fn files_page(messages: Vec<Message>, limit: usize) -> domain::Page<domain::FileHit> {
    let read = messages.len();
    let mut items = Vec::new();
    let mut used = 0;
    for m in &messages {
        if items.len() >= limit {
            break;
        }
        used += 1;
        items.extend(
            m.files
                .iter()
                .filter(|f| !f.tombstone)
                .map(|f| domain::FileHit {
                    file: f.clone(),
                    conversation: Some(m.id.conversation.clone()),
                    author: m.author.clone(),
                    ts: Some(m.id.ts.clone()),
                }),
        );
    }
    // Unread messages remain, or the read hit the limit: older files may exist.
    let more = used < read || read >= limit;
    let next = used
        .checked_sub(1)
        .and_then(|i| messages.get(i))
        .filter(|_| more)
        .map(|m| m.id.ts.0.clone());
    domain::Page { items, next }
}

/// Recent emoji kept per workspace.
pub const RECENT_EMOJI_CAP: usize = 36;

/// Unix seconds at 00:00:00 UTC on `d`. Local search treats `before:` and
/// `after:` as whole UTC days (both exclusive).
pub fn day_start_secs(d: jiff::civil::Date) -> i64 {
    d.at(0, 0, 0, 0)
        .to_zoned_datetime(jiff::tz::TimeZone::UTC)
        .map_or(0, |z| z.timestamp().as_second())
}

/// A saved draft: where it belongs, its body (with the mentions, emoji and links
/// the user picked) and when it was last edited (Unix seconds).
pub type Draft = (DraftKey, ComposedText, i64);

pub trait Store: Send + Sync + 'static {
    fn apply(&self, ws: &WorkspaceKey, op: PersistOp) -> BoxFuture<'_, Result<(), StoreError>>;
    fn conversations(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<Conversation>, StoreError>>;
    fn members(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<Vec<Member>, StoreError>>;
    /// Newest first: messages strictly older than `before` (or the newest when `None`).
    /// Thread replies that are not `broadcast` are excluded.
    fn messages_before(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Message>, StoreError>>;
    /// Test-support only: the backend contract checks it.
    #[cfg(feature = "test-support")]
    fn latest_ts(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Option<MessageTs>, StoreError>>;

    /// The root then its replies, ascending, at most `limit` messages.
    fn thread(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        root: &MessageTs,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Message>, StoreError>>;
    fn latest_all(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, MessageTs>, StoreError>>;
    fn gaps(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Vec<Gap>, StoreError>>;
    /// Ordered by `created_ms`.
    fn outbox(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<Vec<OutboxItem>, StoreError>>;
    /// Replaces any item with the same id.
    fn outbox_put(
        &self,
        ws: &WorkspaceKey,
        item: OutboxItem,
    ) -> BoxFuture<'_, Result<(), StoreError>>;
    fn outbox_remove(
        &self,
        ws: &WorkspaceKey,
        id: &ClientMsgId,
    ) -> BoxFuture<'_, Result<(), StoreError>>;
    fn drafts(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<Vec<Draft>, StoreError>>;
    /// `None` or a blank body deletes the draft; `edited` is when the user
    /// last changed it (Unix seconds).
    fn draft_put(
        &self,
        ws: &WorkspaceKey,
        key: DraftKey,
        body: Option<ComposedText>,
        edited: i64,
    ) -> BoxFuture<'_, Result<(), StoreError>>;
    /// One message by its timestamp, wherever it sits (channel or thread).
    fn message(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        ts: &MessageTs,
    ) -> BoxFuture<'_, Result<Option<Message>, StoreError>>;
    fn search(
        &self,
        ws: &WorkspaceKey,
        query: &str,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<SearchHit>, StoreError>>;
    /// Locally kept schedules, ordered by `post_at` then id.
    fn local_schedules(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<LocalSchedule>, StoreError>>;
    /// Notification levels kept by Parla itself (never `Default` and unmuted).
    fn local_notify_prefs(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, ConversationPrefs>, StoreError>>;
    /// Parla's own per-conversation extras (never the default).
    fn local_notify_extras(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, NotifyExtras>, StoreError>>;
    /// The conversations the user starred.
    fn local_starred(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<std::collections::HashSet<ConversationId>, StoreError>>;
    /// The sidebar sections the user collapsed (section names, kept by Parla).
    fn sidebar_collapsed(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<std::collections::BTreeSet<String>, StoreError>>;
    /// Collapses (`true`) or expands (`false`) a sidebar section.
    fn sidebar_collapsed_put(
        &self,
        ws: &WorkspaceKey,
        section: String,
        collapsed: bool,
    ) -> BoxFuture<'_, Result<(), StoreError>>;
    /// Reminders kept by Parla itself, ordered by due time (none last), then id.
    fn local_reminders(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<domain::Reminder>, StoreError>>;
    /// Ascending: messages strictly newer than `after`, at most `limit`.
    /// Thread replies that are not `broadcast` are excluded.
    fn messages_after(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        after: &MessageTs,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Message>, StoreError>>;
    /// Most recently used first.
    fn recent_emoji(
        &self,
        ws: &WorkspaceKey,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<EmojiRef>, StoreError>>;
    /// Records a use at `at_ms`; keeps the newest [`RECENT_EMOJI_CAP`].
    fn emoji_used(
        &self,
        ws: &WorkspaceKey,
        e: EmojiRef,
        at_ms: i64,
    ) -> BoxFuture<'_, Result<(), StoreError>>;
    /// Text (when non-empty) plus the query's filters, applied locally.
    /// Dates are exclusive whole UTC days; `Has::File` and `Has::Reaction`
    /// are honoured, `Link` and `Pin` are not stored and are ignored.
    fn search_filtered(
        &self,
        ws: &WorkspaceKey,
        q: &SearchQuery,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<SearchHit>, StoreError>>;
    /// The workspace's user groups, ordered by handle then id (at most
    /// `domain::MAX_USER_GROUPS`).
    fn user_groups(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<domain::UserGroup>, StoreError>>;
    /// The files shared in `c`, newest first: those of messages strictly
    /// older than `before` (thread replies included), each message's files in
    /// order, deleted files left out. A page reads at most `limit` messages
    /// (clamped to [`FILES_PAGE_MAX`]) and stops after the message that
    /// reaches `limit` files, so a message's files are never split. `next`
    /// holds the oldest message's timestamp when older files may exist.
    fn conversation_files(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: usize,
    ) -> BoxFuture<'_, Result<domain::Page<domain::FileHit>, StoreError>>;
    /// Test-support only: removes everything stored for one workspace.
    #[cfg(feature = "test-support")]
    fn wipe(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<(), StoreError>>;
}
