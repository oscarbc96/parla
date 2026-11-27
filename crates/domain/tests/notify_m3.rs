//! the M3 notification decision (self DND, per-conversation
//! levels, optimistic snooze and prefs writes, and the DND end).

use std::collections::HashMap;
use std::sync::Arc;

use domain::notify::{NotifyContext, decide};
use domain::test_support::{cid, conv, msg, uid};
use domain::{
    Bootstrap, CallOutput, Changed, ConversationPrefs, DndDay, DndStatus, Effect, Input, Load,
    Loaded, Mention, Message, NotifyLevel, ProviderCall, ProviderError, ProviderEvent,
    WorkspaceState, dnd_ends, reduce,
};
use jiff::tz::TimeZone;
use settings::{DesktopNotifications, NotificationSettings, WorkspaceKey};

const NOW: i64 = 1_700_000_000;

fn ws() -> WorkspaceKey {
    WorkspaceKey::new("fake", "w1")
}
fn settings(d: DesktopNotifications) -> NotificationSettings {
    NotificationSettings {
        desktop: d,
        ..Default::default()
    }
}
fn run(
    s: &NotificationSettings,
    c: &domain::Conversation,
    m: &Message,
    dnd: Option<&DndStatus>,
    prefs: Option<&ConversationPrefs>,
) -> Option<domain::notify::Notification> {
    let me = uid("u1");
    let ctx = NotifyContext {
        settings: s,
        self_user: &me,
        now: jiff::civil::time(12, 0, 0, 0),
        now_unix: NOW,
        weekday: jiff::civil::Weekday::Monday,
        dnd,
        prefs,
        extras: None,
        thread_followed: false,
        thread_started_by_me: false,
        focused: None,
        window_active: false,
        user_groups: None,
    };
    decide(&ctx, &ws(), c, m, "Ann")
}
fn mention(mut m: Message, u: &str) -> Message {
    m.mentions = Arc::from([Mention::User(uid(u))]);
    m
}
fn dm() -> domain::Conversation {
    let mut c = conv("d1", "ann");
    c.kind = domain::ConversationKind::Direct;
    c
}
fn snoozed(until: i64) -> DndStatus {
    DndStatus {
        snooze_until: Some(until),
        ..Default::default()
    }
}

#[test]
fn dnd_active_suppresses() {
    let s = settings(DesktopNotifications::All);
    // A DM notifies otherwise.
    assert!(run(&s, &dm(), &msg("d1", "1.1", "hi"), None, None).is_some());
    // Snoozed DND suppresses everything.
    assert!(
        run(
            &s,
            &dm(),
            &msg("d1", "1.1", "hi"),
            Some(&snoozed(NOW + 60)),
            None
        )
        .is_none()
    );
}

#[test]
fn dnd_schedule_window_suppresses_only_inside() {
    let s = settings(DesktopNotifications::All);
    let m = msg("d1", "1.1", "hi");
    let d = DndStatus {
        enabled: true,
        next_start: Some(NOW + 60),
        next_end: Some(NOW + 3600),
        snooze_until: None,
    };
    // Before the window the notification still goes out.
    assert!(run(&s, &dm(), &m, Some(&d), None).is_some());
    let d = DndStatus {
        enabled: true,
        next_start: Some(NOW - 60),
        next_end: Some(NOW + 3600),
        snooze_until: None,
    };
    assert!(run(&s, &dm(), &m, Some(&d), None).is_none());
    // At `next_end` the window is over (`active` is half-open).
    let d = DndStatus {
        enabled: true,
        next_start: Some(NOW - 3600),
        next_end: Some(NOW),
        snooze_until: None,
    };
    assert!(run(&s, &dm(), &m, Some(&d), None).is_some());
}

#[test]
fn level_nothing_suppresses_dm() {
    let s = settings(DesktopNotifications::All);
    let prefs = ConversationPrefs {
        level: NotifyLevel::Nothing,
        muted: false,
    };
    assert!(run(&s, &dm(), &msg("d1", "1.1", "hi"), None, Some(&prefs)).is_none());
    let m = mention(msg("c1", "1.1", "yo"), "u1");
    assert!(run(&s, &conv("c1", "gen"), &m, None, Some(&prefs)).is_none());
}

