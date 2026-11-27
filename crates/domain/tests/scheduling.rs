use std::path::PathBuf;
use std::sync::Arc;

use domain::test_support::{cid, uid};
use domain::{
    Bootstrap, CallOutput, Changed, Effect, Input, LATE_AFTER_SECS, LocalSchedule,
    LocalScheduleState, MAX_AHEAD_DAYS, PersistOp, ProviderCall, ProviderError, ScheduleError,
    ScheduleId, ScheduledMessage, Upload, WorkspaceState, parse_preset, reduce, schedule_zone,
    state_schedule_zone, validate,
};
use jiff::{Timestamp, ZonedDateTime, civil, tz::TimeZone};
use settings::WorkspaceKey;

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![],
            members: vec![],
        }),
    );
    s
}
fn sid(s: &str) -> ScheduleId {
    ScheduleId(Arc::from(s))
}
fn madrid(dt: civil::DateTime) -> ZonedDateTime {
    dt.to_zoned_datetime(TimeZone::get("Europe/Madrid").unwrap())
        .unwrap()
}
fn send(s: &mut WorkspaceState, id: &str, files: Vec<Upload>, post_at: i64) -> Vec<Effect> {
    reduce(
        s,
        Input::ScheduleSend {
            conversation: cid("C1"),
            thread: None,
            text: Arc::from("later"),
            files,
            post_at,
            local_id: sid(id),
            now_ms: 1_000,
        },
    )
}
fn upload() -> Upload {
    Upload {
        path: PathBuf::from("/x/a.png"),
        name: Arc::from("a.png"),
        mime: Arc::from("image/png"),
        size: 1,
        filetype: None,
    }
}
fn local(id: &str, post_at: i64, state: LocalScheduleState) -> LocalSchedule {
    LocalSchedule {
        id: sid(id),
        conversation: cid("C1"),
        thread: None,
        text: Arc::from(id),
        files: vec![],
        post_at,
        created_ms: 0,
        state,
    }
}
fn wake(fx: &[Effect]) -> Option<Option<i64>> {
    fx.iter().rev().find_map(|e| match e {
        Effect::WakeAt(t) => Some(*t),
        _ => None,
    })
}
fn sent(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::SendLocal(l) => Some(l.id.0.to_string()),
            _ => None,
        })
        .collect()
}
fn sm(id: &str, text: &str, post_at: i64) -> ScheduledMessage {
    ScheduledMessage {
        id: sid(id),
        conversation: cid("C1"),
        thread: None,
        text: Arc::from(text),
        post_at,
    }
}

#[test]
fn presets_in_zone() {
    let now = madrid(civil::date(2026, 11, 16).at(18, 0, 0, 0)); // Monday
    let t = parse_preset("tomorrow 09:00", &now).unwrap();
    assert_eq!(t, "2026-11-17T08:00:00Z".parse::<Timestamp>().unwrap());
    let t = parse_preset("monday 09:00", &now).unwrap();
    assert_eq!(t, "2026-11-23T08:00:00Z".parse::<Timestamp>().unwrap());
    let t = parse_preset("Friday 17:30", &now).unwrap();
    assert_eq!(t, "2026-11-20T16:30:00Z".parse::<Timestamp>().unwrap());
    let t = parse_preset("in 90m", &now).unwrap();
    assert_eq!(t, now.timestamp() + jiff::SignedDuration::from_mins(90));
    let t = parse_preset("in 2h", &now).unwrap();
    assert_eq!(t, now.timestamp() + jiff::SignedDuration::from_hours(2));
    let t = parse_preset("in 3d", &now).unwrap();
    assert_eq!(t, "2026-11-19T17:00:00Z".parse::<Timestamp>().unwrap());
    assert!(parse_preset("someday", &now).is_none());
    assert!(parse_preset("tomorrow 25:00", &now).is_none());
    assert!(parse_preset("in 0m", &now).is_none());
}

#[test]
fn today_preset_only_if_ahead() {
    let now = madrid(civil::date(2026, 11, 16).at(18, 0, 0, 0));
    assert!(parse_preset("today 09:00", &now).is_none());
    assert!(parse_preset("today 18:00", &now).is_none());
    let t = parse_preset("today 19:00", &now).unwrap();
    assert_eq!(t, "2026-11-16T18:00:00Z".parse::<Timestamp>().unwrap());
}

