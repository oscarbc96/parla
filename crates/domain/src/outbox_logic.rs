//! Ordered, at-most-once outbox for offline sends. Pure: the reducer calls
//! these helpers and the session executes the resulting `Effect::Outbox`.

use std::sync::Arc;

use crate::ids::{ClientMsgId, MessageId, MessageTs};
use crate::model::{DeliveryState, Message, ProviderError};
use crate::outbox::{OutboxItem, OutboxState};
use crate::reduce::{Changed, Effect};
use crate::state::{Connection, WorkspaceState};
use crate::threads;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboxOp {
    Put(OutboxItem),
    Remove(ClientMsgId),
}

/// Rate-limited sends resent at once before the outbox falls back to its timer.
const RATE_LIMIT_RESENDS: u32 = 3;

/// Sends typed while online go out at once, up to this many in flight (the
/// provider's own per-conversation burst); a backlog goes one at a time.
pub const MAX_IN_FLIGHT: usize = 5;

/// The reason of an item that was being sent when the app quit.
pub const RESTART_REASON: &str = "may have been sent; check before retrying";

/// Milliseconds since the Unix epoch to a `secs.micros` `MessageTs`.
fn ms_ts(ms: i64) -> MessageTs {
    MessageTs(Arc::from(format!(
        "{}.{:06}",
        ms / 1000,
        (ms % 1000) * 1000
    )))
}

fn pending_message(state: &WorkspaceState, i: &OutboxItem, ts: MessageTs) -> Message {
    let mut m = Message::new(
        MessageId {
            conversation: i.conversation.clone(),
            ts,
        },
        state.self_user.clone(),
        i.text.clone(),
    );
    m.thread_root = i.thread.clone();
    m.broadcast = i.broadcast;
    m.client_msg_id = Some(i.id.clone());
    m.state = match &i.state {
        OutboxState::Failed(e) => DeliveryState::Failed(e.clone()),
        _ => DeliveryState::Pending,
    };
    m
}

/// Insert keeping `created_ms` order; ties keep arrival order.
fn insert_item(state: &mut WorkspaceState, item: OutboxItem) {
    let pos = state
        .outbox
        .partition_point(|x| x.created_ms <= item.created_ms);
    state.outbox.insert(pos, item);
}

fn set_message_state(state: &mut WorkspaceState, i: &OutboxItem, ds: DeliveryState) {
    threads::set_state(state, &i.conversation, i.thread.as_ref(), &i.id, ds);
}

pub(crate) fn enqueue(
    state: &mut WorkspaceState,
    mut item: OutboxItem,
    now: MessageTs,
    now_ms: i64,
    fx: &mut Vec<Effect>,
) {
    // The store orders the outbox by `created_ms` alone (ids are random): a
    // send that ties with a queued one goes after it.
    let id = item.id.clone();
    let mut ms = now_ms;
    while state.outbox.iter().any(|i| i.created_ms == ms) {
        ms += 1;
    }
    item.created_ms = ms;
    let pending = pending_message(state, &item, now);
    let conv = item.conversation.clone();
    threads::place(state, pending);
    fx.push(Effect::Outbox(OutboxOp::Put(item.clone())));
    insert_item(state, item);
    fx.push(Effect::Changed(Changed::Timeline(conv)));
    // A send typed while online goes at once, without waiting for the round
    // trip of the sends before it, unless a backlog is queued ahead of it.
    let first_queued = state
        .outbox
        .iter()
        .position(|i| i.state == OutboxState::Queued);
    let pos = state.outbox.iter().position(|i| i.id == id);
    if state.connection == Connection::Online
        && state.inflight.len() < MAX_IN_FLIGHT
        && first_queued.is_some()
        && first_queued == pos
    {
        if let Some(pos) = pos {
            start(state, pos, fx);
        }
        return;
    }
    flush(state, fx);
}