#[test]
fn level_all_overrides_mentions_setting() {
    let s = settings(DesktopNotifications::Mentions);
    let prefs = ConversationPrefs {
        level: NotifyLevel::All,
        muted: false,
    };
    // A plain channel post notifies even though the setting is Mentions.
    assert!(
        run(
            &s,
            &conv("c1", "gen"),
            &msg("c1", "1.1", "hi"),
            None,
            Some(&prefs)
        )
        .is_some()
    );
    // Default keeps the setting: still mentions-only.
    let prefs = ConversationPrefs {
        level: NotifyLevel::Default,
        muted: false,
    };
    assert!(
        run(
            &s,
            &conv("c1", "gen"),
            &msg("c1", "1.1", "hi"),
            None,
            Some(&prefs)
        )
        .is_none()
    );
}

#[test]
fn prefs_muted_overrides_conversation() {
    let s = settings(DesktopNotifications::All);
    let muted = ConversationPrefs {
        muted: true,
        ..Default::default()
    };
    // Not muted on the conversation, muted by prefs.
    assert!(
        run(
            &s,
            &conv("c1", "gen"),
            &msg("c1", "1.1", "hi"),
            None,
            Some(&muted)
        )
        .is_none()
    );
    let m = mention(msg("c1", "1.1", "yo"), "u1");
    assert!(run(&s, &conv("c1", "gen"), &m, None, Some(&muted)).is_some());
    // Muted on the conversation, unmuted by prefs.
    let unmuted = ConversationPrefs::default();
    let mut ch = conv("c1", "gen");
    ch.is_muted = true;
    assert!(run(&s, &ch, &msg("c1", "1.1", "hi"), None, Some(&unmuted)).is_some());
}

fn dnd_of(st: &WorkspaceState, u: &str) -> DndStatus {
    st.dnd.get(&uid(u)).cloned().unwrap_or_default()
}

fn bootstrap() -> Bootstrap {
    Bootstrap {
        self_user: uid("u1"),
        workspace_name: Arc::from("W"),
        conversations: vec![],
        members: vec![],
    }
}

#[test]
fn bootstrap_loads_dnd_and_prefs_when_capable() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.caps.dnd = true;
    st.caps.notify_prefs = true;
    let fx = reduce(&mut st, Input::Bootstrapped(bootstrap()));
    assert!(fx.iter().any(|e| matches!(e, Effect::Load(Load::Dnd))));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Load(Load::NotifyPrefs)))
    );
    // Without the capabilities nothing is loaded.
    let mut st = WorkspaceState::new(ws(), 100);
    let fx = reduce(&mut st, Input::Bootstrapped(bootstrap()));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Load(_))));
    // The answers land in the state.
    let mut st = WorkspaceState::new(ws(), 100);
    st.self_user = Some(uid("u1"));
    let d = DndStatus {
        enabled: true,
        ..Default::default()
    };
    reduce(&mut st, Input::Loaded(Ok(Loaded::Dnd(d))));
    assert!(dnd_of(&st, "u1").enabled);
    let mut prefs = HashMap::new();
    prefs.insert(cid("c1"), ConversationPrefs::default());
    let fx = reduce(&mut st, Input::Loaded(Ok(Loaded::NotifyPrefs(prefs))));
    assert!(st.notify_prefs.contains_key(&cid("c1")));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::NotifyPrefs)))
    );
    let mut users = HashMap::new();
    users.insert(uid("u2"), snoozed(NOW + 10));
    reduce(&mut st, Input::Loaded(Ok(Loaded::DndUsers(users))));
    assert_eq!(dnd_of(&st, "u2").snooze_until, Some(NOW + 10));
}

#[test]
fn events_update_dnd_and_prefs() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.self_user = Some(uid("u1"));
    let d = DndStatus {
        enabled: true,
        ..Default::default()
    };
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::DndChanged {
            user: uid("u2"),
            status: d.clone(),
        }),
    );
    assert_eq!(dnd_of(&st, "u2"), d);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Dnd)))
    );
    let mut prefs = HashMap::new();
    prefs.insert(cid("c1"), ConversationPrefs::default());
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::NotifyPrefsChanged(prefs)),
    );
    assert!(st.notify_prefs.contains_key(&cid("c1")));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::NotifyPrefs)))
    );
}

