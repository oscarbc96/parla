//! Scheduled sends: presets, limits, and the reducer rules for provider-side
//! and local schedules.

use std::collections::HashSet;
use std::sync::Arc;

use jiff::tz::TimeZone;
use jiff::{Timestamp, ZonedDateTime};

use crate::calls::{CallOutput, Load, Loaded, ProviderCall};
use crate::files::Upload;
use crate::ids::{ConversationId, MessageTs};
use crate::model::{PersistOp, ProviderError};
use crate::reduce::{Changed, Effect};
use crate::schedule::{LocalSchedule, LocalScheduleState, ScheduleId, ScheduledMessage};
use crate::state::WorkspaceState;
use crate::when_parse::{at, next_weekday, offset, short_unit, time, weekday};

/// The provider refuses schedules further ahead than this.
pub const MAX_AHEAD_DAYS: i64 = 120;
/// A local schedule overdue by more than this asks before sending.
pub const LATE_AFTER_SECS: i64 = 300;

const MAY_HAVE_SENT: &str = "may have been sent; check before retrying";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    Past,
    TooFar,
    /// Files, or a provider without scheduled send: Parla keeps it locally.
    FilesNeedLocal,
}

/// The zone schedule times are read in: the override, else the profile's,
/// else the system's. An unknown name is skipped.
pub fn schedule_zone(profile_tz: Option<&str>, override_tz: Option<&str>) -> TimeZone {
    zone_or(profile_tz, override_tz, TimeZone::system)
}

/// The one zone rule: the override, else the profile's, else `fallback()`.
fn zone_or(
    profile_tz: Option<&str>,
    override_tz: Option<&str>,
    fallback: impl FnOnce() -> TimeZone,
) -> TimeZone {
    [override_tz, profile_tz]
        .into_iter()
        .flatten()
        .find_map(|n| TimeZone::get(n).ok())
        .unwrap_or_else(fallback)
}

/// [`schedule_zone`] for the signed-in user of `state`: the override, else the
/// user's profile zone, else `fallback`.
pub fn state_schedule_zone(
    state: &WorkspaceState,
    override_tz: Option<&str>,
    fallback: TimeZone,
) -> TimeZone {
    let profile_tz = state
        .self_user
        .as_ref()
        .and_then(|u| state.profiles.get(u))
        .and_then(|p| p.tz.as_deref());
    zone_or(profile_tz, override_tz, || fallback)
}

/// `"tomorrow HH:MM"`, `"today HH:MM"` (only while still ahead),
/// `"<weekday> HH:MM"` (the next such day, never today), `"in <n>m|h|d"`.
pub fn parse_preset(p: &str, now: &ZonedDateTime) -> Option<Timestamp> {
    let p = p.trim().to_ascii_lowercase();
    let mut it = p.split_whitespace();
    let (head, arg) = (it.next()?, it.next()?);
    if it.next().is_some() {
        return None;
    }
    if head == "in" {
        return relative(arg, now);
    }
    let time = time(arg)?;
    let today = now.date();
    let date = match head {
        "today" => today,
        "tomorrow" => today.tomorrow().ok()?,
        w => next_weekday(today, weekday(w)?)?,
    };
    let at = at(date, time, now.time_zone())?;
    (at > now.timestamp()).then_some(at)
}

fn relative(arg: &str, now: &ZonedDateTime) -> Option<Timestamp> {
    let unit = arg.chars().last()?;
    let n: i64 = arg[..arg.len() - unit.len_utf8()].parse().ok()?;
    if n <= 0 {
        return None;
    }
    offset(now, n, short_unit(unit)?)
}

/// `post_at` and `now` are unix seconds.
pub fn validate(
    post_at: i64,
    now: i64,
    has_files: bool,
    server_capable: bool,
) -> Result<(), ScheduleError> {
    if post_at <= now {
        Err(ScheduleError::Past)
    } else if post_at > now + MAX_AHEAD_DAYS * 86_400 {
        Err(ScheduleError::TooFar)
    } else if has_files || !server_capable {
        Err(ScheduleError::FilesNeedLocal)
    } else {
        Ok(())
    }
}

/// A server schedule call in flight, kept so its failure is never a lost
/// message.
#[derive(Clone, Debug)]
pub(crate) struct PendingSchedule {
    conversation: ConversationId,
    thread: Option<MessageTs>,
    text: Arc<str>,
    post_at: i64,
    /// Id the text gets if it ends up local.
    fallback: ScheduleId,
    created_ms: i64,
    /// A reschedule's second half: any failure keeps the text as `Failed`.
    reschedule: bool,
}