#[test]
fn zone_override_wins() {
    let z = schedule_zone(Some("Europe/Madrid"), Some("Asia/Tokyo"));
    assert_eq!(z.iana_name(), Some("Asia/Tokyo"));
    let z = schedule_zone(Some("Europe/Madrid"), None);
    assert_eq!(z.iana_name(), Some("Europe/Madrid"));
    // An unknown override falls through to the profile zone.
    let z = schedule_zone(Some("Europe/Madrid"), Some("Not/AZone"));
    assert_eq!(z.iana_name(), Some("Europe/Madrid"));
    // Neither: the system zone, whatever it is.
    let _ = schedule_zone(None, None);
}

#[test]
fn validate_past_too_far_and_files() {
    let now = 1_000_000;
    assert_eq!(
        validate(now - 1, now, false, true),
        Err(ScheduleError::Past)
    );
    assert_eq!(validate(now, now, false, true), Err(ScheduleError::Past));
    assert_eq!(validate(now + 60, now, false, true), Ok(()));
    let far = now + MAX_AHEAD_DAYS * 86_400;
    assert_eq!(validate(far, now, false, true), Ok(()));
    assert_eq!(
        validate(far + 1, now, false, true),
        Err(ScheduleError::TooFar)
    );
    assert_eq!(
        validate(now + 60, now, true, true),
        Err(ScheduleError::FilesNeedLocal)
    );
    assert_eq!(
        validate(now + 60, now, false, false),
        Err(ScheduleError::FilesNeedLocal)
    );
}

#[test]
fn server_schedule_inserts_on_success() {
    let mut s = state();
    let fx = send(&mut s, "l1", vec![], 5_000);
    let call = fx
        .iter()
        .find_map(|e| match e {
            Effect::Call(c @ ProviderCall::Schedule { .. }) => Some(c.clone()),
            _ => None,
        })
        .expect("Call(Schedule)");
    assert!(s.local_schedules.is_empty());
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call,
            result: Ok(CallOutput::Scheduled(sm("S1", "later", 5_000))),
        },
    );
    assert_eq!(s.scheduled.len(), 1);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Scheduled)))
    );
    // Loaded replaces the list, ordered by post_at.
    reduce(
        &mut s,
        Input::Loaded(Ok(domain::Loaded::Scheduled(vec![
            sm("B", "b", 9),
            sm("A", "a", 3),
        ]))),
    );
    let ids: Vec<_> = s.scheduled.iter().map(|x| x.id.0.to_string()).collect();
    assert_eq!(ids, ["A", "B"]);
}

#[test]
fn scheduled_changed_refetches() {
    let mut s = state();
    let fx = reduce(
        &mut s,
        Input::Provider(domain::ProviderEvent::ScheduledChanged),
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Load(domain::Load::Scheduled)))
    );
}

#[test]
fn files_schedule_locally() {
    let mut s = state();
    let fx = send(&mut s, "l1", vec![upload()], 5_000);
    assert!(!fx.iter().any(|e| matches!(e, Effect::Call(_))));
    assert_eq!(s.local_schedules.len(), 1);
    assert_eq!(s.local_schedules[0].state, LocalScheduleState::Pending);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::LocalSchedulePut(_))))
    );
    assert_eq!(wake(&fx), Some(Some(5_000)));
}