#[test]
fn none_answer_refetches_dnd() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.self_user = Some(uid("u1"));
    reduce(
        &mut st,
        Input::Snooze {
            minutes: 30,
            now_unix: NOW,
        },
    );
    // No authoritative value: ask for the current one.
    let fx = reduce(
        &mut st,
        Input::CallFinished {
            call: ProviderCall::Snooze(30),
            result: Ok(CallOutput::None),
        },
    );
    assert!(fx.iter().any(|e| matches!(e, Effect::Load(Load::Dnd))));
}

#[test]
fn dnd_users_and_prefs_events_emit_changed_only_on_change() {
    let mut st = WorkspaceState::new(ws(), 100);
    let d = DndStatus {
        enabled: true,
        ..Default::default()
    };
    st.dnd.insert(uid("u2"), d.clone());
    // The same status again changes nothing.
    let fx = reduce(
        &mut st,
        Input::Loaded(Ok(Loaded::DndUsers(HashMap::from([(uid("u2"), d)])))),
    );
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Dnd)))
    );
    // A real change does notify.
    let other = DndStatus::default();
    let fx = reduce(
        &mut st,
        Input::Loaded(Ok(Loaded::DndUsers(HashMap::from([(uid("u2"), other)])))),
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Dnd)))
    );
    // The same prefs again change nothing.
    st.notify_prefs
        .insert(cid("c1"), ConversationPrefs::default());
    let same = HashMap::from([(cid("c1"), ConversationPrefs::default())]);
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::NotifyPrefsChanged(same)),
    );
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::NotifyPrefs)))
    );
    // A different level does notify.
    let changed = HashMap::from([(
        cid("c1"),
        ConversationPrefs {
            level: NotifyLevel::All,
            muted: false,
        },
    )]);
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::NotifyPrefsChanged(changed)),
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::NotifyPrefs)))
    );
}

#[test]
fn dnd_ends_is_structured_and_only_when_active() {
    let utc = TimeZone::UTC;
    let at = |secs: i64| {
        jiff::Timestamp::from_second(secs)
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC)
    };
    // 1970-01-01T00:00Z is "now": 14:30 the same day.
    let e = dnd_ends(&snoozed(52_200), 0, &utc).unwrap();
    assert_eq!(e.at, at(52_200));
    assert_eq!(e.day, DndDay::Today);
    // Tomorrow 09:00.
    let e = dnd_ends(&snoozed(118_800), 0, &utc).unwrap();
    assert_eq!(e.day, DndDay::Tomorrow);
    // A later day keeps its date: 1970-01-05T09:00Z.
    let e = dnd_ends(&snoozed(378_000), 0, &utc).unwrap();
    assert_eq!(e.day, DndDay::On(jiff::civil::date(1970, 1, 5)));
    // A schedule window that already started ends at `next_end`.
    let window = DndStatus {
        enabled: true,
        next_start: Some(0),
        next_end: Some(52_200),
        snooze_until: None,
    };
    assert_eq!(dnd_ends(&window, 0, &utc).unwrap().day, DndDay::Today);
    // A future window is not active now.
    let future = DndStatus {
        enabled: true,
        next_start: Some(1),
        next_end: Some(52_200),
        snooze_until: None,
    };
    assert_eq!(dnd_ends(&future, 0, &utc), None);
    // `enabled = false` is not active.
    let off = DndStatus {
        enabled: false,
        next_start: Some(0),
        next_end: Some(52_200),
        snooze_until: None,
    };
    assert_eq!(dnd_ends(&off, 0, &utc), None);
    // Nothing to show.
    assert_eq!(dnd_ends(&snoozed(-1), 0, &utc), None);
    assert_eq!(dnd_ends(&DndStatus::default(), 0, &utc), None);
    // Both apply: the later end wins.
    let both = DndStatus {
        enabled: true,
        next_start: Some(0),
        next_end: Some(118_800),
        snooze_until: Some(52_200),
    };
    let e = dnd_ends(&both, 0, &utc).unwrap();
    assert_eq!(e.at, at(118_800));
    assert_eq!(e.day, DndDay::Tomorrow);
}

fn fail(call: ProviderCall) -> Input {
    Input::CallFinished {
        call,
        result: Err(ProviderError::Other("offline".into())),
    }
}

fn ok(call: ProviderCall, out: CallOutput) -> Input {
    Input::CallFinished {
        call,
        result: Ok(out),
    }
}