/// Starts the oldest `Queued` item when online and nothing is in flight, so a
/// backlog (queued offline, reloaded, retried) goes one at a time, in order.
/// While offline, queued items wait for the provider's own backoff reconnect:
/// hurrying it after every drop turned a server that hangs up at once into a
/// reconnect storm. Only the user's "Retry now" skips that wait.
pub(crate) fn flush(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if state.connection != Connection::Online {
        return;
    }
    if !state.inflight.is_empty() {
        return;
    }
    if let Some(pos) = state
        .outbox
        .iter()
        .position(|i| i.state == OutboxState::Queued)
    {
        start(state, pos, fx);
    }
}

/// Hands the item at `pos` to the provider.
fn start(state: &mut WorkspaceState, pos: usize, fx: &mut Vec<Effect>) {
    let i = &mut state.outbox[pos];
    i.state = OutboxState::Sending;
    let i = i.clone();
    state.inflight.push(i.id.clone());
    fx.push(Effect::Outbox(OutboxOp::Put(i.clone())));
    match i.thread {
        Some(root) => fx.push(Effect::CallReply {
            root: MessageId {
                conversation: i.conversation,
                ts: root,
            },
            text: i.text,
            broadcast: i.broadcast,
            id: i.id,
        }),
        None => fx.push(Effect::CallSend {
            conversation: i.conversation,
            text: i.text,
            id: i.id,
        }),
    }
}

/// Remembers that a send with this message id settled (bounded).
pub(crate) fn note_settled(state: &mut WorkspaceState, id: &MessageId) {
    const KEEP: usize = 8;
    if state.settled.contains(id) {
        return;
    }
    if state.settled.len() == KEEP {
        state.settled.pop_front();
    }
    state.settled.push_back(id.clone());
}

/// The client id of the oldest unsettled send that `m` is the echo of: the
/// signed-in user's own message with the same conversation, thread and text,
/// for a server that does not hand our client id back. Without it the pending
/// row and its echo would sit side by side until the send settles.
pub(crate) fn echo_of(state: &WorkspaceState, m: &Message) -> Option<ClientMsgId> {
    if m.author.is_none() || m.author != state.self_user || state.settled.contains(&m.id) {
        return None;
    }
    // A confirmed copy is already held: this is the late echo of a send that
    // has settled, never of the one in flight now.
    let held = match &m.thread_root {
        Some(r) => state
            .threads
            .get(&threads::root_id(&m.id.conversation, r))
            .is_some_and(|t| t.replies.has_delivered(&m.id)),
        None => state
            .windows
            .get(&m.id.conversation)
            .is_some_and(|w| w.has_delivered(&m.id)),
    };
    if held {
        return None;
    }
    state
        .outbox
        .iter()
        .find(|i| {
            i.conversation == m.id.conversation
                && i.thread == m.thread_root
                && i.text == m.text
                && matches!(i.state, OutboxState::Sending)
        })
        .map(|i| i.id.clone())
}

pub(crate) fn on_echo(state: &mut WorkspaceState, id: &ClientMsgId, fx: &mut Vec<Effect>) {
    if let Some(pos) = state.outbox.iter().position(|i| &i.id == id) {
        state.outbox.remove(pos);
        fx.push(Effect::Outbox(OutboxOp::Remove(id.clone())));
    }
    // The echo proves the server took this send, so its slot is free: a
    // backlog goes on without waiting for the (possibly slow) HTTP answer.
    if let Some(pos) = state.inflight.iter().position(|i| i == id) {
        state.inflight.remove(pos);
        flush(state, fx);
    }
}

