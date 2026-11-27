//! On-demand member lookup. A provider may bring only part of its directory
//! at bootstrap (large accounts stream the rest, and bots are never in it),
//! so authors and thread repliers on screen can be unknown. Each message
//! change scans the touched window or thread for ids with no member and asks
//! the provider for them in one batched `Load::Members`.
//!
//! Bounded and quiet: at most [`MAX_BATCH`] ids per request, an id is in
//! flight at most once, and an id the provider could not resolve is
//! remembered as failed for the session and never asked again. A transient
//! failure (rate limit, network) releases the ids instead. There is no timer;
//! the next batch is sent only when an answer arrives.

use std::collections::HashSet;

use crate::calls::{Load, Loaded};
use crate::ids::UserId;
use crate::model::PersistOp;
use crate::model::{Message, ProviderError};
use crate::reduce::{Changed, Effect};
use crate::state::WorkspaceState;

/// Ids asked for per request.
pub const MAX_BATCH: usize = 50;
/// Failed ids remembered; past this the set is cleared (a bound, not a policy).
const MAX_FAILED: usize = 5000;

#[derive(Clone, Debug, Default)]
pub(crate) struct MemberBook {
    inflight: HashSet<UserId>,
    failed: HashSet<UserId>,
}

impl MemberBook {
    /// Lean mode and sign-out drop nothing here: the sets are already bounded.
    fn skip(&self, u: &UserId) -> bool {
        self.inflight.contains(u) || self.failed.contains(u)
    }
}

/// Collects the ids of `m` that are neither known nor already asked or failed,
/// so known members are never cloned.
fn authors(state: &WorkspaceState, m: &Message, out: &mut Vec<UserId>) {
    for u in m.author.iter().chain(m.reply_users.iter()) {
        if !state.members.contains_key(u) && !state.member_book.skip(u) {
            out.push(u.clone());
        }
    }
}

/// Peers of the open direct conversations and members of the open groups: the
/// sidebar names each row after them. Closed ones are not asked for (they are not listed).
fn peers(state: &WorkspaceState, out: &mut Vec<UserId>) {
    let mut ps: Vec<&UserId> = state
        .conversations
        .values()
        .filter(|c| c.is_open && c.is_member && !c.is_archived)
        .flat_map(|c| c.peer.iter().chain(c.member_ids.iter()))
        .filter(|p| !state.members.contains_key(*p) && !state.member_book.skip(p))
        .collect();
    ps.sort();
    ps.dedup();
    out.extend(ps.into_iter().cloned());
}

fn collect(state: &WorkspaceState, fx: &[Effect], all: bool) -> Vec<UserId> {
    let mut ids = Vec::new();
    if all {
        peers(state, &mut ids);
        let mut cs: Vec<_> = state.windows.keys().collect();
        cs.sort();
        for c in cs {
            state.windows[c]
                .messages()
                .iter()
                .for_each(|m| authors(state, m, &mut ids));
        }
        let mut ts: Vec<_> = state.threads.keys().collect();
        ts.sort();
        for t in ts {
            let th = &state.threads[t];
            th.root.iter().for_each(|m| authors(state, m, &mut ids));
            th.replies
                .messages()
                .iter()
                .for_each(|m| authors(state, m, &mut ids));
        }
        return ids;
    }
    for e in fx {
        match e {
            Effect::Changed(Changed::Conversations) => peers(state, &mut ids),
            Effect::Changed(Changed::Timeline(c)) => {
                if let Some(w) = state.windows.get(c) {
                    w.messages()
                        .iter()
                        .for_each(|m| authors(state, m, &mut ids));
                }
            }
            Effect::Changed(Changed::Thread(root)) => {
                if let Some(th) = state.threads.get(root) {
                    th.root.iter().for_each(|m| authors(state, m, &mut ids));
                    th.replies
                        .messages()
                        .iter()
                        .for_each(|m| authors(state, m, &mut ids));
                }
            }
            _ => {}
        }
    }
    ids
}

fn request(state: &mut WorkspaceState, ids: Vec<UserId>, fx: &mut Vec<Effect>) {
    let mut batch: Vec<UserId> = Vec::new();
    for u in ids {
        if batch.len() >= MAX_BATCH {
            break;
        }
        if state.members.contains_key(&u) || state.member_book.skip(&u) || batch.contains(&u) {
            continue;
        }
        batch.push(u);
    }
    if batch.is_empty() {
        return;
    }
    state.member_book.inflight.extend(batch.iter().cloned());
    fx.push(Effect::Load(Load::Members(batch)));
}

/// After a reduction: ask for the unknown authors of what just changed.
pub(crate) fn scan(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if !state.caps.member_lookup || state.lean {
        return;
    }
    let ids = collect(state, fx, false);
    request(state, ids, fx);
}

/// `Loaded::Members`: store what was found, remember what was not, and send
/// the next batch when more ids are waiting.
pub(crate) fn handle_loaded(
    state: &mut WorkspaceState,
    r: &Result<Loaded, (Load, ProviderError)>,
    fx: &mut Vec<Effect>,
) {
    match r {
        Ok(Loaded::Members { asked, found }) => {
            let mut added = Vec::new();
            for m in found {
                if !state.members.contains_key(&m.id) {
                    state.members.insert(m.id.clone(), m.clone());
                    added.push(m.clone());
                }
            }
            for u in asked {
                state.member_book.inflight.remove(u);
                if !found.iter().any(|m| m.id == *u) && !state.members.contains_key(u) {
                    fail(state, u.clone());
                }
            }
            if !added.is_empty() {
                fx.push(Effect::Persist(PersistOp::Members(added)));
                fx.push(Effect::Changed(Changed::Members));
            }
        }
        Err((Load::Members(ids), e)) => {
            let transient = matches!(
                e,
                ProviderError::RateLimited
                    | ProviderError::Network(_)
                    | ProviderError::SessionExpired
            );
            for u in ids {
                state.member_book.inflight.remove(u);
                if !transient {
                    fail(state, u.clone());
                }
            }
            // A transient failure is not retried here: the next change that
            // shows the id asks again. Nothing to re-send now.
            return;
        }
        _ => return,
    }
    if state.caps.member_lookup && !state.lean {
        let ids = collect(state, fx, true);
        request(state, ids, fx);
    }
}

fn fail(state: &mut WorkspaceState, u: UserId) {
    if state.member_book.failed.len() >= MAX_FAILED {
        state.member_book.failed.clear();
    }
    state.member_book.failed.insert(u);
}