#[test]
fn unsupported_server_falls_back_to_local() {
    let mut s = state();
    let fx = send(&mut s, "l1", vec![], 5_000);
    let call = fx
        .iter()
        .find_map(|e| match e {
            Effect::Call(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call,
            result: Err(ProviderError::Unsupported),
        },
    );
    assert_eq!(s.local_schedules.len(), 1);
    assert_eq!(s.local_schedules[0].id, sid("l1"));
    assert_eq!(s.local_schedules[0].state, LocalScheduleState::Pending);
    assert_eq!(wake(&fx), Some(Some(5_000)));
}

#[test]
fn a_schedule_added_while_the_first_read_is_in_flight_survives_it() {
    let mut s = state();
    // The store's first read is issued at login; the user schedules before it
    // answers, and the provider has no server-side schedules to offer.
    let fx = send(&mut s, "l1", vec![], 5_000);
    let call = fx
        .iter()
        .find_map(|e| match e {
            Effect::Call(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    let _ = reduce(
        &mut s,
        Input::CallFinished {
            call,
            result: Err(ProviderError::Unsupported),
        },
    );
    assert_eq!(s.local_schedules.len(), 1);
    // The read, taken before the schedule was stored, answers now.
    let fx = reduce(&mut s, Input::LocalSchedulesLoaded(vec![]));
    let ids: Vec<_> = s
        .local_schedules
        .iter()
        .map(|l| l.id.0.to_string())
        .collect();
    assert_eq!(ids, ["l1"], "the new schedule is not dropped");
    assert_eq!(wake(&fx), Some(Some(5_000)), "and its timer stays armed");
}

#[test]
fn a_read_in_flight_does_not_override_a_newer_local_state() {
    let mut s = state();
    let _ = send(&mut s, "l1", vec![upload()], 5_000);
    assert_eq!(s.local_schedules.len(), 1);
    // The send starts while the read is in flight.
    let fx_send = reduce(&mut s, Input::LocalTick { now: 5_000 });
    assert_eq!(sent(&fx_send), ["l1"]);
    assert_eq!(s.local_schedules[0].state, LocalScheduleState::Sending);
    // The read still holds the old pending copy: it must not reset the state.
    let fx = reduce(
        &mut s,
        Input::LocalSchedulesLoaded(vec![local("l1", 5_000, LocalScheduleState::Pending)]),
    );
    assert_eq!(s.local_schedules.len(), 1);
    assert_eq!(s.local_schedules[0].state, LocalScheduleState::Sending);
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::SendLocal(_))),
        "no second send"
    );
}

#[test]
fn reschedule_is_delete_then_schedule() {
    let mut s = state();
    let item = sm("S1", "old", 5_000);
    s.scheduled.push(item.clone());
    let fx = reduce(
        &mut s,
        Input::Reschedule {
            item,
            text: Arc::from("new"),
            post_at: 9_000,
        },
    );
    let calls: Vec<_> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::Call(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 1);
    assert!(matches!(calls[0], ProviderCall::DeleteScheduled(_)));
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: calls[0].clone(),
            result: Ok(CallOutput::None),
        },
    );
    assert!(s.scheduled.is_empty());
    let next = fx
        .iter()
        .find_map(|e| match e {
            Effect::Call(ProviderCall::Schedule { text, post_at, .. }) => {
                Some((text.to_string(), *post_at))
            }
            _ => None,
        })
        .expect("Call(Schedule) after delete");
    assert_eq!(next, ("new".to_string(), 9_000));
}

#[test]
fn reschedule_failure_keeps_text() {
    let mut s = state();
    let item = sm("S1", "old", 5_000);
    s.scheduled.push(item.clone());
    let fx = reduce(
        &mut s,
        Input::Reschedule {
            item,
            text: Arc::from("new"),
            post_at: 9_000,
        },
    );
    let del = fx
        .iter()
        .find_map(|e| match e {
            Effect::Call(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: del,
            result: Ok(CallOutput::None),
        },
    );
    let sch = fx
        .iter()
        .find_map(|e| match e {
            Effect::Call(c @ ProviderCall::Schedule { .. }) => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: sch,
            result: Err(ProviderError::Other("boom".into())),
        },
    );
    assert_eq!(s.local_schedules.len(), 1);
    let l = &s.local_schedules[0];
    assert_eq!(&*l.text, "new");
    assert!(matches!(l.state, LocalScheduleState::Failed(_)));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::LocalSchedulePut(_))))
    );
}

