//! Do-not-disturb and per-conversation notification levels: the optimistic
//! writes behind [`crate::reduce::Input`], the state updates for provider
//! answers, and the structured "until …" end.

use std::collections::HashMap;

use crate::calls::{CallOutput, Load, Loaded, ProviderCall};
use crate::flight::{Flight, settle};
use crate::ids::ConversationId;
use crate::model::ProviderError;
use crate::people::{ConversationPrefs, DndStatus, NotifyExtras};
use crate::reduce::{Changed, Effect};
use crate::state::WorkspaceState;

/// In-flight optimistic writes, per key.
#[derive(Clone, Debug, Default)]
pub(crate) struct NotifyBook {
    dnd: Option<Flight>,
    prefs: HashMap<ConversationId, Flight>,
}

/// Pause notifications for `minutes` (self DND), optimistic. `now_unix`
/// is injected: the reducer stays pure. Without a known self user there is
/// nothing to write, so no call is sent.
pub(crate) fn snooze(
    state: &mut WorkspaceState,
    minutes: u32,
    now_unix: i64,
    fx: &mut Vec<Effect>,
) {
    let Some(me) = state.self_user.clone() else {
        return;
    };
    state.notify.dnd.get_or_insert_with(Flight::default).begin();
    let until = now_unix.saturating_add(60 * minutes as i64);
    let mut d = state.dnd.get(&me).cloned().unwrap_or_default();
    if d.snooze_until != Some(until) {
        d.snooze_until = Some(until);
        state.dnd.insert(me, d);
        fx.push(Effect::Changed(Changed::Dnd));
    }
    fx.push(Effect::Call(ProviderCall::Snooze(minutes)));
}

/// End a snooze, optimistic. Nothing is sent without a self user.
pub(crate) fn end_snooze(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    let Some(me) = state.self_user.clone() else {
        return;
    };
    state.notify.dnd.get_or_insert_with(Flight::default).begin();
    if let Some(d) = state.dnd.get_mut(&me)
        && d.snooze_until.take().is_some()
    {
        fx.push(Effect::Changed(Changed::Dnd));
    }
    fx.push(Effect::Call(ProviderCall::EndSnooze));
}

/// Set a conversation's notification level, optimistic.
pub(crate) fn set_prefs(
    state: &mut WorkspaceState,
    conversation: ConversationId,
    prefs: ConversationPrefs,
    fx: &mut Vec<Effect>,
) {
    state
        .notify
        .prefs
        .entry(conversation.clone())
        .or_default()
        .begin();
    if state.notify_prefs.get(&conversation) != Some(&prefs) {
        state.notify_prefs.insert(conversation.clone(), prefs);
        fx.push(Effect::Changed(Changed::NotifyPrefs));
    }
    fx.push(Effect::Call(ProviderCall::SetNotifyPrefs {
        conversation,
        prefs,
    }));
}

/// Set a conversation's level in Parla itself: no provider call, persisted.
/// The default (level `Default`, not muted) removes the entry.
pub(crate) fn set_local_prefs(
    state: &mut WorkspaceState,
    conversation: ConversationId,
    prefs: ConversationPrefs,
    fx: &mut Vec<Effect>,
) {
    let before = state.local_notify_prefs.get(&conversation).copied();
    let after = (prefs != ConversationPrefs::default()).then_some(prefs);
    if before == after {
        return;
    }
    match after {
        Some(p) => state.local_notify_prefs.insert(conversation.clone(), p),
        None => state.local_notify_prefs.remove(&conversation),
    };
    fx.push(Effect::Changed(Changed::NotifyPrefs));
    fx.push(Effect::Persist(crate::PersistOp::LocalNotifyPrefsPut {
        conversation,
        prefs,
    }));
}

/// Set a conversation's Parla-side extras, persisted. The default removes the
/// entry.
pub(crate) fn set_extras(
    state: &mut WorkspaceState,
    conversation: ConversationId,
    extras: NotifyExtras,
    fx: &mut Vec<Effect>,
) {
    let before = state.local_notify_extras.get(&conversation).copied();
    let after = (extras != NotifyExtras::default()).then_some(extras);
    if before == after {
        return;
    }
    match after {
        Some(e) => state.local_notify_extras.insert(conversation.clone(), e),
        None => state.local_notify_extras.remove(&conversation),
    };
    fx.push(Effect::Changed(Changed::NotifyPrefs));
    fx.push(Effect::Persist(crate::PersistOp::LocalNotifyExtrasPut {
        conversation,
        extras,
    }));
}

/// The unix time of the earliest timed mute; the shell arms one timer for it.
pub fn next_mute_deadline(state: &WorkspaceState) -> Option<i64> {
    state
        .local_notify_extras
        .values()
        .filter_map(|e| e.mute_until)
        .min()
}

/// Ends every timed mute due at `now`; the rest of an entry is kept.
pub(crate) fn mute_tick(state: &mut WorkspaceState, now: i64, fx: &mut Vec<Effect>) {
    let due: Vec<ConversationId> = state
        .local_notify_extras
        .iter()
        .filter(|(_, e)| e.mute_until.is_some_and(|u| u <= now))
        .map(|(c, _)| c.clone())
        .collect();
    for c in due {
        let extras = NotifyExtras {
            mute_until: None,
            ..state.local_notify_extras[&c]
        };
        set_extras(state, c, extras, fx);
    }
}

