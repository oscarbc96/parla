//! Unread and mention engine: pure helpers used by the reducer.

use crate::ids::{ConversationId, MessageId, MessageTs, UserId};
use crate::model::PersistOp;
use crate::model::{Conversation, ConversationKind, DeliveryState, Message};
use crate::reduce::{Changed, Effect};
use crate::rich::Mention;
use crate::state::WorkspaceState;
use crate::sync_types::ConversationCounts;
use crate::user_groups::{UserGroups, mentions_my_group};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnreadTotals {
    pub conversations: u32,
    pub mentions: u32,
}

/// Which conversation has the user's attention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Focus {
    pub conversation: ConversationId,
    pub at_bottom: bool,
    pub window_active: bool,
}

/// Direct mention of the user.
pub(crate) fn direct_mention(m: &Message, me: &UserId) -> bool {
    m.mentions
        .iter()
        .any(|x| matches!(x, Mention::User(u) if u == me))
}

/// True for `Mention::User(self)`, any broadcast, a group the user is in
/// (from `groups`), and every message in a direct or group conversation.
pub fn mentions_self(
    m: &Message,
    self_user: &UserId,
    kind: ConversationKind,
    groups: Option<&UserGroups>,
) -> bool {
    matches!(kind, ConversationKind::Direct | ConversationKind::Group)
        || direct_mention(m, self_user)
        || m.mentions
            .iter()
            .any(|x| matches!(x, Mention::Broadcast(_)))
        || mentions_my_group(&m.mentions, groups)
}

/// Mention rule for one incoming message (muted: direct mentions only).
pub(crate) fn counts_as_mention(
    m: &Message,
    me: &UserId,
    conv: &Conversation,
    groups: &UserGroups,
) -> bool {
    if conv.is_muted {
        direct_mention(m, me)
    } else {
        mentions_self(m, me, conv.kind, Some(groups))
    }
}

impl WorkspaceState {
    pub fn totals(&self) -> UnreadTotals {
        let mut t = UnreadTotals::default();
        // Only conversations the user is in count (a previewed channel is not).
        for c in self.conversations.values().filter(|c| c.is_member) {
            if !c.is_muted && c.unread {
                t.conversations += 1;
            }
            t.mentions += c.mentions;
        }
        t
    }
}

pub(crate) fn counts_loaded(
    state: &mut WorkspaceState,
    all: Vec<ConversationCounts>,
    fx: &mut Vec<Effect>,
) {
    for k in all {
        if let Some(c) = state.conversations.get_mut(&k.conversation) {
            // A snapshot older than a live post or a local mark-read must
            // not move `latest` or `last_read` backwards.
            c.latest = c.latest.take().max(k.latest);
            if c.last_read > k.last_read {
                // Already read further than the snapshot: its counts are stale.
                continue;
            }
            c.last_read = k.last_read;
            c.unread = k.unread;
            c.mentions = k.mentions;
        }
    }
    fx.push(Effect::Changed(Changed::Conversations));
}

/// A live post by someone else: update unread/mentions. Returns nothing; the
/// caller follows with `maybe_mark_read`.
pub(crate) fn on_live_post(state: &mut WorkspaceState, m: &Message) {
    let Some(me) = state.self_user.clone() else {
        return;
    };
    if m.author.as_ref() == Some(&me) {
        return;
    }
    let Some(conv) = state.conversations.get_mut(&m.id.conversation) else {
        return;
    };
    if conv.last_read.as_ref().is_some_and(|r| *r >= m.id.ts) {
        return;
    }
    conv.unread = true;
    if counts_as_mention(m, &me, conv, &state.user_groups) {
        conv.mentions += 1;
    }
}

fn is_attentive(state: &WorkspaceState, c: &ConversationId) -> bool {
    !state.lean
        && state.windows.get(c).is_none_or(|w| w.is_live())
        && state
            .focus
            .as_ref()
            .is_some_and(|f| f.conversation == *c && f.at_bottom && f.window_active)
        && !state.unread_hold.contains(c)
}

/// Emit `MarkRead` once when the focused conversation is at the bottom of an
/// active window and the newest ts is beyond `last_read`.
pub(crate) fn maybe_mark_read(
    state: &mut WorkspaceState,
    c: &ConversationId,
    fx: &mut Vec<Effect>,
) {
    if !is_attentive(state, c) {
        return;
    }
    let newest = state
        .windows
        .get(c)
        .and_then(|w| w.newest_ts().cloned())
        .into_iter()
        .chain(state.conversations.get(c).and_then(|x| x.latest.clone()))
        .max();
    let Some(ts) = newest else { return };
    let Some(conv) = state.conversations.get_mut(c) else {
        return;
    };
    if !conv.is_member {
        return;
    }
    if conv.last_read.as_ref().is_some_and(|r| *r >= ts) {
        return;
    }
    conv.last_read = Some(ts.clone());
    conv.unread = false;
    conv.mentions = 0;
    fx.push(Effect::Changed(Changed::Conversations));
    fx.push(Effect::Persist(PersistOp::ReadMarker {
        conversation: c.clone(),
        ts: ts.clone(),
    }));
    fx.push(Effect::MarkRead {
        conversation: c.clone(),
        ts,
    });
}

