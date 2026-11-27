//! Who is typing where, and the throttle on telling the provider we type.
//! Pure: time arrives as inputs (`Input::Clock` before each provider batch,
//! `Input::TypingExpired`, `Input::Typed`), never from a clock or a timer.
//!
//! A typer shows for [`TYPING_TTL`] after their last event. Expired typers
//! are dropped on the next typing event, on the one expiry timer the shell
//! arms (`Effect::TypingWakeAt`, replaced on every change) and are filtered
//! out on render ([`Typer::active`]). Everything is bounded: at most
//! [`TYPERS_MAX`] typers per place and [`TYPING_PLACES_MAX`] places.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::ids::{ConversationId, MessageTs, UserId};
use crate::reduce::{Changed, Effect};
use crate::state::{Connection, WorkspaceState};

/// How long a typer stays shown after their last typing event. The web
/// client repeats its typing frame every few seconds while a person types.
pub const TYPING_TTL: Duration = Duration::from_secs(6);

/// At most one outbound typing signal per conversation in this time.
pub const TYPING_SEND_EVERY: Duration = Duration::from_secs(3);

/// Most typers kept per conversation or thread; the oldest is dropped.
pub const TYPERS_MAX: usize = 8;

/// Most conversations and threads with typers kept at once.
pub const TYPING_PLACES_MAX: usize = 32;

/// One person typing in a conversation or thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typer {
    pub user: UserId,
    /// When their current run of typing events started.
    pub since: Instant,
    /// Their latest typing event.
    pub last: Instant,
}

impl Typer {
    /// When this typer stops showing without a new event.
    pub fn expires(&self) -> Instant {
        self.last + TYPING_TTL
    }

    /// Still shown at `now`.
    pub fn active(&self, now: Instant) -> bool {
        now < self.expires()
    }
}

type Place = (ConversationId, Option<MessageTs>);

/// The typing state of one workspace.
#[derive(Clone, Debug, Default)]
pub(crate) struct TypingBook {
    /// Typers per place, oldest `since` first.
    places: HashMap<Place, Vec<Typer>>,
    /// The expiry timer last asked of the shell.
    wake: Option<Instant>,
    /// When we last told the provider we type, per conversation (only those
    /// within [`TYPING_SEND_EVERY`] are kept).
    sent: HashMap<ConversationId, Instant>,
}

impl WorkspaceState {
    /// Who is typing in `conversation` (its thread `thread` when set), oldest
    /// first, never self. Filter with [`Typer::active`] for the current time.
    pub fn typers(&self, conversation: &ConversationId, thread: Option<&MessageTs>) -> &[Typer] {
        if self.typing.places.is_empty() {
            return &[];
        }
        self.typing
            .places
            .get(&(conversation.clone(), thread.cloned()))
            .map_or(&[], Vec::as_slice)
    }
}

impl WorkspaceState {
    /// Whether `Input::Typed` for `conversation` at `now` would tell the
    /// provider: the shell skips the input (and its work) on every other
    /// keystroke.
    pub fn typing_due(&self, conversation: &ConversationId, now: Instant) -> bool {
        self.caps.typing
            && !self.lean
            && self.connection == Connection::Online
            && self
                .typing
                .sent
                .get(conversation)
                .is_none_or(|at| now.saturating_duration_since(*at) >= TYPING_SEND_EVERY)
    }
}

