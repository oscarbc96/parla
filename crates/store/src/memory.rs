use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;
use std::sync::Mutex;

use std::sync::Arc;

use domain::{
    ClientMsgId, ComposedText, Conversation, ConversationId, ConversationPrefs, DraftKey, EmojiRef,
    Gap, LocalSchedule, Member, Message, MessageTs, OutboxItem, PersistOp, ScheduleId, SearchHit,
    SearchQuery, UserId,
};
use futures::future::{BoxFuture, FutureExt};
use settings::WorkspaceKey;

use crate::{Store, StoreError};

#[derive(Default)]
struct Workspace {
    conversations: BTreeMap<ConversationId, Conversation>,
    members: BTreeMap<UserId, Member>,
    messages: HashMap<ConversationId, BTreeMap<MessageTs, Message>>,
    gaps: HashMap<ConversationId, Vec<Gap>>,
    outbox: HashMap<ClientMsgId, OutboxItem>,
    drafts: HashMap<DraftKey, (ComposedText, i64)>,
    collapsed: std::collections::BTreeSet<String>,
    schedules: BTreeMap<ScheduleId, LocalSchedule>,
    notify_prefs: HashMap<ConversationId, ConversationPrefs>,
    notify_extras: HashMap<ConversationId, domain::NotifyExtras>,
    starred: std::collections::HashSet<ConversationId>,
    reminders: BTreeMap<domain::ReminderId, domain::Reminder>,
    /// `(emoji, used_ms)`, unordered.
    emoji: HashMap<Arc<str>, (EmojiRef, i64)>,
    user_groups: Vec<domain::UserGroup>,
}

/// A thread reply that is not shown in the channel window.
fn hidden_in_channel(m: &Message) -> bool {
    !m.broadcast && m.thread_root.as_ref().is_some_and(|r| *r != m.id.ts)
}

/// How a [`MemoryStore`]'s writes (`apply`, `outbox_put`, `outbox_remove`)
/// answer. Tests use it to stand in for a full disk or a stuck writer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WriteFault {
    #[default]
    None,
    /// Every write fails at once, writing nothing (a full disk).
    Fail,
    /// Every write never answers (a stuck writer).
    Stall,
}

/// In-memory `Store`; state is per workspace key and lost on drop.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<HashMap<WorkspaceKey, Workspace>>,
    fault: Mutex<WriteFault>,
}

impl MemoryStore {
    /// Makes every later write answer as `f` says.
    pub fn set_write_fault(&self, f: WriteFault) {
        *self.fault.lock().unwrap_or_else(|e| e.into_inner()) = f;
    }

    /// The answer of a faulted write, or `None` to write normally.
    fn faulted(&self) -> Option<BoxFuture<'static, Result<(), StoreError>>> {
        match *self.fault.lock().unwrap_or_else(|e| e.into_inner()) {
            WriteFault::None => None,
            WriteFault::Fail => Some(async { Err(StoreError::Other("disk full".into())) }.boxed()),
            WriteFault::Stall => Some(futures::future::pending().boxed()),
        }
    }

    fn with<R>(
        &self,
        ws: &WorkspaceKey,
        f: impl FnOnce(&mut Workspace) -> R,
    ) -> Result<R, StoreError> {
        let mut g = self.inner.lock().map_err(|_| StoreError::Closed)?;
        Ok(f(g.entry(ws.clone()).or_default()))
    }

    fn peek<R>(&self, ws: &WorkspaceKey, f: impl FnOnce(&Workspace) -> R) -> Result<R, StoreError> {
        let g = self.inner.lock().map_err(|_| StoreError::Closed)?;
        Ok(match g.get(ws) {
            Some(w) => f(w),
            None => f(&Workspace::default()),
        })
    }
}

