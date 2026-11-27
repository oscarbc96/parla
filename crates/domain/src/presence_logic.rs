//! Other people's presence (only those on screen are watched) and opening a
//! direct message with someone we have none with.

use std::collections::BTreeSet;

use crate::calls::{CallOutput, ProviderCall};
use crate::ids::UserId;
use crate::model::{ConversationKind, PersistOp, ProviderError};
use crate::reduce::{Changed, Effect};
use crate::state::WorkspaceState;

/// The people on screen changed: remember the set, drop what is no longer
/// watched (the map stays bounded by what is on screen) and tell the provider
/// once. Without `caps.presence_watch` nothing is kept or sent.
pub(crate) fn watch(state: &mut WorkspaceState, users: Vec<UserId>, fx: &mut Vec<Effect>) {
    if !state.caps.presence_watch {
        return;
    }
    let set: BTreeSet<UserId> = users.into_iter().collect();
    if set == state.presence_watch {
        return;
    }
    let before = state.presence.len();
    state.presence.retain(|u, _| set.contains(u));
    state.presence_watch = set;
    fx.push(Effect::Call(ProviderCall::WatchPresence(
        state.presence_watch.iter().cloned().collect(),
    )));
    if state.presence.len() != before {
        fx.push(Effect::Changed(Changed::Presence));
    }
}

/// A `Presence` event: kept for watched people only; a change signals the UI.
pub(crate) fn event(state: &mut WorkspaceState, user: UserId, active: bool, fx: &mut Vec<Effect>) {
    if !state.presence_watch.contains(&user) {
        return;
    }
    if state.presence.insert(user, active) != Some(active) {
        fx.push(Effect::Changed(Changed::Presence));
    }
}

/// `Input::OpenDirect`: ask the provider unless the direct message is known
/// (the host then just navigates to it).
pub(crate) fn open_direct(state: &WorkspaceState, user: UserId, fx: &mut Vec<Effect>) {
    let known = state
        .conversations
        .values()
        .any(|c| c.kind == ConversationKind::Direct && c.peer.as_ref() == Some(&user));
    if !known && state.caps.open_direct {
        fx.push(Effect::Call(ProviderCall::OpenDirect(user)));
    }
}

/// The answer of `OpenDirect`: the conversation joins the list (shown, as the
/// person just asked for it). A failure changes nothing here.
pub(crate) fn call_finished(
    state: &mut WorkspaceState,
    call: &ProviderCall,
    result: &Result<CallOutput, ProviderError>,
    fx: &mut Vec<Effect>,
) {
    let (ProviderCall::OpenDirect(_), Ok(CallOutput::Conversation(c))) = (call, result) else {
        return;
    };
    let mut c = c.clone();
    c.is_open = true;
    state.conversations.insert(c.id.clone(), c.clone());
    fx.push(Effect::Persist(PersistOp::Conversations(vec![c])));
    fx.push(Effect::Changed(Changed::Conversations));
}
