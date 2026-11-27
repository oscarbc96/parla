//! Reminder reducer rules: add, complete, delete and load.
//!
//! Complete and delete apply locally first and roll back when the provider
//! refuses, so the list never waits on a round trip.

use std::sync::Arc;

use crate::calls::{CallOutput, Load, Loaded, ProviderCall};
use crate::ids::UserId;
use crate::model::{PersistOp, ProviderError};
use crate::reduce::{Changed, Effect};
use crate::reminder::{Reminder, ReminderId, ReminderLink, ReminderRepeat};
use crate::state::WorkspaceState;

pub(crate) fn add(text: Arc<str>, when: Arc<str>, user: Option<UserId>, fx: &mut Vec<Effect>) {
    fx.push(Effect::Call(ProviderCall::AddReminder { text, when, user }));
}

pub(crate) fn complete(state: &mut WorkspaceState, id: ReminderId, fx: &mut Vec<Effect>) {
    if !state.caps.reminders {
        let done = state
            .reminders
            .as_mut()
            .and_then(|all| all.iter_mut().find(|r| r.id == id))
            .map(|r| {
                r.completed = true;
                r.clone()
            });
        if let Some(r) = done {
            fx.push(Effect::Changed(Changed::Reminders));
            fx.push(Effect::Persist(PersistOp::LocalReminderPut(r)));
        }
        return;
    }
    if let Some(r) = state
        .reminders
        .as_mut()
        .and_then(|all| all.iter_mut().find(|r| r.id == id))
    {
        r.completed = true;
        fx.push(Effect::Changed(Changed::Reminders));
    }
    fx.push(Effect::Call(ProviderCall::CompleteReminder(id)));
}

pub(crate) fn delete(state: &mut WorkspaceState, id: ReminderId, fx: &mut Vec<Effect>) {
    let local = !state.caps.reminders;
    let Some(all) = state.reminders.as_mut() else {
        return;
    };
    let Some(at) = all.iter().position(|r| r.id == id) else {
        return;
    };
    if local {
        all.remove(at);
        fx.push(Effect::Changed(Changed::Reminders));
        fx.push(Effect::Persist(PersistOp::LocalReminderRemove(id)));
        return;
    }
    let prev = at.checked_sub(1).map(|i| all[i].id.clone());
    let removed = all.remove(at);
    state.reminders_undo.push((prev, removed));
    fx.push(Effect::Changed(Changed::Reminders));
    fx.push(Effect::Call(ProviderCall::DeleteReminder(id)));
}

pub(crate) fn open(state: &WorkspaceState, fx: &mut Vec<Effect>) {
    // Without the provider's reminders the list comes from the store at startup.
    if state.reminders.is_none() && state.caps.reminders {
        fx.push(Effect::Load(Load::Reminders));
    }
}

/// The provider changed reminders server-side; reload the list.
pub(crate) fn changed(fx: &mut Vec<Effect>) {
    fx.push(Effect::Load(Load::Reminders));
}

pub(crate) fn call_finished(
    state: &mut WorkspaceState,
    call: ProviderCall,
    result: Result<CallOutput, ProviderError>,
    fx: &mut Vec<Effect>,
) {
    match call {
        ProviderCall::AddReminder { .. } => match result {
            Ok(CallOutput::Reminder(r)) => insert(state, r, fx),
            Ok(_) => changed(fx),
            // The request went out, so the reminder may be stored even though the answer
            // failed or lacked it; reload once so it shows if it is.
            Err(_) => changed(fx),
        },
        // The reminder is gone (or cannot be changed): do not roll back into a
        // stale item, drop any undo entry and fetch the server's truth.
        ProviderCall::CompleteReminder(id) | ProviderCall::DeleteReminder(id)
            if matches!(result, Err(ProviderError::NotAllowed(_))) =>
        {
            state.reminders_undo.retain(|(_, r)| r.id != id);
            changed(fx);
        }
        ProviderCall::CompleteReminder(id) if result.is_err() => {
            rollback_complete(state, &id, fx);
        }
        ProviderCall::DeleteReminder(id) if result.is_err() => {
            rollback_delete(state, &id, fx);
        }
        // Accepted: the undo entry is no longer needed.
        ProviderCall::DeleteReminder(id) => {
            state.reminders_undo.retain(|(_, r)| r.id != id);
        }
        _ => {}
    }
}