/// The answers of `ProviderCall::{Snooze, EndSnooze, SetNotifyPrefs}`, in
/// any order. Each answer settles one in-flight write; a lone write's
/// `Dnd` answer is kept, overlapping writes keep the optimistic state
/// (the latest write wins). When the last answer is in and any failed, the
/// truth is refetched instead of rolling back stored priors.
pub(crate) fn call_finished(
    state: &mut WorkspaceState,
    call: &ProviderCall,
    result: &Result<CallOutput, ProviderError>,
    fx: &mut Vec<Effect>,
) {
    match call {
        ProviderCall::Snooze(_) | ProviderCall::EndSnooze => {
            let st = settle(&mut state.notify.dnd, result.is_err());
            if st.refetch {
                fx.push(Effect::Load(Load::Dnd));
                return;
            }
            match result {
                Ok(CallOutput::Dnd(s)) if st.authoritative => {
                    if let Some(me) = state.self_user.clone()
                        && state.dnd.get(&me) != Some(s)
                    {
                        state.dnd.insert(me, s.clone());
                        fx.push(Effect::Changed(Changed::Dnd));
                    }
                }
                // No authoritative value: read the current one.
                Ok(CallOutput::None) if st.idle => fx.push(Effect::Load(Load::Dnd)),
                _ => {}
            }
        }
        ProviderCall::SetNotifyPrefs { conversation, .. } => {
            let mut slot = state.notify.prefs.remove(conversation);
            let st = settle(&mut slot, result.is_err());
            if let Some(f) = slot {
                state.notify.prefs.insert(conversation.clone(), f);
            }
            if st.refetch {
                fx.push(Effect::Load(Load::NotifyPrefs));
            }
        }
        _ => {}
    }
}

/// `Loaded(Dnd | DndUsers | NotifyPrefs)` and the provider's live events
/// put the current values into the state, notifying only on a real change.
pub(crate) fn handle_loaded(
    state: &mut WorkspaceState,
    r: &Result<Loaded, (Load, ProviderError)>,
    fx: &mut Vec<Effect>,
) {
    let Ok(l) = r else { return };
    match l {
        // While a self write is in flight the optimistic value stands.
        Loaded::Dnd(s) => {
            if state.notify.dnd.is_none()
                && let Some(me) = state.self_user.clone()
                && state.dnd.get(&me) != Some(s)
            {
                state.dnd.insert(me, s.clone());
                fx.push(Effect::Changed(Changed::Dnd));
            }
        }
        Loaded::DndUsers(all) => {
            let changed = all
                .iter()
                .any(|(user, status)| state.dnd.get(user) != Some(status));
            if changed {
                state.dnd.extend(all.clone());
                fx.push(Effect::Changed(Changed::Dnd));
            }
        }
        Loaded::NotifyPrefs(all) => {
            // A conversation with a write in flight keeps its optimistic
            // value: the snapshot may predate the write.
            let mut next = all.clone();
            for c in state.notify.prefs.keys() {
                match state.notify_prefs.get(c) {
                    Some(p) => next.insert(c.clone(), *p),
                    None => next.remove(c),
                };
            }
            if state.notify_prefs != next {
                state.notify_prefs = next;
                fx.push(Effect::Changed(Changed::NotifyPrefs));
            }
        }
        _ => {}
    }
}

/// The day a DND end falls on, relative to now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DndDay {
    Today,
    Tomorrow,
    On(jiff::civil::Date),
}

/// When the user's DND ends: the instant in `tz`, and the day it falls on.
/// An empty DND state, a future window or `enabled = false` are not active
/// now, so there is nothing to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DndEnds {
    pub at: jiff::ZonedDateTime,
    pub day: DndDay,
}

/// The end of an active DND: a snooze, an active schedule window, or the
/// later of the two when both apply. `None` when DND is not active now.
pub fn dnd_ends(s: &DndStatus, now: i64, tz: &jiff::tz::TimeZone) -> Option<DndEnds> {
    if !s.active(now) {
        return None;
    }
    let snooze = s.snooze_until.filter(|&until| until > now);
    let window = match (s.enabled, s.next_start, s.next_end) {
        (true, Some(start), Some(end)) if start <= now && now < end => Some(end),
        _ => None,
    };
    let end = [snooze, window].into_iter().flatten().max()?;
    let at = jiff::Timestamp::from_second(end)
        .ok()?
        .to_zoned_datetime(tz.clone());
    let today = jiff::Timestamp::from_second(now)
        .ok()?
        .to_zoned_datetime(tz.clone())
        .date();
    let date = at.date();
    let day = if date == today {
        DndDay::Today
    } else if date == today.tomorrow().ok()? {
        DndDay::Tomorrow
    } else {
        DndDay::On(date)
    };
    Some(DndEnds { at, day })
}
