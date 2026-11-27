//! Following threads: whether a thread's replies reach the user.
//!
//! The state shown is, in order: what the user or the provider said last, or
//! else whether the user wrote the root or one of the loaded replies (the
//! provider follows those for them). A toggle applies at once; a refusal rolls
//! back to the last state the provider confirmed, unless a newer toggle or a
//! provider event came after it.

use std::collections::HashMap;

use crate::calls::{CallOutput, ProviderCall};
use crate::ids::MessageId;
use crate::model::ProviderError;
use crate::reduce::{Changed, Effect};
use crate::state::WorkspaceState;

/// Most follow states held; past it the oldest without a call in flight go.
pub const MAX_THREAD_FOLLOW: usize = 512;

/// The follow states held, with the counter that orders them.
#[derive(Clone, Debug, Default)]
pub(crate) struct FollowStates {
    map: HashMap<MessageId, Entry>,
    next: u64,
}

#[derive(Clone, Debug)]
struct Entry {
    /// What is shown; `None` falls back to authorship.
    shown: Option<bool>,
    /// What the provider last confirmed or reported; `None` is authorship.
    confirmed: Option<bool>,
    /// Sequence of the newest toggle or provider event.
    latest: u64,
    /// Calls in flight, oldest first: the value asked and its sequence.
    inflight: Vec<(bool, u64)>,
    /// Insertion order, for eviction.
    born: u64,
}

impl FollowStates {
    fn tick(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn entry(&mut self, root: MessageId) -> &mut Entry {
        let born = self.tick();
        if !self.map.contains_key(&root) && self.map.len() >= MAX_THREAD_FOLLOW {
            self.evict();
        }
        self.map.entry(root).or_insert(Entry {
            shown: None,
            confirmed: None,
            latest: 0,
            inflight: Vec::new(),
            born,
        })
    }

    /// Drops the oldest state with no call in flight.
    fn evict(&mut self) {
        let oldest = self
            .map
            .iter()
            .filter(|(_, e)| e.inflight.is_empty())
            .min_by_key(|(_, e)| e.born)
            .map(|(r, _)| r.clone());
        if let Some(r) = oldest {
            self.map.remove(&r);
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
}

impl WorkspaceState {
    /// Whether the user follows the thread under `root`.
    pub fn thread_followed(&self, root: &MessageId) -> bool {
        if let Some(f) = self.thread_follow.map.get(root).and_then(|e| e.shown) {
            return f;
        }
        let Some(me) = self.self_user.as_ref() else {
            return false;
        };
        self.threads.get(root).is_some_and(|t| {
            t.root
                .iter()
                .chain(t.replies.messages())
                .any(|m| m.author.as_ref() == Some(me))
        })
    }

    /// Follow states held (a test hook for the bound).
    pub fn thread_follow_len(&self) -> usize {
        self.thread_follow.len()
    }

    /// Whether a follow state is held for `root` (a test hook for the bound).
    pub fn thread_follow_known(&self, root: &MessageId) -> bool {
        self.thread_follow.map.contains_key(root)
    }
}

fn changed(root: MessageId, fx: &mut Vec<Effect>) {
    fx.push(Effect::Changed(Changed::Thread(root)));
}

pub(crate) fn set(state: &mut WorkspaceState, root: MessageId, follow: bool, fx: &mut Vec<Effect>) {
    if !state.caps.thread_follow {
        return;
    }
    let seq = state.thread_follow.tick();
    let e = state.thread_follow.entry(root.clone());
    e.shown = Some(follow);
    e.latest = seq;
    e.inflight.push((follow, seq));
    changed(root.clone(), fx);
    let last_read = state
        .threads
        .get(&root)
        .and_then(|t| t.replies.newest_delivered_ts().cloned());
    fx.push(Effect::Call(ProviderCall::SetThreadFollowed {
        root,
        follow,
        last_read,
    }));
}

/// Applies a state the provider reported; true when the shown state changed.
fn confirm(state: &mut WorkspaceState, root: MessageId, followed: bool) -> bool {
    let was = state.thread_followed(&root);
    let seq = state.thread_follow.tick();
    let e = state.thread_follow.entry(root);
    e.shown = Some(followed);
    e.confirmed = Some(followed);
    e.latest = seq;
    was != followed
}

pub(crate) fn provider_changed(
    state: &mut WorkspaceState,
    root: MessageId,
    followed: bool,
    fx: &mut Vec<Effect>,
) {
    if confirm(state, root.clone(), followed) && state.threads.contains_key(&root) {
        changed(root, fx);
    }
}

/// A reply was sent: the provider follows the thread again, and so do we.
pub(crate) fn replied(state: &mut WorkspaceState, root: MessageId, fx: &mut Vec<Effect>) {
    let unfollowed = state
        .thread_follow
        .map
        .get(&root)
        .is_some_and(|e| e.shown == Some(false));
    if unfollowed && confirm(state, root.clone(), true) {
        changed(root, fx);
    }
}

pub(crate) fn call_finished(
    state: &mut WorkspaceState,
    call: &ProviderCall,
    result: &Result<CallOutput, ProviderError>,
    fx: &mut Vec<Effect>,
) {
    let ProviderCall::SetThreadFollowed { root, follow, .. } = call else {
        return;
    };
    let was = state.thread_followed(root);
    let Some(e) = state.thread_follow.map.get_mut(root) else {
        return;
    };
    let Some(pos) = e.inflight.iter().position(|(f, _)| f == follow) else {
        return;
    };
    let (_, seq) = e.inflight.remove(pos);
    if result.is_ok() {
        e.confirmed = Some(*follow);
    } else if seq == e.latest {
        e.shown = e.confirmed;
    }
    if state.thread_followed(root) != was || result.is_err() {
        changed(root.clone(), fx);
    }
}