/// A reschedule waiting for its delete to succeed.
#[derive(Clone, Debug)]
pub(crate) struct PendingResched {
    old: ScheduleId,
    text: Arc<str>,
    post_at: i64,
}

/// Private bookkeeping, kept out of the public `WorkspaceState` fields.
#[derive(Clone, Debug, Default)]
pub(crate) struct SchedBook {
    calls: Vec<PendingSchedule>,
    resched: Vec<PendingResched>,
    /// Late items the user has been asked about.
    late_asked: HashSet<ScheduleId>,
    /// Late items the user said to send.
    late_ok: HashSet<ScheduleId>,
}

pub(crate) struct SendArgs {
    pub conversation: ConversationId,
    pub thread: Option<MessageTs>,
    pub text: Arc<str>,
    pub files: Vec<Upload>,
    pub post_at: i64,
    pub local_id: ScheduleId,
    pub now_ms: i64,
}

pub(crate) fn schedule_send(state: &mut WorkspaceState, a: SendArgs, fx: &mut Vec<Effect>) {
    if a.files.is_empty() {
        state.sched.calls.push(PendingSchedule {
            conversation: a.conversation.clone(),
            thread: a.thread.clone(),
            text: a.text.clone(),
            post_at: a.post_at,
            fallback: a.local_id,
            created_ms: a.now_ms,
            reschedule: false,
        });
        fx.push(Effect::Call(ProviderCall::Schedule {
            conversation: a.conversation,
            thread: a.thread,
            text: a.text,
            post_at: a.post_at,
        }));
    } else {
        add_local(
            state,
            LocalSchedule {
                id: a.local_id,
                conversation: a.conversation,
                thread: a.thread,
                text: a.text,
                files: a.files,
                post_at: a.post_at,
                created_ms: a.now_ms,
                state: LocalScheduleState::Pending,
            },
            fx,
        );
    }
}

pub(crate) fn reschedule(
    state: &mut WorkspaceState,
    item: ScheduledMessage,
    text: Arc<str>,
    post_at: i64,
    fx: &mut Vec<Effect>,
) {
    state.sched.resched.push(PendingResched {
        old: item.id.clone(),
        text,
        post_at,
    });
    fx.push(Effect::Call(ProviderCall::DeleteScheduled(item)));
}

pub(crate) fn delete(state: &mut WorkspaceState, id: ScheduleId, fx: &mut Vec<Effect>) {
    if remove_local(state, &id, fx) {
        wake(state, fx);
    } else if let Some(item) = state.scheduled.iter().find(|s| s.id == id) {
        fx.push(Effect::Call(ProviderCall::DeleteScheduled(item.clone())));
    }
}

pub(crate) fn call_finished(
    state: &mut WorkspaceState,
    call: ProviderCall,
    result: Result<CallOutput, ProviderError>,
    fx: &mut Vec<Effect>,
) {
    match call {
        ProviderCall::Schedule {
            conversation,
            thread,
            text,
            post_at,
        } => {
            let Some(i) = state.sched.calls.iter().position(|p| {
                p.conversation == conversation
                    && p.thread == thread
                    && p.text == text
                    && p.post_at == post_at
            }) else {
                return;
            };
            let p = state.sched.calls.remove(i);
            match result {
                Ok(CallOutput::Scheduled(s)) => {
                    insert_scheduled(state, s);
                    fx.push(Effect::Changed(Changed::Scheduled));
                }
                Ok(_) => fx.push(Effect::Load(Load::Scheduled)),
                Err(e) => {
                    // Never lose the text: no server support keeps it as a
                    // pending local schedule, any other failure as a failed one.
                    let st = if !p.reschedule && e == ProviderError::Unsupported {
                        LocalScheduleState::Pending
                    } else {
                        LocalScheduleState::Failed(e.to_string())
                    };
                    add_local(
                        state,
                        LocalSchedule {
                            id: p.fallback,
                            conversation: p.conversation,
                            thread: p.thread,
                            text: p.text,
                            files: vec![],
                            post_at: p.post_at,
                            created_ms: p.created_ms,
                            state: st,
                        },
                        fx,
                    );
                }
            }
        }
        ProviderCall::DeleteScheduled(item) => {
            let r = state.sched.resched.iter().position(|r| r.old == item.id);
            let r = r.map(|i| state.sched.resched.remove(i));
            match &result {
                Ok(_) => {}
                Err(ProviderError::NotAllowed(_)) => {
                    // The item is gone or cannot be removed (already sent?): drop it
                    // locally, abandon any reschedule so no second copy is scheduled, and
                    // refetch the server's truth.
                    let before = state.scheduled.len();
                    state.scheduled.retain(|s| s.id != item.id);
                    if state.scheduled.len() != before {
                        fx.push(Effect::Changed(Changed::Scheduled));
                    }
                    refetch(fx);
                    return;
                }
                Err(_) => {
                    // Nothing changed server side; a reschedule is abandoned.
                    return;
                }
            }
            let before = state.scheduled.len();
            state.scheduled.retain(|s| s.id != item.id);
            if state.scheduled.len() != before {
                fx.push(Effect::Changed(Changed::Scheduled));
            }
            if let Some(r) = r {
                state.sched.calls.push(PendingSchedule {
                    conversation: item.conversation.clone(),
                    thread: item.thread.clone(),
                    text: r.text.clone(),
                    post_at: r.post_at,
                    fallback: ScheduleId(Arc::from(format!("local-{}", item.id.0))),
                    created_ms: 0,
                    reschedule: true,
                });
                fx.push(Effect::Call(ProviderCall::Schedule {
                    conversation: item.conversation,
                    thread: item.thread,
                    text: r.text,
                    post_at: r.post_at,
                }));
            }
        }
        _ => {}
    }
}

