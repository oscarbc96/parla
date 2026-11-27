//! Optimistic reactions. Echoes and double toggles are idempotent and counts
//! never go below zero.

use std::sync::Arc;

use crate::ids::{MessageId, UserId};
use crate::model::{Message, PersistOp, ProviderError};
use crate::reactions::{CustomEmoji, Reaction};
use crate::reduce::{Changed, Effect};
use crate::rich::EmojiRef;
use crate::state::WorkspaceState;
use crate::threads;

/// Applies one add/remove for `user`; true when anything changed.
fn apply(m: &mut Message, emoji: &EmojiRef, user: &UserId, is_self: bool, add: bool) -> bool {
    let mut v = m.reactions.to_vec();
    let pos = v.iter().position(|r| r.emoji == *emoji);
    let present = |r: &Reaction| r.users.contains(user) || (is_self && r.me);
    let changed = match (pos, add) {
        (Some(i), true) => {
            let r = &mut v[i];
            if present(r) {
                false
            } else {
                r.count += 1;
                let mut u = r.users.to_vec();
                u.push(user.clone());
                r.users = Arc::from(u);
                r.me |= is_self;
                true
            }
        }
        (None, true) => {
            v.push(Reaction {
                emoji: emoji.clone(),
                count: 1,
                users: Arc::from(vec![user.clone()]),
                me: is_self,
            });
            true
        }
        (Some(i), false) => {
            let r = &mut v[i];
            // Our own removal needs proof we reacted; an echo of one we
            // already applied finds nothing and is a no-op.
            if is_self && !present(r) {
                false
            } else {
                r.count = r.count.saturating_sub(1);
                r.users = r.users.iter().filter(|u| *u != user).cloned().collect();
                if is_self {
                    r.me = false;
                }
                if r.count == 0 {
                    v.remove(i);
                }
                true
            }
        }
        (None, false) => false,
    };
    if changed {
        m.reactions = Arc::from(v);
    }
    changed
}

struct Update {
    held: bool,
    changed: bool,
    reactions: Option<Arc<[Reaction]>>,
}

/// Applies to every held copy (channel window, thread replies and roots).
fn update(
    state: &mut WorkspaceState,
    id: &MessageId,
    emoji: &EmojiRef,
    user: &UserId,
    add: bool,
    fx: &mut Vec<Effect>,
) -> Update {
    let is_self = state.self_user.as_ref() == Some(user);
    let mut u = Update {
        held: false,
        changed: false,
        reactions: None,
    };
    let t = threads::for_each_copy(state, id, |m| {
        let hit = apply(m, emoji, user, is_self, add);
        if hit {
            u.reactions = Some(m.reactions.clone());
        }
        hit
    });
    u.held = t.held;
    u.changed = t.in_view() || t.referenced;
    t.emit(id, fx);
    u
}

fn persist(id: &MessageId, u: Update, fx: &mut Vec<Effect>) {
    if let Some(reactions) = u.reactions {
        fx.push(Effect::Persist(PersistOp::Reactions {
            id: id.clone(),
            reactions,
        }));
    }
}

pub(crate) fn react(
    state: &mut WorkspaceState,
    id: MessageId,
    emoji: EmojiRef,
    add: bool,
    fx: &mut Vec<Effect>,
) {
    let Some(me) = state.self_user.clone() else {
        return;
    };
    let u = update(state, &id, &emoji, &me, add, fx);
    if u.held && !u.changed {
        // Already in the requested state: nothing to send.
        return;
    }
    persist(&id, u, fx);
    fx.push(Effect::CallReact { id, emoji, add });
}

pub(crate) fn finished(
    state: &mut WorkspaceState,
    id: MessageId,
    emoji: EmojiRef,
    add: bool,
    result: Result<(), ProviderError>,
    fx: &mut Vec<Effect>,
) {
    if result.is_ok() {
        return;
    }
    let Some(me) = state.self_user.clone() else {
        return;
    };
    let u = update(state, &id, &emoji, &me, !add, fx);
    persist(&id, u, fx);
}

pub(crate) fn changed(
    state: &mut WorkspaceState,
    id: MessageId,
    emoji: EmojiRef,
    user: UserId,
    added: bool,
    fx: &mut Vec<Effect>,
) {
    let u = update(state, &id, &emoji, &user, added, fx);
    persist(&id, u, fx);
}

pub(crate) fn emoji_loaded(
    state: &mut WorkspaceState,
    all: Vec<CustomEmoji>,
    fx: &mut Vec<Effect>,
) {
    state.custom_emoji = all.into_iter().map(|e| (e.shortcode.clone(), e)).collect();
    fx.push(Effect::Changed(Changed::Emoji));
}
