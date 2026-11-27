//! Jump to any message: a window detached from the live end holds the
//! neighbourhood of the target until the user scrolls back down.

use crate::ids::{ConversationId, MessageId};
use crate::model::{HistoryPage, Message, PersistOp};
use crate::reduce::{Changed, Effect, window};
use crate::search::AroundPage;
use crate::state::WorkspaceState;
use crate::threads;

/// A held message: in a channel window or in any open thread's replies.
fn held(state: &WorkspaceState, id: &MessageId) -> Option<(Message, bool)> {
    if let Some(m) = state
        .windows
        .get(&id.conversation)
        .and_then(|w| w.messages().iter().find(|m| m.id == *id))
    {
        return Some((m.clone(), true));
    }
    state
        .threads
        .values()
        .find_map(|t| t.replies.messages().iter().find(|m| m.id == *id))
        .map(|m| (m.clone(), false))
}

fn mark(state: &mut WorkspaceState, id: MessageId, fx: &mut Vec<Effect>) {
    let c = id.conversation.clone();
    state.jump = Some(id);
    fx.push(Effect::Changed(Changed::Jump(c)));
}

/// Opens the thread of `reply` with the reply as its jump target, then
/// brings the root into view in the channel.
fn open_reply(
    state: &mut WorkspaceState,
    reply: Message,
    root: crate::ids::MessageTs,
    fx: &mut Vec<Effect>,
) {
    let root = threads::root_id(&reply.id.conversation, &root);
    threads::entry(state, &root).replies.insert(reply.clone());
    threads::open_as(state, root.clone(), false, fx);
    if let Some(t) = state.threads.get_mut(&root) {
        t.jump = Some(reply.id);
    }
    let in_window = state
        .windows
        .get(&root.conversation)
        .is_some_and(|w| w.messages().iter().any(|m| m.id == root));
    if in_window {
        mark(state, root, fx);
    } else {
        fx.push(Effect::FetchAround { target: root });
    }
}

pub(crate) fn jump_to(state: &mut WorkspaceState, id: MessageId, fx: &mut Vec<Effect>) {
    match held(state, &id) {
        Some((m, in_channel)) => match m.thread_root.clone() {
            Some(r) if !(in_channel && m.broadcast) => open_reply(state, m, r, fx),
            _ if in_channel => mark(state, id, fx),
            _ => fx.push(Effect::FetchAround { target: id }),
        },
        None => fx.push(Effect::FetchAround { target: id }),
    }
}

pub(crate) fn around_loaded(
    state: &mut WorkspaceState,
    target: MessageId,
    page: AroundPage,
    fx: &mut Vec<Effect>,
) {
    let c = target.conversation.clone();
    let found = page.messages.iter().find(|m| m.id == target).cloned();
    if let Some(m) = &found
        && let Some(r) = m.thread_root.clone()
        && !m.broadcast
    {
        fx.push(Effect::Persist(PersistOp::Messages {
            conversation: c,
            messages: vec![m.clone()],
        }));
        open_reply(state, m.clone(), r, fx);
        return;
    }
    window(state, &c).replace(
        page.messages.clone(),
        Some(&target),
        page.has_more_before,
        page.has_more_after,
    );
    fx.push(Effect::Persist(PersistOp::Messages {
        conversation: c.clone(),
        messages: page.messages,
    }));
    fx.push(Effect::Changed(Changed::Timeline(c)));
    if found.is_some() {
        mark(state, target, fx);
    }
}

pub(crate) fn newer_loaded(
    state: &mut WorkspaceState,
    c: ConversationId,
    page: HistoryPage,
    fx: &mut Vec<Effect>,
) {
    let Some(w) = state.windows.get_mut(&c).filter(|w| !w.is_live()) else {
        return;
    };
    let more = page.has_more && !page.messages.is_empty();
    for m in page.messages.iter().cloned() {
        w.insert(m);
    }
    w.set_has_more_after(more);
    fx.push(Effect::Persist(PersistOp::Messages {
        conversation: c.clone(),
        messages: page.messages,
    }));
    fx.push(Effect::Changed(Changed::Timeline(c.clone())));
    if !more {
        crate::unread::maybe_mark_read(state, &c, fx);
    }
}

pub(crate) fn to_latest(state: &mut WorkspaceState, c: ConversationId, fx: &mut Vec<Effect>) {
    state.windows.remove(&c);
    if state.jump.as_ref().is_some_and(|j| j.conversation == c) {
        state.jump = None;
    }
    fx.push(Effect::Reload(c.clone()));
    fx.push(Effect::Changed(Changed::Timeline(c)));
}