pub(crate) fn loaded(state: &mut WorkspaceState, all: Vec<ScheduledMessage>, fx: &mut Vec<Effect>) {
    state.scheduled = all;
    state.scheduled.sort_by_key(|s| s.post_at);
    fx.push(Effect::Changed(Changed::Scheduled));
}

pub(crate) fn local_loaded(
    state: &mut WorkspaceState,
    mut all: Vec<LocalSchedule>,
    fx: &mut Vec<Effect>,
) {
    for l in &mut all {
        if l.state == LocalScheduleState::Sending {
            l.state = LocalScheduleState::Failed(MAY_HAVE_SENT.into());
            fx.push(Effect::Persist(PersistOp::LocalSchedulePut(l.clone())));
        }
    }
    // The read answers with the store as it was when it was taken (at the first
    // connection). A schedule added while it was in flight is not in it, and an
    // item it knows may have moved on since (a send started, an edit landed):
    // keep the session's copy for every id it already holds, then merge.
    all.retain(|l| !state.local_schedules.iter().any(|s| s.id == l.id));
    all.append(&mut state.local_schedules);
    all.sort_by_key(|l| l.post_at);
    state.local_schedules = all;
    fx.push(Effect::Changed(Changed::Scheduled));
    wake(state, fx);
}

pub(crate) fn tick(state: &mut WorkspaceState, now: i64, fx: &mut Vec<Effect>) {
    let late: Vec<ScheduleId> = state
        .local_schedules
        .iter()
        .filter(|l| {
            l.state == LocalScheduleState::Pending
                && now - l.post_at > LATE_AFTER_SECS
                && !state.sched.late_asked.contains(&l.id)
                && !state.sched.late_ok.contains(&l.id)
        })
        .map(|l| l.id.clone())
        .collect();
    if !late.is_empty() {
        state.sched.late_asked.extend(late.iter().cloned());
        fx.push(Effect::AskLate(late));
    }
    let busy = state
        .local_schedules
        .iter()
        .any(|l| l.state == LocalScheduleState::Sending);
    if !busy {
        let next = state.local_schedules.iter().find(|l| {
            l.state == LocalScheduleState::Pending
                && l.post_at <= now
                && !state.sched.late_asked.contains(&l.id)
                && (now - l.post_at <= LATE_AFTER_SECS || state.sched.late_ok.contains(&l.id))
        });
        if let Some(id) = next.map(|l| l.id.clone()) {
            start_send(state, &id, fx);
        }
    }
    wake(state, fx);
}

pub(crate) fn late_answer(
    state: &mut WorkspaceState,
    ids: Vec<ScheduleId>,
    send: bool,
    fx: &mut Vec<Effect>,
) {
    for id in &ids {
        state.sched.late_asked.remove(id);
    }
    if send {
        state.sched.late_ok.extend(ids);
        send_next_late_ok(state, fx);
    } else {
        for id in &ids {
            remove_local(state, id, fx);
        }
    }
    wake(state, fx);
}

