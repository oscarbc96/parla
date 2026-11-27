//! Cross-workspace views: "All unreads", "All mentions" and the badge total.

use std::sync::Arc;

use settings::WorkspaceKey;

use crate::ids::{ConversationId, MessageId, MessageTs};
use crate::model::{ConversationKind, DeliveryState, Message};
use crate::schedule::{LocalScheduleState, ScheduleId};
use crate::state::WorkspaceState;
use crate::unread::counts_as_mention;

#[derive(Clone, Copy)]
pub struct WorkspaceRef<'a> {
    pub key: &'a WorkspaceKey,
    pub state: &'a WorkspaceState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateRow {
    pub workspace: WorkspaceKey,
    pub conversation: ConversationId,
    pub name: Arc<str>,
    pub kind: ConversationKind,
    pub unread: bool,
    pub mentions: u32,
    pub latest: Option<MessageTs>,
    pub last_read: Option<MessageTs>,
}

fn rows<'a>(
    ws: impl IntoIterator<Item = WorkspaceRef<'a>>,
    keep: impl Fn(&crate::model::Conversation) -> bool,
) -> Vec<(AggregateRow, &'a WorkspaceState)> {
    let mut out = Vec::new();
    for w in ws {
        for c in w.state.conversations.values().filter(|c| keep(c)) {
            out.push((
                AggregateRow {
                    workspace: w.key.clone(),
                    conversation: c.id.clone(),
                    name: c.name.clone(),
                    kind: c.kind,
                    unread: c.unread,
                    mentions: c.mentions,
                    latest: c.latest.clone(),
                    last_read: c.last_read.clone(),
                },
                w.state,
            ));
        }
    }
    // Mentions first, then newest first.
    out.sort_by(|(a, _), (b, _)| {
        (b.mentions > 0)
            .cmp(&(a.mentions > 0))
            .then_with(|| b.latest.cmp(&a.latest))
    });
    out
}

/// Unread conversations (muted ones only with mentions): mentions first,
/// then newest first.
pub fn all_unreads<'a>(ws: impl IntoIterator<Item = WorkspaceRef<'a>>) -> Vec<AggregateRow> {
    rows(ws, |c| c.unread && (!c.is_muted || c.mentions > 0))
        .into_iter()
        .map(|(r, _)| r)
        .collect()
}

/// Conversations with mentions, each with the held messages after
/// `last_read` that mention the user, newest first.
pub fn all_mentions<'a>(
    ws: impl IntoIterator<Item = WorkspaceRef<'a>>,
) -> Vec<(AggregateRow, Vec<MessageId>)> {
    rows(ws, |c| c.mentions > 0)
        .into_iter()
        .map(|(r, st)| {
            let mut ids = Vec::new();
            if let (Some(me), Some(w), Some(conv)) = (
                &st.self_user,
                st.windows.get(&r.conversation),
                st.conversations.get(&r.conversation),
            ) {
                let read = conv.last_read.as_ref();
                for m in w.messages().iter().rev() {
                    if read.is_none_or(|t| m.id.ts > *t)
                        && m.author.as_ref() != Some(me)
                        && counts_as_mention(m, me, conv, &st.user_groups)
                    {
                        ids.push(m.id.clone());
                    }
                }
            }
            (r, ids)
        })
        .collect()
}

/// The newest `max` held messages of `conversation` after its `last_read`
/// (delivered, not mine; only mentions when muted), oldest first: the previews of an "All unreads" row. Empty when the
/// conversation has no held window.
pub fn unread_previews<'a>(
    state: &'a WorkspaceState,
    conversation: &ConversationId,
    max: usize,
) -> Vec<&'a Message> {
    let (Some(w), Some(conv)) = (
        state.windows.get(conversation),
        state.conversations.get(conversation),
    ) else {
        return Vec::new();
    };
    let read = conv.last_read.as_ref();
    let mut out: Vec<&Message> = w
        .messages()
        .iter()
        .rev()
        .filter(|m| read.is_none_or(|t| m.id.ts > *t))
        // Never my own or unsent messages; a muted (mention-only) row shows
        // the mentions that put it there.
        .filter(|m| m.state == DeliveryState::Delivered)
        .filter(|m| match &state.self_user {
            Some(me) => {
                m.author.as_ref() != Some(me)
                    && (!conv.is_muted || counts_as_mention(m, me, conv, &state.user_groups))
            }
            None => !conv.is_muted,
        })
        .take(max)
        .collect();
    out.reverse();
    out
}

/// One scheduled message of any workspace, provider-side or local.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateScheduled {
    pub workspace: WorkspaceKey,
    pub id: ScheduleId,
    pub conversation: ConversationId,
    pub text: Arc<str>,
    /// Unix seconds.
    pub post_at: i64,
    /// Kept and sent by Parla itself.
    pub local: bool,
    pub state: Option<LocalScheduleState>,
}

/// Every scheduled message of every workspace, soonest first.
pub fn all_scheduled<'a>(
    ws: impl IntoIterator<Item = WorkspaceRef<'a>>,
) -> Vec<AggregateScheduled> {
    let mut out = Vec::new();
    for w in ws {
        out.extend(w.state.scheduled.iter().map(|m| AggregateScheduled {
            workspace: w.key.clone(),
            id: m.id.clone(),
            conversation: m.conversation.clone(),
            text: m.text.clone(),
            post_at: m.post_at,
            local: false,
            state: None,
        }));
        out.extend(w.state.local_schedules.iter().map(|m| AggregateScheduled {
            workspace: w.key.clone(),
            id: m.id.clone(),
            conversation: m.conversation.clone(),
            text: m.text.clone(),
            post_at: m.post_at,
            local: true,
            state: Some(m.state.clone()),
        }));
    }
    out.sort_by_key(|r| r.post_at);
    out
}