#[test]
fn reschedule_delete_failure_changes_nothing() {
    let mut s = state();
    let item = sm("S1", "old", 5_000);
    s.scheduled.push(item.clone());
    let fx = reduce(
        &mut s,
        Input::Reschedule {
            item,
            text: Arc::from("new"),
            post_at: 9_000,
        },
    );
    let del = fx
        .iter()
        .find_map(|e| match e {
            Effect::Call(c) => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: del,
            result: Err(ProviderError::Other("boom".into())),
        },
    );
    assert_eq!(s.scheduled.len(), 1);
    assert!(s.local_schedules.is_empty());
    assert!(!fx.iter().any(|e| matches!(e, Effect::Call(_))));
}

#[test]
fn delete_schedule_server_and_local() {
    let mut s = state();
    s.scheduled.push(sm("S1", "x", 5_000));
    let fx = reduce(&mut s, Input::DeleteSchedule(sid("S1")));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Call(ProviderCall::DeleteScheduled(_))))
    );
    reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::DeleteScheduled(sm("S1", "x", 5_000)),
            result: Ok(CallOutput::None),
        },
    );
    assert!(s.scheduled.is_empty());
    reduce(
        &mut s,
        Input::LocalSchedulesLoaded(vec![local("l1", 5_000, LocalScheduleState::Pending)]),
    );
    let fx = reduce(&mut s, Input::DeleteSchedule(sid("l1")));
    assert!(s.local_schedules.is_empty());
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::LocalScheduleRemove(_))))
    );
    assert_eq!(wake(&fx), Some(None));
}

#[test]
fn local_due_sends_one_at_a_time() {
    let mut s = state();
    let now = 10_000;
    reduce(
        &mut s,
        Input::LocalSchedulesLoaded(vec![
            local("a", now - 10, LocalScheduleState::Pending),
            local("b", now - 5, LocalScheduleState::Pending),
            local("c", now + 600, LocalScheduleState::Pending),
        ]),
    );
    let fx = reduce(&mut s, Input::LocalTick { now });
    assert_eq!(sent(&fx), ["a"]);
    // Persisted as Sending before SendLocal.
    let put = fx
        .iter()
        .position(|e| matches!(e, Effect::Persist(PersistOp::LocalSchedulePut(l)) if l.state == LocalScheduleState::Sending))
        .unwrap();
    let send_at = fx
        .iter()
        .position(|e| matches!(e, Effect::SendLocal(_)))
        .unwrap();
    assert!(put < send_at);
    // While one is in flight a tick sends nothing.
    let fx = reduce(&mut s, Input::LocalTick { now });
    assert!(sent(&fx).is_empty());
    // Completion removes it; the next due one waits for the wake.
    let fx = reduce(
        &mut s,
        Input::LocalSent {
            id: sid("a"),
            result: Ok(()),
        },
    );
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Persist(PersistOp::LocalScheduleRemove(i)) if *i == sid("a"))
    ));
    assert_eq!(wake(&fx), Some(Some(now - 5)));
    let fx = reduce(&mut s, Input::LocalTick { now });
    assert_eq!(sent(&fx), ["b"]);
    // A failure keeps the item as Failed.
    reduce(
        &mut s,
        Input::LocalSent {
            id: sid("b"),
            result: Err(ProviderError::Other("nope".into())),
        },
    );
    assert!(
        s.local_schedules
            .iter()
            .any(|l| l.id == sid("b") && matches!(l.state, LocalScheduleState::Failed(_)))
    );
}