/// A `ProviderEvent::Typing` at `state.now_mono` (sent before each batch).
pub(crate) fn event(
    state: &mut WorkspaceState,
    conversation: ConversationId,
    user: UserId,
    thread: Option<MessageTs>,
    fx: &mut Vec<Effect>,
) {
    let Some(now) = state.now_mono else { return };
    if state.lean || state.self_user.as_ref() == Some(&user) {
        return;
    }
    prune(state, now, fx);
    let book = &mut state.typing;
    let place = (conversation.clone(), thread);
    if !book.places.contains_key(&place) && book.places.len() >= TYPING_PLACES_MAX {
        // Drop the place whose newest event is oldest.
        let oldest = book
            .places
            .iter()
            .min_by_key(|(_, v)| v.iter().map(|t| t.last).max())
            .map(|(k, _)| k.clone());
        if let Some(k) = oldest {
            book.places.remove(&k);
            fx.push(Effect::Changed(Changed::Typing(k.0)));
        }
    }
    let typers = book.places.entry(place).or_default();
    if let Some(t) = typers.iter_mut().find(|t| t.user == user) {
        t.last = now;
    } else {
        if typers.len() >= TYPERS_MAX {
            typers.remove(0);
        }
        typers.push(Typer {
            user,
            since: now,
            last: now,
        });
        fx.push(Effect::Changed(Changed::Typing(conversation)));
    }
    rearm(state, fx);
}

/// The expiry timer fired.
pub(crate) fn expired(state: &mut WorkspaceState, now: Instant, fx: &mut Vec<Effect>) {
    state.typing.wake = None;
    prune(state, now, fx);
    rearm(state, fx);
}

/// A message from `user` arrived in `conversation`: they stopped typing there.
pub(crate) fn posted(
    state: &mut WorkspaceState,
    conversation: &ConversationId,
    user: &UserId,
    fx: &mut Vec<Effect>,
) {
    let book = &mut state.typing;
    if book.places.is_empty() {
        return;
    }
    let mut hit = false;
    book.places.retain(|(c, _), v| {
        if c == conversation {
            let before = v.len();
            v.retain(|t| &t.user != user);
            hit |= v.len() != before;
        }
        !v.is_empty()
    });
    if hit {
        fx.push(Effect::Changed(Changed::Typing(conversation.clone())));
        rearm(state, fx);
    }
}

/// Disconnected or lean: nobody is shown typing.
pub(crate) fn clear(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    let book = &mut state.typing;
    book.sent.clear();
    if book.places.is_empty() {
        return;
    }
    let mut gone: Vec<ConversationId> = book.places.drain().map(|((c, _), _)| c).collect();
    gone.sort();
    gone.dedup();
    for c in gone {
        fx.push(Effect::Changed(Changed::Typing(c)));
    }
    rearm(state, fx);
}

/// The user typed in a draft of `conversation` (or its thread `thread`): tell
/// the provider, at most once per [`TYPING_SEND_EVERY`] per conversation.
pub(crate) fn typed(
    state: &mut WorkspaceState,
    conversation: ConversationId,
    thread: Option<MessageTs>,
    now: Instant,
    fx: &mut Vec<Effect>,
) {
    if !state.caps.typing || state.lean || state.connection != Connection::Online {
        return;
    }
    let sent = &mut state.typing.sent;
    sent.retain(|_, at| now.saturating_duration_since(*at) < TYPING_SEND_EVERY);
    if sent.contains_key(&conversation) {
        return;
    }
    sent.insert(conversation.clone(), now);
    fx.push(Effect::SendTyping {
        conversation,
        thread,
    });
}

/// Drops the typers expired at `now`, signalling their conversations.
fn prune(state: &mut WorkspaceState, now: Instant, fx: &mut Vec<Effect>) {
    let book = &mut state.typing;
    let mut gone: Vec<ConversationId> = Vec::new();
    book.places.retain(|(c, _), v| {
        let before = v.len();
        v.retain(|t| t.active(now));
        if v.len() != before && !gone.contains(c) {
            gone.push(c.clone());
        }
        !v.is_empty()
    });
    for c in gone {
        fx.push(Effect::Changed(Changed::Typing(c)));
    }
}

/// Asks the shell for one timer at the earliest expiry, when it moved.
fn rearm(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    let book = &mut state.typing;
    let next = book
        .places
        .values()
        .flat_map(|v| v.iter().map(Typer::expires))
        .min();
    if next != book.wake {
        book.wake = next;
        fx.push(Effect::TypingWakeAt(next));
    }
}