pub(crate) fn handle_loaded(
    state: &mut WorkspaceState,
    r: Result<Loaded, (Load, ProviderError)>,
    fx: &mut Vec<Effect>,
) {
    if let Ok(Loaded::Reminders(all)) = r {
        state.reminders = Some(all);
        state.reminders_undo.clear();
        fx.push(Effect::Changed(Changed::Reminders));
    }
}

fn insert(state: &mut WorkspaceState, r: Reminder, fx: &mut Vec<Effect>) {
    let all = state.reminders.get_or_insert_with(Vec::new);
    match all.iter_mut().find(|x| x.id == r.id) {
        Some(existing) => *existing = r,
        None => all.push(r),
    }
    fx.push(Effect::Changed(Changed::Reminders));
}

fn rollback_complete(state: &mut WorkspaceState, id: &ReminderId, fx: &mut Vec<Effect>) {
    let Some(r) = state
        .reminders
        .as_mut()
        .and_then(|all| all.iter_mut().find(|r| r.id == *id))
    else {
        return;
    };
    r.completed = false;
    fx.push(Effect::Changed(Changed::Reminders));
}

fn rollback_delete(state: &mut WorkspaceState, id: &ReminderId, fx: &mut Vec<Effect>) {
    let Some(at) = state.reminders_undo.iter().rposition(|(_, r)| r.id == *id) else {
        return;
    };
    let (prev, reminder) = state.reminders_undo.remove(at);
    let all = state.reminders.get_or_insert_with(Vec::new);
    // Put it back where it was: after the reminder it followed. The neighbour
    // may itself have been deleted meanwhile, in which case the front is the
    // closest stable place.
    let at = prev
        .and_then(|p| all.iter().position(|r| r.id == p))
        .map_or(0, |i| i + 1);
    all.insert(at, reminder);
    fx.push(Effect::Changed(Changed::Reminders));
}

pub(crate) fn add_local(
    state: &mut WorkspaceState,
    id: ReminderId,
    text: Arc<str>,
    due: i64,
    (recurring, interval_secs): (bool, i64),
    (user, link): (UserId, Option<ReminderLink>),
    fx: &mut Vec<Effect>,
) {
    let r = Reminder {
        id,
        text,
        due: Some(due),
        recurring,
        interval_secs,
        completed: false,
        fired: false,
        for_user: user,
        link,
        snoozed_until: None,
    };
    insert(state, r.clone(), fx);
    fx.push(Effect::Persist(PersistOp::LocalReminderPut(r)));
}

/// One of Parla's own reminders, to change in place; `None` for a provider's.
fn local_mut<'a>(state: &'a mut WorkspaceState, id: &ReminderId) -> Option<&'a mut Reminder> {
    if state.caps.reminders {
        return None;
    }
    state
        .reminders
        .as_mut()
        .and_then(|all| all.iter_mut().find(|r| r.id == *id))
}

fn put(r: Reminder, fx: &mut Vec<Effect>) {
    fx.push(Effect::Changed(Changed::Reminders));
    fx.push(Effect::Persist(PersistOp::LocalReminderPut(r)));
}

/// Snoozes a local reminder to `until`, open again with its alert still to
/// come. A one-off moves; a recurring one moves only this occurrence, the
/// series keeping its time.
pub(crate) fn snooze(
    state: &mut WorkspaceState,
    id: &ReminderId,
    until: i64,
    fx: &mut Vec<Effect>,
) {
    let Some(r) = local_mut(state, id) else {
        return;
    };
    if r.recurring {
        r.snoozed_until = Some(until);
    } else {
        r.due = Some(until);
        r.fired = false;
    }
    r.completed = false;
    put(r.clone(), fx);
}

/// Changes a local reminder's text, time (kept when `due` is `None`) and
/// repeat. A new time alerts again and drops a snooze; a reminder set to
/// repeat whose time has passed moves to its next occurrence after `now`.
pub(crate) fn edit(
    state: &mut WorkspaceState,
    id: &ReminderId,
    text: Arc<str>,
    due: Option<i64>,
    repeat: ReminderRepeat,
    now: i64,
    fx: &mut Vec<Effect>,
) {
    let Some(r) = local_mut(state, id) else {
        return;
    };
    r.text = text;
    if let Some(d) = due {
        r.due = Some(d);
        r.fired = false;
        r.snoozed_until = None;
    }
    r.recurring = repeat != ReminderRepeat::Never;
    r.interval_secs = repeat.interval_secs();
    if r.recurring {
        r.fired = false;
        if let Some(d) = r.due.filter(|d| *d <= now) {
            let step = r.interval_secs;
            r.due = Some(d.saturating_add(((now - d) / step + 1).saturating_mul(step)));
        }
    } else {
        r.snoozed_until = None;
    }
    put(r.clone(), fx);
}