#[test]
fn late_items_ask() {
    let mut s = state();
    let now = 100_000;
    reduce(
        &mut s,
        Input::LocalSchedulesLoaded(vec![
            local("late1", now - 600, LocalScheduleState::Pending),
            local(
                "late2",
                now - LATE_AFTER_SECS - 1,
                LocalScheduleState::Pending,
            ),
            local("ok", now - 10, LocalScheduleState::Pending),
        ]),
    );
    let fx = reduce(&mut s, Input::LocalTick { now });
    let ask: Vec<Vec<ScheduleId>> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::AskLate(v) => Some(v.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(ask.len(), 1);
    assert_eq!(ask[0], vec![sid("late1"), sid("late2")]);
    // The on-time one still goes; late ones wait for the answer.
    assert_eq!(sent(&fx), ["ok"]);
    // The next tick does not ask again.
    reduce(
        &mut s,
        Input::LocalSent {
            id: sid("ok"),
            result: Ok(()),
        },
    );
    let fx = reduce(&mut s, Input::LocalTick { now: now + 1 });
    assert!(!fx.iter().any(|e| matches!(e, Effect::AskLate(_))));
    assert!(sent(&fx).is_empty());
    // Yes: they go one at a time.
    let fx = reduce(
        &mut s,
        Input::SendLateAnswer {
            ids: vec![sid("late1"), sid("late2")],
            send: true,
        },
    );
    assert_eq!(sent(&fx), ["late1"]);
    let fx = reduce(
        &mut s,
        Input::LocalSent {
            id: sid("late1"),
            result: Ok(()),
        },
    );
    assert_eq!(sent(&fx), ["late2"]);
}

#[test]
fn late_items_declined_are_dropped() {
    let mut s = state();
    let now = 100_000;
    reduce(
        &mut s,
        Input::LocalSchedulesLoaded(vec![local("late", now - 600, LocalScheduleState::Pending)]),
    );
    reduce(&mut s, Input::LocalTick { now });
    let fx = reduce(
        &mut s,
        Input::SendLateAnswer {
            ids: vec![sid("late")],
            send: false,
        },
    );
    assert!(s.local_schedules.is_empty());
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::LocalScheduleRemove(_))))
    );
    assert!(sent(&fx).is_empty());
}

#[test]
fn sending_at_restart_is_failed_not_resent() {
    let mut s = state();
    let fx = reduce(
        &mut s,
        Input::LocalSchedulesLoaded(vec![local("a", 10, LocalScheduleState::Sending)]),
    );
    assert!(
        matches!(&s.local_schedules[0].state, LocalScheduleState::Failed(m) if m.contains("may have been sent"))
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::LocalSchedulePut(_))))
    );
    let fx = reduce(&mut s, Input::LocalTick { now: 20 });
    assert!(sent(&fx).is_empty());
    assert_eq!(wake(&fx), Some(None));
}

#[test]
fn wake_at_tracks_next_due() {
    let mut s = state();
    let fx = reduce(
        &mut s,
        Input::LocalSchedulesLoaded(vec![
            local("b", 700, LocalScheduleState::Pending),
            local("a", 500, LocalScheduleState::Pending),
            local("f", 100, LocalScheduleState::Failed("x".into())),
        ]),
    );
    assert_eq!(wake(&fx), Some(Some(500)));
    let fx = reduce(&mut s, Input::DeleteSchedule(sid("a")));
    assert_eq!(wake(&fx), Some(Some(700)));
    let fx = reduce(&mut s, Input::DeleteSchedule(sid("b")));
    assert_eq!(wake(&fx), Some(None));
}

#[test]
fn delete_of_gone_item_removes_it_and_refetches() {
    let mut s = state();
    s.scheduled.push(sm("S1", "x", 5_000));
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::DeleteScheduled(sm("S1", "x", 5_000)),
            result: Err(ProviderError::NotAllowed("gone".into())),
        },
    );
    assert!(s.scheduled.is_empty());
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Load(domain::Load::Scheduled)))
    );
}

#[test]
fn reschedule_of_gone_item_does_not_schedule_a_copy() {
    let mut s = state();
    let item = sm("S1", "old", 5_000);
    s.scheduled.push(item.clone());
    reduce(
        &mut s,
        Input::Reschedule {
            item: item.clone(),
            text: Arc::from("new"),
            post_at: 9_000,
        },
    );
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::DeleteScheduled(item),
            result: Err(ProviderError::NotAllowed("gone".into())),
        },
    );
    assert!(s.scheduled.is_empty());
    assert!(s.local_schedules.is_empty());
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Call(ProviderCall::Schedule { .. })))
    );
}

#[test]
fn huge_relative_presets_are_refused_not_panics() {
    let now = madrid(civil::date(2026, 11, 16).at(18, 0, 0, 0));
    for p in [
        "in 99999999999h",
        "in 9223372036854775807m",
        "in 9223372036854775807d",
        "in 99999999999999d",
    ] {
        assert!(parse_preset(p, &now).is_none(), "{p}");
    }
}