/// Settles a finished send. After a success or a hard failure the next
/// item goes out, even when this item is already gone; after a network
/// failure we wait for `Connected`, or for a one-shot retry timer when the
/// connection is still up, so we do not spin. A rate-limited send goes again
/// at once: the provider waits out the server's hold before it answers and
/// before its next call, so the outbox adds no delay of its own (only after
/// a few rate limits in a row does the timer take over).
pub(crate) fn finished(
    state: &mut WorkspaceState,
    id: &ClientMsgId,
    result: &Result<Message, ProviderError>,
    fx: &mut Vec<Effect>,
) {
    state.inflight.retain(|i| i != id);
    let requeue = matches!(
        result,
        Err(ProviderError::Network(_) | ProviderError::RateLimited)
    );
    let mut resend = false;
    match result {
        Ok(m) => {
            note_settled(state, &m.id);
            on_echo(state, id, fx)
        }
        Err(e) => {
            if let Some(pos) = state.outbox.iter().position(|i| &i.id == id) {
                let it = &mut state.outbox[pos];
                if requeue {
                    it.state = OutboxState::Queued;
                    it.attempts += 1;
                    let limited = matches!(result, Err(ProviderError::RateLimited));
                    if limited && it.attempts <= RATE_LIMIT_RESENDS {
                        resend = true;
                    } else if state.connection == Connection::Online {
                        fx.push(Effect::RetryOutbox {
                            after_secs: it.attempts.saturating_mul(5).min(60),
                        });
                    }
                } else {
                    it.state = OutboxState::Failed(e.to_string());
                }
                let it = it.clone();
                if let OutboxState::Failed(r) = &it.state {
                    set_message_state(state, &it, DeliveryState::Failed(r.clone()));
                }
                fx.push(Effect::Outbox(OutboxOp::Put(it.clone())));
                fx.push(Effect::Changed(Changed::Timeline(it.conversation)));
            }
        }
    }
    if resend
        && state.connection == Connection::Online
        && let Some(pos) = state.outbox.iter().position(|i| &i.id == id)
    {
        // The rate-limited send goes again at once, ahead of any backlog.
        start(state, pos, fx);
    } else if !requeue || resend {
        flush(state, fx);
    }
}

pub(crate) fn loaded(state: &mut WorkspaceState, items: Vec<OutboxItem>, fx: &mut Vec<Effect>) {
    state.outbox.clear();
    state.inflight.clear();
    for mut i in items {
        if i.state == OutboxState::Sending {
            i.state = OutboxState::Failed(RESTART_REASON.to_string());
            fx.push(Effect::Outbox(OutboxOp::Put(i.clone())));
        }
        let m = pending_message(state, &i, ms_ts(i.created_ms));
        let conv = i.conversation.clone();
        let held = state
            .windows
            .get(&conv)
            .is_some_and(|w| w.find_client(&i.id).is_some())
            || i.thread.as_ref().is_some_and(|r| {
                state
                    .threads
                    .get(&threads::root_id(&conv, r))
                    .is_some_and(|t| t.replies.find_client(&i.id).is_some())
            });
        if !held {
            threads::place(state, m);
        }
        fx.push(Effect::Changed(Changed::Timeline(conv)));
        insert_item(state, i);
    }
    flush(state, fx);
}

pub(crate) fn retry(state: &mut WorkspaceState, id: &ClientMsgId, fx: &mut Vec<Effect>) {
    let Some(i) = state.outbox.iter_mut().find(|i| &i.id == id) else {
        return;
    };
    if !matches!(i.state, OutboxState::Failed(_)) {
        return;
    }
    i.state = OutboxState::Queued;
    let i = i.clone();
    set_message_state(state, &i, DeliveryState::Pending);
    fx.push(Effect::Outbox(OutboxOp::Put(i.clone())));
    fx.push(Effect::Changed(Changed::Timeline(i.conversation)));
    flush(state, fx);
}

pub(crate) fn discard(state: &mut WorkspaceState, id: &ClientMsgId, fx: &mut Vec<Effect>) {
    let Some(pos) = state.outbox.iter().position(|i| &i.id == id) else {
        return;
    };
    let i = state.outbox.remove(pos);
    threads::remove_client(state, &i.conversation, i.thread.as_ref(), id);
    fx.push(Effect::Outbox(OutboxOp::Remove(id.clone())));
    fx.push(Effect::Changed(Changed::Timeline(i.conversation)));
}