pub(crate) fn local_sent(
    state: &mut WorkspaceState,
    id: ScheduleId,
    result: Result<(), ProviderError>,
    fx: &mut Vec<Effect>,
) {
    state.sched.late_ok.remove(&id);
    match result {
        Ok(()) => {
            remove_local(state, &id, fx);
        }
        Err(e) => {
            if let Some(l) = state.local_schedules.iter_mut().find(|l| l.id == id) {
                l.state = LocalScheduleState::Failed(e.to_string());
                fx.push(Effect::Persist(PersistOp::LocalSchedulePut(l.clone())));
                fx.push(Effect::Changed(Changed::Scheduled));
            }
        }
    }
    send_next_late_ok(state, fx);
    wake(state, fx);
}

/// Items the user approved go straight out, one at a time.
fn send_next_late_ok(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if state
        .local_schedules
        .iter()
        .any(|l| l.state == LocalScheduleState::Sending)
    {
        return;
    }
    let next = state
        .local_schedules
        .iter()
        .find(|l| l.state == LocalScheduleState::Pending && state.sched.late_ok.contains(&l.id))
        .map(|l| l.id.clone());
    if let Some(id) = next {
        start_send(state, &id, fx);
    }
}

fn start_send(state: &mut WorkspaceState, id: &ScheduleId, fx: &mut Vec<Effect>) {
    if let Some(l) = state.local_schedules.iter_mut().find(|l| l.id == *id) {
        l.state = LocalScheduleState::Sending;
        // Persisted before the send so a crash never resends it.
        fx.push(Effect::Persist(PersistOp::LocalSchedulePut(l.clone())));
        fx.push(Effect::SendLocal(l.clone()));
        fx.push(Effect::Changed(Changed::Scheduled));
    }
}

fn add_local(state: &mut WorkspaceState, l: LocalSchedule, fx: &mut Vec<Effect>) {
    fx.push(Effect::Persist(PersistOp::LocalSchedulePut(l.clone())));
    let at = state
        .local_schedules
        .partition_point(|x| x.post_at <= l.post_at);
    state.local_schedules.insert(at, l);
    fx.push(Effect::Changed(Changed::Scheduled));
    wake(state, fx);
}

fn remove_local(state: &mut WorkspaceState, id: &ScheduleId, fx: &mut Vec<Effect>) -> bool {
    let before = state.local_schedules.len();
    state.local_schedules.retain(|l| l.id != *id);
    state.sched.late_asked.remove(id);
    state.sched.late_ok.remove(id);
    if state.local_schedules.len() == before {
        return false;
    }
    fx.push(Effect::Persist(PersistOp::LocalScheduleRemove(id.clone())));
    fx.push(Effect::Changed(Changed::Scheduled));
    true
}

/// The next time a tick is needed: the earliest pending item not waiting on
/// an answer. `None` cancels the timer.
fn wake(state: &WorkspaceState, fx: &mut Vec<Effect>) {
    // One send at a time: while one is in flight an overdue item must not arm
    // a past timer (a busy loop). `local_sent` re-arms.
    let busy = state
        .local_schedules
        .iter()
        .any(|l| l.state == LocalScheduleState::Sending);
    if busy {
        fx.push(Effect::WakeAt(None));
        return;
    }
    let next = state
        .local_schedules
        .iter()
        .filter(|l| {
            l.state == LocalScheduleState::Pending && !state.sched.late_asked.contains(&l.id)
        })
        .map(|l| l.post_at)
        .min();
    fx.push(Effect::WakeAt(next));
}

/// Pending provider-side schedule after `Loaded`/`ScheduledChanged`.
pub(crate) fn refetch(fx: &mut Vec<Effect>) {
    fx.push(Effect::Load(Load::Scheduled));
}

pub(crate) fn handle_loaded(
    state: &mut WorkspaceState,
    r: Result<Loaded, (Load, ProviderError)>,
    fx: &mut Vec<Effect>,
) {
    if let Ok(Loaded::Scheduled(all)) = r {
        loaded(state, all, fx);
    }
}

fn insert_scheduled(state: &mut WorkspaceState, s: ScheduledMessage) {
    state.scheduled.retain(|x| x.id != s.id);
    let at = state.scheduled.partition_point(|x| x.post_at <= s.post_at);
    state.scheduled.insert(at, s);
}