impl Store for MemoryStore {
    fn apply(&self, ws: &WorkspaceKey, op: PersistOp) -> BoxFuture<'_, Result<(), StoreError>> {
        if let Some(f) = self.faulted() {
            return f;
        }
        let r = self.with(ws, |w| match op {
            PersistOp::Conversations(cs) => {
                for c in cs {
                    w.conversations.insert(c.id.clone(), c);
                }
            }
            PersistOp::Members(ms) => {
                for m in ms {
                    w.members.insert(m.id.clone(), m);
                }
            }
            PersistOp::Messages {
                conversation,
                messages,
            } => {
                let map = w.messages.entry(conversation).or_default();
                for m in messages {
                    map.insert(m.id.ts.clone(), m);
                }
            }
            PersistOp::EditMessage {
                id,
                text,
                plain,
                edited,
            } => {
                if let Some(m) = w
                    .messages
                    .get_mut(&id.conversation)
                    .and_then(|m| m.get_mut(&id.ts))
                {
                    m.text = text;
                    m.plain = plain;
                    m.edited = Some(edited);
                }
            }
            PersistOp::DeleteMessage(id) => {
                if let Some(map) = w.messages.get_mut(&id.conversation) {
                    map.remove(&id.ts);
                }
            }
            PersistOp::ReadMarker { conversation, ts } => {
                if let Some(c) = w.conversations.get_mut(&conversation) {
                    c.last_read = Some(ts);
                }
            }
            PersistOp::ReplaceMessage(m) => {
                w.messages
                    .entry(m.id.conversation.clone())
                    .or_default()
                    .insert(m.id.ts.clone(), m);
            }
            PersistOp::Reactions { id, reactions } => {
                if let Some(m) = w
                    .messages
                    .get_mut(&id.conversation)
                    .and_then(|m| m.get_mut(&id.ts))
                {
                    m.reactions = reactions;
                }
            }
            PersistOp::ThreadSummary {
                root,
                reply_count,
                latest_reply,
            } => {
                if let Some(m) = w
                    .messages
                    .get_mut(&root.conversation)
                    .and_then(|m| m.get_mut(&root.ts))
                {
                    m.reply_count = reply_count;
                    m.latest_reply = latest_reply;
                }
            }
            PersistOp::AddGap(g) => {
                let v = w.gaps.entry(g.conversation.clone()).or_default();
                if !v.contains(&g) {
                    v.push(g);
                }
            }
            PersistOp::CloseGap(c) => {
                if let Some(v) = w.gaps.get_mut(&c.conversation) {
                    let out: Vec<Gap> = v.drain(..).flat_map(|g| g.subtract(&c)).collect();
                    *v = out;
                }
            }
            // The in-memory conversation model keeps no last-opened time.
            PersistOp::Opened { .. } => {}
            PersistOp::LocalSchedulePut(l) => {
                w.schedules.insert(l.id.clone(), l);
            }
            PersistOp::LocalScheduleRemove(id) => {
                w.schedules.remove(&id);
            }
            PersistOp::LocalNotifyPrefsPut {
                conversation,
                prefs,
            } => {
                if prefs == ConversationPrefs::default() {
                    w.notify_prefs.remove(&conversation);
                } else {
                    w.notify_prefs.insert(conversation, prefs);
                }
            }
            PersistOp::LocalNotifyExtrasPut {
                conversation,
                extras,
            } => {
                if extras == domain::NotifyExtras::default() {
                    w.notify_extras.remove(&conversation);
                } else {
                    w.notify_extras.insert(conversation, extras);
                }
            }
            PersistOp::LocalStarredPut {
                conversation,
                starred,
            } => {
                if starred {
                    w.starred.insert(conversation);
                } else {
                    w.starred.remove(&conversation);
                }
            }
            PersistOp::LocalReminderPut(r) => {
                w.reminders.insert(r.id.clone(), r);
            }
            PersistOp::LocalReminderRemove(id) => {
                w.reminders.remove(&id);
            }
            PersistOp::UserGroups(mut all) => {
                // Handle, then id: the order the sqlite store reads.
                all.sort_by(|a, b| a.handle.cmp(&b.handle).then_with(|| a.id.cmp(&b.id)));
                all.truncate(domain::MAX_USER_GROUPS);
                w.user_groups = all;
            }
            PersistOp::FileDeleted(file) => {
                for m in w.messages.values_mut().flat_map(|map| map.values_mut()) {
                    domain::tombstone_file(&mut m.files, &file);
                }
            }
        });
        async move { r }.boxed()
    }

    fn conversations(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<Conversation>, StoreError>> {
        let r = self.peek(ws, |w| w.conversations.values().cloned().collect());
        async move { r }.boxed()
    }

    fn members(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<Vec<Member>, StoreError>> {
        let r = self.peek(ws, |w| w.members.values().cloned().collect());
        async move { r }.boxed()
    }

    fn messages_before(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Message>, StoreError>> {
        let r = self.peek(ws, |w| {
            let Some(map) = w.messages.get(c) else {
                return Vec::new();
            };
            let upper = before.as_ref().map_or(Bound::Unbounded, Bound::Excluded);
            map.range((Bound::Unbounded, upper))
                .rev()
                .filter(|(_, m)| !hidden_in_channel(m))
                .take(limit)
                .map(|(_, m)| m.clone())
                .collect()
        });
        async move { r }.boxed()
    }

    #[cfg(feature = "test-support")]
    fn latest_ts(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Option<MessageTs>, StoreError>> {
        let r = self.peek(ws, |w| {
            w.messages
                .get(c)
                .and_then(|m| m.keys().next_back().cloned())
        });
        async move { r }.boxed()
    }

    fn thread(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        root: &MessageTs,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Message>, StoreError>> {
        let r = self.peek(ws, |w| {
            w.messages.get(c).map_or_else(Vec::new, |map| {
                map.values()
                    .filter(|m| m.id.ts == *root || m.thread_root.as_ref() == Some(root))
                    .take(limit)
                    .cloned()
                    .collect()
            })
        });
        async move { r }.boxed()
    }

    fn latest_all(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, MessageTs>, StoreError>> {
        let r = self.peek(ws, |w| {
            w.messages
                .iter()
                .filter_map(|(c, m)| {
                    let (ts, _) = m.iter().rev().find(|(_, m)| !hidden_in_channel(m))?;
                    Some((c.clone(), ts.clone()))
                })
                .collect()
        });
        async move { r }.boxed()
    }

    fn gaps(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Vec<Gap>, StoreError>> {
        let r = self.peek(ws, |w| {
            let mut v = w.gaps.get(c).cloned().unwrap_or_default();
            v.sort_by(|a, b| a.after.cmp(&b.after));
            v
        });
        async move { r }.boxed()
    }

    fn outbox(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<Vec<OutboxItem>, StoreError>> {
        let r = self.peek(ws, |w| {
            let mut v: Vec<_> = w.outbox.values().cloned().collect();
            v.sort_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)));
            v
        });
        async move { r }.boxed()
    }

    fn outbox_put(
        &self,
        ws: &WorkspaceKey,
        item: OutboxItem,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        if let Some(f) = self.faulted() {
            return f;
        }
        let r = self.with(ws, |w| {
            w.outbox.insert(item.id.clone(), item);
        });
        async move { r }.boxed()
    }

    fn outbox_remove(
        &self,
        ws: &WorkspaceKey,
        id: &ClientMsgId,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        if let Some(f) = self.faulted() {
            return f;
        }
        let r = self.with(ws, |w| {
            w.outbox.remove(id);
        });
        async move { r }.boxed()
    }

    fn drafts(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<Vec<crate::Draft>, StoreError>> {
        let r = self.peek(ws, |w| {
            let mut v: Vec<_> = w
                .drafts
                .iter()
                .map(|(k, (b, at))| (k.clone(), b.clone(), *at))
                .collect();
            // The order SQLite returns: conversation, then thread (none first).
            v.sort_by(|(a, _, _), (b, _, _)| {
                let key = |k: &DraftKey| {
                    (
                        k.conversation.0.clone(),
                        k.thread.as_ref().map(|t| t.0.clone()),
                    )
                };
                key(a).cmp(&key(b))
            });
            v
        });
        async move { r }.boxed()
    }

    fn draft_put(
        &self,
        ws: &WorkspaceKey,
        key: DraftKey,
        body: Option<ComposedText>,
        edited: i64,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let r = self.with(ws, |w| match body {
            Some(b) if !b.text.trim().is_empty() => {
                w.drafts.insert(key, (b, edited));
            }
            _ => {
                w.drafts.remove(&key);
            }
        });
        async move { r }.boxed()
    }

    fn message(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        ts: &MessageTs,
    ) -> BoxFuture<'_, Result<Option<Message>, StoreError>> {
        let r = self.peek(ws, |w| w.messages.get(c).and_then(|m| m.get(ts)).cloned());
        async move { r }.boxed()
    }

    fn search(
        &self,
        ws: &WorkspaceKey,
        query: &str,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<SearchHit>, StoreError>> {
        let needle = query.to_lowercase();
        let r = self.peek(ws, |w| {
            if needle.is_empty() {
                return Vec::new();
            }
            let mut hits: Vec<&Message> = w
                .messages
                .values()
                .flat_map(|m| m.values())
                .filter(|m| m.plain.to_lowercase().contains(&needle))
                .collect();
            hits.sort_by(|a, b| b.id.ts.cmp(&a.id.ts));
            hits.into_iter()
                .take(limit)
                .map(|m| SearchHit {
                    id: m.id.clone(),
                    snippet: m.plain.clone(),
                    rank: 0.0,
                })
                .collect()
        });
        async move { r }.boxed()
    }

    fn local_notify_prefs(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, ConversationPrefs>, StoreError>> {
        let r = self.peek(ws, |w| w.notify_prefs.clone());
        async move { r }.boxed()
    }

    fn sidebar_collapsed(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<std::collections::BTreeSet<String>, StoreError>> {
        let r = self.peek(ws, |w| w.collapsed.clone());
        async move { r }.boxed()
    }

    fn sidebar_collapsed_put(
        &self,
        ws: &WorkspaceKey,
        section: String,
        collapsed: bool,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let r = self.with(ws, |w| {
            if collapsed {
                w.collapsed.insert(section);
            } else {
                w.collapsed.remove(&section);
            }
        });
        async move { r }.boxed()
    }

    fn local_notify_extras(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, domain::NotifyExtras>, StoreError>> {
        let r = self.peek(ws, |w| w.notify_extras.clone());
        async move { r }.boxed()
    }

    fn local_starred(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<std::collections::HashSet<ConversationId>, StoreError>> {
        let r = self.peek(ws, |w| w.starred.clone());
        async move { r }.boxed()
    }

    fn local_reminders(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<domain::Reminder>, StoreError>> {
        let r = self.peek(ws, |w| {
            let mut v: Vec<_> = w.reminders.values().cloned().collect();
            // Due time (none last), then id: the order the sqlite store reads.
            v.sort_by(|a, b| {
                a.due
                    .is_none()
                    .cmp(&b.due.is_none())
                    .then(a.due.cmp(&b.due))
                    .then_with(|| a.id.cmp(&b.id))
            });
            v
        });
        async move { r }.boxed()
    }

    fn local_schedules(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<LocalSchedule>, StoreError>> {
        let r = self.peek(ws, |w| {
            let mut v: Vec<_> = w.schedules.values().cloned().collect();
            v.sort_by(|a, b| a.post_at.cmp(&b.post_at).then_with(|| a.id.cmp(&b.id)));
            v
        });
        async move { r }.boxed()
    }

    fn messages_after(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        after: &MessageTs,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<Message>, StoreError>> {
        let r = self.peek(ws, |w| {
            let Some(map) = w.messages.get(c) else {
                return Vec::new();
            };
            map.range((Bound::Excluded(after), Bound::Unbounded))
                .filter(|(_, m)| !hidden_in_channel(m))
                .take(limit)
                .map(|(_, m)| m.clone())
                .collect()
        });
        async move { r }.boxed()
    }

    fn recent_emoji(
        &self,
        ws: &WorkspaceKey,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<EmojiRef>, StoreError>> {
        let r = self.peek(ws, |w| {
            let mut v: Vec<_> = w.emoji.values().cloned().collect();
            v.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then_with(|| a.0.shortcode.cmp(&b.0.shortcode))
            });
            v.into_iter().take(limit).map(|(e, _)| e).collect()
        });
        async move { r }.boxed()
    }

    fn emoji_used(
        &self,
        ws: &WorkspaceKey,
        e: EmojiRef,
        at_ms: i64,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        let r = self.with(ws, |w| {
            w.emoji.insert(e.shortcode.clone(), (e, at_ms));
            if w.emoji.len() > crate::RECENT_EMOJI_CAP {
                let oldest = w
                    .emoji
                    .iter()
                    .min_by(|a, b| a.1.1.cmp(&b.1.1).then_with(|| b.0.cmp(a.0)))
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    w.emoji.remove(&k);
                }
            }
        });
        async move { r }.boxed()
    }

    fn search_filtered(
        &self,
        ws: &WorkspaceKey,
        q: &SearchQuery,
        limit: usize,
    ) -> BoxFuture<'_, Result<Vec<SearchHit>, StoreError>> {
        let terms: Vec<String> = q
            .text
            .split_whitespace()
            .map(|t| t.trim_matches(|c| c == '"' || c == '*').to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        let before = q.before.map(crate::day_start_secs);
        let after = q
            .after
            .map(|d| crate::day_start_secs(d).saturating_add(86_400));
        let r = self.peek(ws, |w| {
            let mut hits: Vec<&Message> = w
                .messages
                .iter()
                .filter(|(c, _)| q.conversations.is_empty() || q.conversations.contains(c))
                .flat_map(|(_, m)| m.values())
                .filter(|m| {
                    q.authors.is_empty() || m.author.as_ref().is_some_and(|a| q.authors.contains(a))
                })
                .filter(|m| {
                    let secs =
                        m.id.ts
                            .0
                            .split('.')
                            .next()
                            .and_then(|s| s.parse::<i64>().ok())
                            .unwrap_or(0);
                    before.is_none_or(|b| secs < b) && after.is_none_or(|a| secs >= a)
                })
                .filter(|m| {
                    q.has.iter().all(|h| match h {
                        domain::Has::File => !m.files.is_empty(),
                        domain::Has::Reaction => !m.reactions.is_empty(),
                        domain::Has::Link | domain::Has::Pin => true,
                    })
                })
                .filter(|m| {
                    let plain = m.plain.to_lowercase();
                    terms.iter().all(|t| plain.contains(t.as_str()))
                })
                .collect();
            hits.sort_by(|a, b| b.id.ts.cmp(&a.id.ts));
            hits.into_iter()
                .take(limit)
                .map(|m| SearchHit {
                    id: m.id.clone(),
                    snippet: m.plain.clone(),
                    rank: 0.0,
                })
                .collect()
        });
        async move { r }.boxed()
    }

    fn user_groups(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<domain::UserGroup>, StoreError>> {
        let r = self.peek(ws, |w| w.user_groups.clone());
        async move { r }.boxed()
    }

    fn conversation_files(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: usize,
    ) -> BoxFuture<'_, Result<domain::Page<domain::FileHit>, StoreError>> {
        let limit = limit.clamp(1, crate::FILES_PAGE_MAX);
        let r = self.peek(ws, |w| {
            let Some(map) = w.messages.get(c) else {
                return Vec::new();
            };
            let upper = before.as_ref().map_or(Bound::Unbounded, Bound::Excluded);
            map.range((Bound::Unbounded, upper))
                .rev()
                .filter(|(_, m)| !m.files.is_empty())
                .take(limit)
                .map(|(_, m)| m.clone())
                .collect()
        });
        async move { r.map(|v| crate::files_page(v, limit)) }.boxed()
    }

    #[cfg(feature = "test-support")]
    fn wipe(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<(), StoreError>> {
        let r = self
            .inner
            .lock()
            .map_err(|_| StoreError::Closed)
            .map(|mut g| {
                g.remove(ws);
            });
        async move { r }.boxed()
    }
}