#[test]
fn busy_send_cancels_the_wake_up_until_it_finishes() {
    let mut s = state();
    reduce(
        &mut s,
        Input::LocalSchedulesLoaded(vec![
            local("a", 100, LocalScheduleState::Pending),
            local("b", 150, LocalScheduleState::Pending),
        ]),
    );
    let fx = reduce(&mut s, Input::LocalTick { now: 200 });
    assert_eq!(sent(&fx), ["a"]);
    // "b" is overdue but "a" is still sending: no timer, no busy loop.
    assert_eq!(wake(&fx), Some(None));
    let fx = reduce(&mut s, Input::LocalTick { now: 201 });
    assert_eq!(wake(&fx), Some(None));
    // Once "a" is done the timer is armed again for "b".
    let fx = reduce(
        &mut s,
        Input::LocalSent {
            id: sid("a"),
            result: Ok(()),
        },
    );
    assert_eq!(wake(&fx), Some(Some(150)));
}

fn state_with_profile_tz(tz: Option<&str>) -> WorkspaceState {
    let mut st = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    let me = uid("U1");
    st.self_user = Some(me.clone());
    st.profiles.insert(
        me.clone(),
        domain::Profile {
            user: me,
            display_name: "me".into(),
            real_name: "Me".into(),
            title: None,
            pronouns: None,
            tz: tz.map(Arc::from),
            status: None,
            email: None,
            phone: None,
            fields: Vec::new(),
            image: None,
            is_bot: false,
            deleted: false,
        },
    );
    st
}

#[test]
fn send_later_zone_is_the_profile_zone_with_a_settings_override() {
    let utc = TimeZone::UTC;
    let st = state_with_profile_tz(Some("Europe/Madrid"));
    // Profile zone, not the fallback (the fixture used to show UTC).
    let z = state_schedule_zone(&st, None, utc.clone());
    assert_eq!(z.iana_name(), Some("Europe/Madrid"));
    // The settings override wins over the profile.
    let z = state_schedule_zone(&st, Some("Asia/Tokyo"), utc.clone());
    assert_eq!(z.iana_name(), Some("Asia/Tokyo"));
    // An unknown override falls through to the profile.
    let z = state_schedule_zone(&st, Some("Not/AZone"), utc.clone());
    assert_eq!(z.iana_name(), Some("Europe/Madrid"));
    // No profile zone: the fallback.
    let st = state_with_profile_tz(None);
    assert_eq!(state_schedule_zone(&st, None, utc.clone()), utc);
    // No profile loaded at all: the fallback too.
    let bare = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    assert_eq!(state_schedule_zone(&bare, None, utc.clone()), utc);
}

#[test]
fn preset_in_the_spring_forward_gap_lands_after_the_gap() {
    // Madrid skips 02:00..03:00 on 2026-03-29, so 02:30 does not exist: it
    // reads as 03:30 CEST (01:30 UTC), never earlier than asked.
    let now = madrid(civil::date(2026, 3, 28).at(10, 0, 0, 0));
    let t = parse_preset("tomorrow 02:30", &now).unwrap();
    assert_eq!(t, "2026-03-29T01:30:00Z".parse::<Timestamp>().unwrap());
    let z = t.to_zoned_datetime(TimeZone::get("Europe/Madrid").unwrap());
    assert_eq!((z.hour(), z.minute()), (3, 30));
}

#[test]
fn preset_in_the_fall_back_fold_takes_the_first_occurrence() {
    // 02:30 happens twice on 2026-10-25 in Madrid (CEST, then CET): the first.
    let now = madrid(civil::date(2026, 10, 24).at(10, 0, 0, 0));
    let t = parse_preset("tomorrow 02:30", &now).unwrap();
    assert_eq!(t, "2026-10-25T00:30:00Z".parse::<Timestamp>().unwrap());
    // The relative preset is real elapsed time: 24 h after 02:30 CEST is still
    // the 25th, not the 26th.
    let at_fold = madrid(civil::date(2026, 10, 25).at(0, 30, 0, 0));
    let t = parse_preset("in 24h", &at_fold).unwrap();
    assert_eq!(
        t,
        at_fold.timestamp() + jiff::SignedDuration::from_hours(24)
    );
}