fn has_load(fx: &[Effect], l: Load) -> bool {
    fx.iter().any(|e| {
        matches!(
            (e, &l),
            (Effect::Load(Load::Dnd), Load::Dnd)
                | (Effect::Load(Load::NotifyPrefs), Load::NotifyPrefs)
        )
    })
}

fn changed_dnd(fx: &[Effect]) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Changed(Changed::Dnd)))
}

fn changed_prefs(fx: &[Effect]) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Changed(Changed::NotifyPrefs)))
}

fn snooze_in(min: u32) -> Input {
    Input::Snooze {
        minutes: min,
        now_unix: NOW,
    }
}

#[test]
fn snooze_is_optimistic_and_a_failure_refetches() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.self_user = Some(uid("u1"));
    let fx = reduce(&mut st, snooze_in(30));
    let d = dnd_of(&st, "u1");
    assert_eq!(d.snooze_until, Some(NOW + 1800));
    assert!(d.active(NOW));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Call(ProviderCall::Snooze(30))))
    );
    assert!(changed_dnd(&fx));
    // A failure asks for the truth instead of restoring a stored prior.
    let fx = reduce(&mut st, fail(ProviderCall::Snooze(30)));
    assert!(has_load(&fx, Load::Dnd));
    // The refetched value lands.
    reduce(
        &mut st,
        Input::Loaded(Ok(Loaded::Dnd(DndStatus::default()))),
    );
    assert_eq!(dnd_of(&st, "u1"), DndStatus::default());
}

#[test]
fn lone_snooze_success_takes_the_provider_answer() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.self_user = Some(uid("u1"));
    reduce(&mut st, snooze_in(5));
    let answer = DndStatus {
        enabled: false,
        next_start: None,
        next_end: None,
        snooze_until: Some(NOW + 999),
    };
    let fx = reduce(
        &mut st,
        ok(ProviderCall::Snooze(5), CallOutput::Dnd(answer.clone())),
    );
    assert_eq!(dnd_of(&st, "u1"), answer);
    assert!(!has_load(&fx, Load::Dnd));
}

#[test]
fn dnd_answers_out_of_order_keep_the_latest_write() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.self_user = Some(uid("u1"));
    st.dnd.insert(uid("u1"), snoozed(NOW + 600));
    reduce(&mut st, snooze_in(5));
    reduce(&mut st, Input::EndSnooze);
    assert_eq!(dnd_of(&st, "u1").snooze_until, None);
    // EndSnooze answers first, then the older Snooze answer: still ended,
    // and no refetch because both succeeded.
    let fx = reduce(
        &mut st,
        ok(
            ProviderCall::EndSnooze,
            CallOutput::Dnd(DndStatus::default()),
        ),
    );
    assert!(!has_load(&fx, Load::Dnd));
    let fx = reduce(
        &mut st,
        ok(ProviderCall::Snooze(5), CallOutput::Dnd(snoozed(NOW + 300))),
    );
    assert!(!has_load(&fx, Load::Dnd));
    assert_eq!(dnd_of(&st, "u1").snooze_until, None);
}

#[test]
fn failed_snooze_waits_for_the_other_write_before_refetching() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.self_user = Some(uid("u1"));
    reduce(&mut st, snooze_in(5));
    reduce(&mut st, Input::EndSnooze);
    let fx = reduce(&mut st, fail(ProviderCall::Snooze(5)));
    assert!(!has_load(&fx, Load::Dnd));
    assert_eq!(dnd_of(&st, "u1").snooze_until, None);
    let fx = reduce(
        &mut st,
        ok(
            ProviderCall::EndSnooze,
            CallOutput::Dnd(DndStatus::default()),
        ),
    );
    assert!(has_load(&fx, Load::Dnd));
    // The failure was consumed: a later clean write does not refetch.
    reduce(&mut st, snooze_in(5));
    let fx = reduce(
        &mut st,
        ok(ProviderCall::Snooze(5), CallOutput::Dnd(snoozed(NOW + 300))),
    );
    assert!(!has_load(&fx, Load::Dnd));
}

#[test]
fn end_snooze_failure_refetches() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.self_user = Some(uid("u1"));
    st.dnd.insert(uid("u1"), snoozed(NOW + 60));
    let fx = reduce(&mut st, Input::EndSnooze);
    assert_eq!(dnd_of(&st, "u1").snooze_until, None);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Call(ProviderCall::EndSnooze)))
    );
    let fx = reduce(&mut st, fail(ProviderCall::EndSnooze));
    assert!(has_load(&fx, Load::Dnd));
}