/// A message changed or went away: the local reminders about it show its new
/// plain text, or none.
pub(crate) fn message_changed(
    state: &mut WorkspaceState,
    id: &crate::ids::MessageId,
    plain: Option<&str>,
    fx: &mut Vec<Effect>,
) {
    if state.caps.reminders {
        return;
    }
    let mut any = false;
    for r in state.reminders.iter_mut().flatten() {
        let Some(link) = r.link.as_mut().filter(|l| l.message == *id) else {
            continue;
        };
        let new = plain.map_or_else(|| Arc::from(""), crate::reminder::excerpt);
        if link.excerpt == new {
            continue;
        }
        link.excerpt = new;
        any = true;
        fx.push(Effect::Persist(PersistOp::LocalReminderPut(r.clone())));
    }
    if any {
        fx.push(Effect::Changed(Changed::Reminders));
    }
}

pub(crate) fn local_loaded(state: &mut WorkspaceState, all: Vec<Reminder>, fx: &mut Vec<Effect>) {
    state.reminders = Some(all);
    fx.push(Effect::Changed(Changed::Reminders));
}

/// When each of Parla's own reminders alerts next: open and dated, a one-off
/// not fired yet, or a snoozed occurrence.
fn pending(state: &WorkspaceState) -> impl Iterator<Item = (&Reminder, i64)> {
    state
        .reminders
        .iter()
        .flatten()
        .filter(|r| !r.completed)
        .filter_map(|r| {
            let due = r.due.filter(|_| !r.fired);
            match (r.snoozed_until, due) {
                (Some(s), Some(d)) => Some(s.min(d)),
                (s, d) => s.or(d),
            }
            .map(|at| (r, at))
        })
}

/// The unix time of the earliest pending local alert; `None` when there is
/// none or the provider keeps the reminders. The shell arms one timer for it.
pub fn next_reminder_due(state: &WorkspaceState) -> Option<i64> {
    if state.caps.reminders {
        return None;
    }
    pending(state).map(|(_, d)| d).min()
}

/// Step for a recurring reminder stored without an interval (saved before it
/// was kept): weekly.
const DEFAULT_STEP_SECS: i64 = 7 * 86_400;

/// Raises the alert of every local reminder due at `now` and marks it fired
/// (persisted), so it never alerts again (a recurring one moves to its next due), also after a restart. Past-due ones
/// found at startup alert once. While the user's notifications are paused
/// nothing alerts and nothing is marked: the shell ticks again when the pause
/// ends, so the held alert comes once, also after a restart.
pub(crate) fn tick(state: &mut WorkspaceState, now: i64, fx: &mut Vec<Effect>) {
    if state.caps.reminders {
        return;
    }
    let paused = state
        .self_user
        .as_ref()
        .and_then(|me| state.dnd.get(me))
        .is_some_and(|d| d.active(now));
    if paused {
        return;
    }
    let Some(all) = state.reminders.as_mut() else {
        return;
    };
    let mut due: Vec<&mut Reminder> = all
        .iter_mut()
        .filter(|r| {
            !r.completed
                && (r.snoozed_until.is_some_and(|s| s <= now)
                    || (!r.fired && r.due.is_some_and(|d| d <= now)))
        })
        .collect();
    due.sort_by_key(|r| r.next_at());
    if due.is_empty() {
        return;
    }
    for r in due {
        // One alert covers a snoozed occurrence and a due one at once.
        let alerted = r.clone();
        r.snoozed_until = r.snoozed_until.filter(|s| *s > now);
        let series_due = !r.fired && r.due.is_some_and(|d| d <= now);
        if series_due && r.recurring {
            // Repeats by its own interval, skipping the occurrences missed
            // while Parla was closed.
            let step = if r.interval_secs > 0 {
                r.interval_secs
            } else {
                DEFAULT_STEP_SECS
            };
            let d = r.due.unwrap_or(now);
            let missed = (now - d) / step + 1;
            r.due = Some(d.saturating_add(missed.saturating_mul(step)));
        } else if series_due {
            r.fired = true;
        }
        fx.push(Effect::Persist(PersistOp::LocalReminderPut(r.clone())));
        fx.push(Effect::ReminderDue(if series_due && !r.recurring {
            r.clone()
        } else {
            alerted
        }));
    }
    fx.push(Effect::Changed(Changed::Reminders));
}