pub(crate) fn focus(
    state: &mut WorkspaceState,
    conversation: Option<ConversationId>,
    at_bottom: bool,
    window_active: bool,
    fx: &mut Vec<Effect>,
) {
    // Moving focus elsewhere lifts the "mark unread" hold on other conversations.
    state
        .unread_hold
        .retain(|c| conversation.as_ref() == Some(c));
    match conversation {
        Some(c) => {
            state.focus = Some(Focus {
                conversation: c.clone(),
                at_bottom,
                window_active,
            });
            maybe_mark_read(state, &c, fx);
            // Scrolled to the bottom of a detached window: load what follows.
            if at_bottom
                && let Some(w) = state.windows.get(&c).filter(|w| !w.is_live())
                && let Some(after) = w.newest_delivered_ts().cloned()
            {
                fx.push(Effect::FetchNewer {
                    conversation: c,
                    after,
                });
            }
        }
        None => state.focus = None,
    }
}

/// Recompute `unread` and `mentions` from the held window after the marker moved.
pub(crate) fn recompute(state: &mut WorkspaceState, c: &ConversationId) {
    let Some(me) = state.self_user.clone() else {
        return;
    };
    let held = state.windows.get(c);
    let groups = &state.user_groups;
    let Some(conv) = state.conversations.get_mut(c) else {
        return;
    };
    let Some(read) = conv.last_read.clone() else {
        return;
    };
    let latest = conv
        .latest
        .clone()
        .into_iter()
        .chain(held.and_then(|w| w.newest_ts().cloned()))
        .max();
    conv.unread = latest.as_ref().is_some_and(|l| *l > read);
    conv.mentions = if conv.unread {
        held.map_or(0, |w| {
            w.messages()
                .iter()
                .filter(|m| {
                    m.id.ts > read
                        && m.author.as_ref() != Some(&me)
                        && counts_as_mention(m, &me, conv, groups)
                })
                .count() as u32
        })
    } else {
        0
    };
}

/// Marks `c` read up to `ts` whether or not it is shown.
pub(crate) fn mark_read(
    state: &mut WorkspaceState,
    c: ConversationId,
    ts: MessageTs,
    fx: &mut Vec<Effect>,
) {
    let Some(conv) = state.conversations.get_mut(&c) else {
        return;
    };
    // Reading a channel the user is not in marks nothing on the server.
    if !conv.is_member {
        return;
    }
    if conv.last_read.as_ref().is_some_and(|r| *r >= ts) {
        return;
    }
    conv.last_read = Some(ts.clone());
    conv.unread = false;
    conv.mentions = 0;
    state.unread_hold.remove(&c);
    fx.push(Effect::Persist(PersistOp::ReadMarker {
        conversation: c.clone(),
        ts: ts.clone(),
    }));
    fx.push(Effect::MarkRead {
        conversation: c,
        ts,
    });
    fx.push(Effect::Changed(Changed::Conversations));
}

pub(crate) fn mark_unread(state: &mut WorkspaceState, id: MessageId, fx: &mut Vec<Effect>) {
    let c = id.conversation.clone();
    let prev: MessageTs = state
        .windows
        .get(&c)
        // Only confirmed rows: unconfirmed ones sit at the bottom with a local ts.
        .and_then(|w| {
            w.messages()
                .iter()
                .rev()
                .find(|m| m.state == DeliveryState::Delivered && m.id.ts < id.ts)
        })
        .map_or_else(|| id.ts.just_before(), |m| m.id.ts.clone());
    let Some(conv) = state.conversations.get_mut(&c) else {
        return;
    };
    conv.last_read = Some(prev.clone());
    conv.unread = true;
    if conv.latest.as_ref().is_none_or(|l| *l < id.ts) {
        conv.latest = Some(id.ts);
    }
    recompute(state, &c);
    state.unread_hold.insert(c.clone());
    fx.push(Effect::Persist(PersistOp::ReadMarker {
        conversation: c.clone(),
        ts: prev.clone(),
    }));
    fx.push(Effect::MarkRead {
        conversation: c,
        ts: prev,
    });
    fx.push(Effect::Changed(Changed::Conversations));
}