#[test]
fn noop_dnd_writes_emit_no_changed() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.self_user = Some(uid("u1"));
    st.dnd.insert(uid("u1"), snoozed(NOW + 1800));
    // Same snooze_until again.
    let fx = reduce(&mut st, snooze_in(30));
    assert!(!changed_dnd(&fx));
    // Ending a snooze that is not there.
    st.dnd.insert(uid("u1"), DndStatus::default());
    let fx = reduce(&mut st, Input::EndSnooze);
    assert!(!changed_dnd(&fx));
}

#[test]
fn snooze_without_a_self_user_sends_nothing() {
    let mut st = WorkspaceState::new(ws(), 100);
    let fx = reduce(&mut st, snooze_in(30));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Call(_))));
    let fx = reduce(&mut st, Input::EndSnooze);
    assert!(!fx.iter().any(|e| matches!(e, Effect::Call(_))));
}

fn prefs_nothing() -> ConversationPrefs {
    ConversationPrefs {
        level: NotifyLevel::Nothing,
        muted: true,
    }
}

fn set_prefs(c: &str, prefs: ConversationPrefs) -> Input {
    Input::SetNotifyPrefs {
        conversation: cid(c),
        prefs,
    }
}

fn prefs_call(c: &str, prefs: ConversationPrefs) -> ProviderCall {
    ProviderCall::SetNotifyPrefs {
        conversation: cid(c),
        prefs,
    }
}

#[test]
fn set_prefs_is_optimistic_and_a_failure_refetches() {
    let mut st = WorkspaceState::new(ws(), 100);
    let prefs = prefs_nothing();
    let fx = reduce(&mut st, set_prefs("c1", prefs));
    assert_eq!(st.notify_prefs.get(&cid("c1")), Some(&prefs));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Call(ProviderCall::SetNotifyPrefs { .. })))
    );
    assert!(changed_prefs(&fx));
    let fx = reduce(&mut st, fail(prefs_call("c1", prefs)));
    assert!(has_load(&fx, Load::NotifyPrefs));
}

#[test]
fn two_prefs_writes_to_one_conversation_refetch_after_both_answer() {
    let mut st = WorkspaceState::new(ws(), 100);
    let a = prefs_nothing();
    let b = ConversationPrefs {
        level: NotifyLevel::All,
        muted: false,
    };
    reduce(&mut st, set_prefs("c1", a));
    reduce(&mut st, set_prefs("c1", b));
    // A fails while B is still in flight: no refetch yet, B's value stands.
    let fx = reduce(&mut st, fail(prefs_call("c1", a)));
    assert!(!has_load(&fx, Load::NotifyPrefs));
    assert_eq!(st.notify_prefs.get(&cid("c1")), Some(&b));
    // B answers: now the truth is fetched.
    let fx = reduce(&mut st, ok(prefs_call("c1", b), CallOutput::None));
    assert!(has_load(&fx, Load::NotifyPrefs));
}

#[test]
fn prefs_failures_are_tracked_per_conversation() {
    let mut st = WorkspaceState::new(ws(), 100);
    let a = prefs_nothing();
    reduce(&mut st, set_prefs("c1", a));
    reduce(&mut st, set_prefs("c2", a));
    // c1's failure does not wait for c2.
    let fx = reduce(&mut st, fail(prefs_call("c1", a)));
    assert!(has_load(&fx, Load::NotifyPrefs));
    let fx = reduce(&mut st, ok(prefs_call("c2", a), CallOutput::None));
    assert!(!has_load(&fx, Load::NotifyPrefs));
}

#[test]
fn noop_prefs_write_emits_no_changed() {
    let mut st = WorkspaceState::new(ws(), 100);
    let a = prefs_nothing();
    st.notify_prefs.insert(cid("c1"), a);
    let fx = reduce(&mut st, set_prefs("c1", a));
    assert!(!changed_prefs(&fx));
}

