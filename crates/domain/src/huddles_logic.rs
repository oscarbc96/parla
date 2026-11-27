//! Huddle state and the ring machine. Pure: time arrives as inputs
//! (`Input::Clock`, `Input::RingTimeout`), never from a clock or a timer.

use std::time::{Duration, Instant};

use crate::calls::{Load, Loaded, ProviderCall};
use crate::huddle::{Huddle, HuddleId};
use crate::ids::{ConversationId, UserId};
use crate::model::ConversationKind;
use crate::reduce::{Changed, Effect};
use crate::state::WorkspaceState;

/// How long a ring lasts before it stops on its own. The live spike
/// has not measured it; this is the fallback value.
pub const RING_TIMEOUT: Duration = Duration::from_secs(45);

/// After a reconnect, rings are ignored for this long (seconds) unless the
/// huddle list is reloaded first.
pub(crate) const ARM_AFTER_SECS: i64 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ring {
    pub conversation: ConversationId,
    pub huddle: HuddleId,
    pub from: UserId,
    pub since: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingEnd {
    Answered,
    AnsweredElsewhere,
    Declined,
    Cancelled,
    Ended,
    TimedOut,
    Dnd,
}

/// Reconnect arming state for rings, grouped to keep `WorkspaceState` small.
#[derive(Clone, Debug)]
pub(crate) struct RingClock {
    /// Unix time after which rings are live again (set on reconnect).
    pub(crate) arm_at: Option<i64>,
    /// Huddles whose ring the user declined or that timed out, oldest first.
    /// A later frame for the same huddle must not ring again; bounded by
    /// [`DISMISSED_MAX`] and forgotten when the huddle ends.
    pub(crate) dismissed: Vec<HuddleId>,
}

/// Most dismissed huddles remembered per workspace.
const DISMISSED_MAX: usize = 16;

/// Most conversations whose live huddle state wins over a stale list answer.
const TOUCHED_MAX: usize = 64;

/// A `Load::Huddles` answer and the live changes that overtook it, grouped to
/// keep `WorkspaceState` small. The answer is a snapshot taken when the load
/// was issued, so a conversation a live `HuddleUpdated` touched since keeps
/// its live state instead of being dropped or overwritten by that snapshot.
#[derive(Clone, Debug, Default)]
pub(crate) struct HuddleLoad {
    /// Requests waiting for an answer.
    pub(crate) in_flight: u32,
    /// Conversations touched by live updates since the first request,
    /// oldest first; bounded by [`TOUCHED_MAX`].
    pub(crate) touched: Vec<ConversationId>,
}

fn touch(state: &mut WorkspaceState, c: &ConversationId) {
    if state.huddle_load.in_flight == 0 {
        return;
    }
    let t = &mut state.huddle_load.touched;
    if t.contains(c) {
        return;
    }
    if t.len() >= TOUCHED_MAX {
        t.remove(0);
    }
    t.push(c.clone());
}

fn dismiss(state: &mut WorkspaceState, id: HuddleId) {
    let d = &mut state.ring_clock.dismissed;
    if d.contains(&id) {
        return;
    }
    if d.len() >= DISMISSED_MAX {
        d.remove(0);
    }
    d.push(id);
}

impl WorkspaceState {
    /// How many dismissed huddles are remembered (tests).
    #[cfg(feature = "test-support")]
    pub fn dismissed_rings(&self) -> usize {
        self.ring_clock.dismissed.len()
    }
}

fn stop(state: &mut WorkspaceState, reason: RingEnd, fx: &mut Vec<Effect>) {
    if let Some(r) = state.ring.take() {
        if matches!(reason, RingEnd::Declined | RingEnd::TimedOut) {
            dismiss(state, r.huddle.clone());
        }
        fx.push(Effect::StopRing {
            conversation: r.conversation,
            reason,
        });
        fx.push(Effect::Changed(Changed::Ring));
    }
}

fn ringing_for(state: &WorkspaceState, c: &ConversationId) -> bool {
    state.ring.as_ref().is_some_and(|r| r.conversation == *c)
}

fn arm_if_due(state: &mut WorkspaceState) {
    if !state.ring_armed
        && state
            .ring_clock
            .arm_at
            .is_some_and(|at| state.now_unix >= at)
    {
        state.ring_armed = true;
        state.ring_clock.arm_at = None;
    }
}

pub(crate) fn clock(state: &mut WorkspaceState, unix: i64, mono: Instant) {
    state.now_unix = unix;
    state.now_mono = Some(mono);
    arm_if_due(state);
}

pub(crate) fn connected(state: &mut WorkspaceState, reconnect: bool, fx: &mut Vec<Effect>) {
    if reconnect {
        state.ring_armed = false;
        state.ring_clock.arm_at = Some(state.now_unix.saturating_add(ARM_AFTER_SECS));
    }
    if state.caps.huddles {
        state.huddle_load.in_flight = state.huddle_load.in_flight.saturating_add(1);
        fx.push(Effect::Load(Load::Huddles));
    }
}

/// Make `in_huddle` follow a huddle's participants: add the new ones,
/// remove those that left (only users this huddle listed before).
fn sync_participants(
    state: &mut WorkspaceState,
    old: Option<&Huddle>,
    new: Option<&Huddle>,
) -> bool {
    let mut changed = false;
    if let Some(o) = old {
        for u in o.participants.iter() {
            if !new.is_some_and(|n| n.participants.contains(u)) {
                changed |= state.in_huddle.remove(u);
            }
        }
    }
    if let Some(n) = new {
        for u in n.participants.iter() {
            changed |= state.in_huddle.insert(u.clone());
        }
    }
    changed
}

fn upsert(state: &mut WorkspaceState, h: Huddle) -> bool {
    let old = state.huddles.insert(h.conversation.clone(), h.clone());
    let changed = old.as_ref() != Some(&h);
    sync_participants(state, old.as_ref(), Some(&h)) | changed
}

pub(crate) fn updated(state: &mut WorkspaceState, h: Huddle, fx: &mut Vec<Effect>) {
    let c = h.conversation.clone();
    touch(state, &c);
    let me_in = state
        .self_user
        .as_ref()
        .is_some_and(|me| h.participants.contains(me));
    if upsert(state, h) {
        fx.push(Effect::Changed(Changed::Huddles));
    }
    if me_in && ringing_for(state, &c) {
        stop(state, RingEnd::AnsweredElsewhere, fx);
    }
}

pub(crate) fn ended(
    state: &mut WorkspaceState,
    c: &ConversationId,
    id: &HuddleId,
    fx: &mut Vec<Effect>,
) {
    if state.huddles.get(c).is_some_and(|h| h.id == *id)
        && let Some(old) = state.huddles.remove(c)
    {
        sync_participants(state, Some(&old), None);
        fx.push(Effect::Changed(Changed::Huddles));
    }
    state.ring_clock.dismissed.retain(|d| d != id);
    if state.ring.as_ref().is_some_and(|r| r.huddle == *id) {
        stop(state, RingEnd::Ended, fx);
    }
}

pub(crate) fn ring(
    state: &mut WorkspaceState,
    conversation: ConversationId,
    huddle: HuddleId,
    from: UserId,
    fx: &mut Vec<Effect>,
) {
    arm_if_due(state);
    let Some(me) = state.self_user.clone() else {
        return;
    };
    let direct = state
        .conversations
        .get(&conversation)
        .is_some_and(|c| matches!(c.kind, ConversationKind::Direct | ConversationKind::Group));
    let self_dnd = state.dnd.get(&me).is_some_and(|d| d.active(state.now_unix));
    let in_it = state.in_huddle.contains(&me)
        || state
            .huddles
            .get(&conversation)
            .is_some_and(|h| h.participants.contains(&me));
    if !state.caps.huddles
        || !direct
        || state.ring_clock.dismissed.contains(&huddle)
        || !state.ring_enabled
        || !state.ring_armed
        || self_dnd
        || in_it
        || state.ring.is_some()
        || from == me
    {
        return;
    }
    let Some(since) = state.now_mono else {
        return;
    };
    let r = Ring {
        conversation,
        huddle,
        from,
        since,
    };
    state.ring = Some(r.clone());
    fx.push(Effect::StartRing(r));
    fx.push(Effect::Changed(Changed::Ring));
}

pub(crate) fn cancelled(state: &mut WorkspaceState, c: &ConversationId, fx: &mut Vec<Effect>) {
    if ringing_for(state, c) {
        stop(state, RingEnd::Cancelled, fx);
    }
}

pub(crate) fn in_huddle(
    state: &mut WorkspaceState,
    user: UserId,
    active: bool,
    fx: &mut Vec<Effect>,
) {
    let changed = if active {
        state.in_huddle.insert(user)
    } else {
        state.in_huddle.remove(&user)
    };
    if changed {
        fx.push(Effect::Changed(Changed::Huddles));
    }
}

pub(crate) fn decline(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if let Some(r) = state.ring.clone() {
        stop(state, RingEnd::Declined, fx);
        fx.push(Effect::Call(ProviderCall::DeclineHuddle {
            conversation: r.conversation,
            id: r.huddle,
        }));
    }
}

pub(crate) fn timeout(state: &mut WorkspaceState, now: Instant, fx: &mut Vec<Effect>) {
    if state
        .ring
        .as_ref()
        .is_some_and(|r| now.saturating_duration_since(r.since) >= RING_TIMEOUT)
    {
        stop(state, RingEnd::TimedOut, fx);
    }
}

pub(crate) fn join(state: &mut WorkspaceState, c: ConversationId, fx: &mut Vec<Effect>) {
    let ringing = ringing_for(state, &c);
    if !ringing && !state.huddles.contains_key(&c) {
        return;
    }
    if ringing {
        stop(state, RingEnd::Answered, fx);
    }
    fx.push(Effect::OpenHuddle { conversation: c });
}

/// Run after every input: a DND that starts ends a ring.
pub(crate) fn dnd_check(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if state.ring.is_none() {
        return;
    }
    let active = state
        .self_user
        .as_ref()
        .and_then(|me| state.dnd.get(me))
        .is_some_and(|d| d.active(state.now_unix));
    if active {
        stop(state, RingEnd::Dnd, fx);
    }
}

pub(crate) fn handle_loaded(
    state: &mut WorkspaceState,
    r: &Result<Loaded, (Load, crate::model::ProviderError)>,
    fx: &mut Vec<Effect>,
) {
    let Ok(Loaded::Huddles(all)) = r else {
        // A failed huddle load is still an answer: stop waiting for it.
        if matches!(r, Err((Load::Huddles, _))) {
            state.huddle_load.in_flight = state.huddle_load.in_flight.saturating_sub(1);
            if state.huddle_load.in_flight == 0 {
                state.huddle_load.touched.clear();
            }
        }
        return;
    };
    state.ring_armed = true;
    state.ring_clock.arm_at = None;
    // A full list is the truth for the conversations no live update overtook
    // since the load was issued; for those, the later live state wins.
    let touched = if state.huddle_load.in_flight > 1 {
        state.huddle_load.touched.clone()
    } else {
        std::mem::take(&mut state.huddle_load.touched)
    };
    state.huddle_load.in_flight = state.huddle_load.in_flight.saturating_sub(1);
    let stale = |h: &Huddle| touched.contains(&h.conversation);
    // A huddle missing from the list ended while the connection was down,
    // so drop it (and the participants it listed).
    let gone: Vec<Huddle> = state
        .huddles
        .values()
        .filter(|h| {
            !stale(h)
                && !all
                    .iter()
                    .any(|n| n.conversation == h.conversation && n.id == h.id)
        })
        .cloned()
        .collect();
    let mut changed = false;
    for h in gone {
        state.huddles.remove(&h.conversation);
        sync_participants(state, Some(&h), None);
        state.ring_clock.dismissed.retain(|d| d != &h.id);
        changed = true;
    }
    for h in all {
        if stale(h) {
            continue;
        }
        changed |= upsert(state, h.clone());
    }
    if changed {
        fx.push(Effect::Changed(Changed::Huddles));
    }
    // A ring for a huddle the fresh list does not know has ended.
    let gone = state.ring.as_ref().is_some_and(|r| {
        !touched.contains(&r.conversation)
            && !all
                .iter()
                .any(|h| h.conversation == r.conversation && h.id == r.huddle)
    });
    if gone {
        stop(state, RingEnd::Ended, fx);
    }
}

pub(crate) fn set_ring_enabled(state: &mut WorkspaceState, on: bool, fx: &mut Vec<Effect>) {
    state.ring_enabled = on;
    if !on {
        stop(state, RingEnd::Cancelled, fx);
    }
}