#[test]
fn loaded_prefs_keep_the_optimistic_value_while_a_write_is_in_flight() {
    let mut st = WorkspaceState::new(ws(), 100);
    let a = prefs_nothing();
    reduce(&mut st, set_prefs("c1", a));
    // A stale snapshot without c1 (and with c2) arrives mid-write.
    let c2 = ConversationPrefs {
        level: NotifyLevel::All,
        muted: false,
    };
    reduce(
        &mut st,
        Input::Loaded(Ok(Loaded::NotifyPrefs(HashMap::from([(cid("c2"), c2)])))),
    );
    assert_eq!(st.notify_prefs.get(&cid("c1")), Some(&a));
    assert_eq!(st.notify_prefs.get(&cid("c2")), Some(&c2));
    // Once the write is answered a snapshot is taken as is.
    reduce(&mut st, ok(prefs_call("c1", a), CallOutput::None));
    reduce(
        &mut st,
        Input::Loaded(Ok(Loaded::NotifyPrefs(HashMap::new()))),
    );
    assert!(st.notify_prefs.is_empty());
}

#[test]
fn dnd_active_suppresses_even_with_level_all() {
    let s = settings(DesktopNotifications::All);
    let prefs = ConversationPrefs {
        level: NotifyLevel::All,
        muted: false,
    };
    let d = snoozed(NOW + 60);
    let m = mention(msg("c1", "1.1", "yo"), "u1");
    assert!(run(&s, &conv("c1", "gen"), &m, Some(&d), Some(&prefs)).is_none());
}

#[test]
fn prefs_changed_event_is_a_snapshot_that_keeps_in_flight_writes() {
    let mut st = WorkspaceState::new(ws(), 100);
    let a = prefs_nothing();
    reduce(&mut st, set_prefs("c1", a));
    let c2 = ConversationPrefs {
        level: NotifyLevel::All,
        muted: false,
    };
    st.notify_prefs.insert(cid("c3"), c2);
    // A snapshot without c3 (unmuted elsewhere) drops it; c1 keeps its optimistic value.
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::NotifyPrefsChanged(HashMap::from([(
            cid("c2"),
            c2,
        )]))),
    );
    assert_eq!(st.notify_prefs.get(&cid("c1")), Some(&a));
    assert_eq!(st.notify_prefs.get(&cid("c2")), Some(&c2));
    assert!(!st.notify_prefs.contains_key(&cid("c3")));
    assert!(changed_prefs(&fx));
}

// Providers without `notify_prefs` keep the levels in Parla (the store).

#[test]
fn local_prefs_apply_persist_and_send_no_call() {
    let mut st = WorkspaceState::new(ws(), 100);
    let prefs = prefs_nothing();
    let input = Input::SetLocalNotifyPrefs {
        conversation: cid("c1"),
        prefs,
    };
    let fx = reduce(&mut st, input.clone());
    assert_eq!(st.local_notify_prefs.get(&cid("c1")), Some(&prefs));
    assert!(changed_prefs(&fx));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Persist(domain::PersistOp::LocalNotifyPrefsPut { prefs: p, .. }) if *p == prefs
    )));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Call(_))));
    // The same write again changes nothing.
    let fx = reduce(&mut st, input);
    assert!(!changed_prefs(&fx));
    // Back to the default removes the entry.
    reduce(
        &mut st,
        Input::SetLocalNotifyPrefs {
            conversation: cid("c1"),
            prefs: ConversationPrefs::default(),
        },
    );
    assert!(st.local_notify_prefs.is_empty());
}

#[test]
fn local_prefs_loaded_fill_state() {
    let mut st = WorkspaceState::new(ws(), 100);
    let all = HashMap::from([(cid("c1"), prefs_nothing())]);
    let fx = reduce(&mut st, Input::LocalNotifyPrefsLoaded(all.clone()));
    assert_eq!(st.local_notify_prefs, all);
    assert!(changed_prefs(&fx));
}

#[test]
fn prefs_for_follows_the_capability() {
    let mut st = WorkspaceState::new(ws(), 100);
    let server = ConversationPrefs {
        level: NotifyLevel::All,
        muted: false,
    };
    st.notify_prefs.insert(cid("c1"), server);
    st.local_notify_prefs.insert(cid("c1"), prefs_nothing());
    // Without the capability only Parla's own levels count.
    assert_eq!(st.prefs_for(&cid("c1")), Some(&prefs_nothing()));
    st.caps.notify_prefs = true;
    assert_eq!(st.prefs_for(&cid("c1")), Some(&server));
    assert_eq!(st.prefs_for(&cid("zz")), None);
}
