//! Parla-side notification extras per conversation: thread replies, sound and
//! a timed mute (a stored deadline, one timer, no polling).

use std::collections::HashMap;
use std::sync::Arc;

use domain::notify::{NotifyContext, decide};
use domain::test_support::{cid, conv, msg, uid};
use domain::{
    Changed, ConversationPrefs, Effect, Input, Mention, Message, MessageTs, NotifyExtras,
    NotifyLevel, PersistOp, WorkspaceState, next_mute_deadline, reduce,
};
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
    m: &Message,
    prefs: Option<&ConversationPrefs>,
    extras: Option<&NotifyExtras>,
    followed: bool,
    started: bool,
) -> Option<domain::notify::Notification> {
    let me = uid("u1");
    let ctx = NotifyContext {
        settings: s,
        self_user: &me,
        now: jiff::civil::time(12, 0, 0, 0),
        now_unix: NOW,
        weekday: jiff::civil::Weekday::Monday,
        dnd: None,
        prefs,
        extras,
        thread_followed: followed,
        thread_started_by_me: started,
        focused: None,
        window_active: false,
        user_groups: None,
    };
    decide(&ctx, &ws(), &conv("c1", "gen"), m, "Ann")
}

fn reply(text: &str) -> Message {
    let mut m = msg("c1", "2.2", text);
    m.thread_root = Some(MessageTs("1.1".into()));
    m
}

fn at(until: i64) -> NotifyExtras {
    NotifyExtras {
        mute_until: Some(until),
        ..Default::default()
    }
}

#[test]
fn followed_thread_reply_notifies_by_default() {
    let s = settings(DesktopNotifications::Mentions);
    assert!(run(&s, &reply("hm"), None, None, true, false).is_some());
    // A thread the user does not follow stays quiet unless it mentions them.
    assert!(run(&s, &reply("hm"), None, None, false, false).is_none());
}

#[test]
fn thread_replies_off_keeps_only_mentions() {
    let s = settings(DesktopNotifications::Mentions);
    let off = NotifyExtras {
        threads_off: true,
        ..Default::default()
    };
    assert!(run(&s, &reply("hm"), None, Some(&off), true, false).is_none());
    let mut m = reply("hey @me");
    m.mentions = Arc::from([Mention::User(uid("u1"))]);
    assert!(run(&s, &m, None, Some(&off), true, false).is_some());
}

#[test]
fn thread_replies_off_still_notifies_in_a_thread_you_started() {
    let s = settings(DesktopNotifications::Mentions);
    let off = NotifyExtras {
        threads_off: true,
        ..Default::default()
    };
    assert!(run(&s, &reply("hm"), None, Some(&off), true, true).is_some());
    // Not started by the user: still quiet.
    assert!(run(&s, &reply("hm"), None, Some(&off), true, false).is_none());
}

#[test]
fn level_nothing_and_mute_still_beat_thread_replies() {
    let s = settings(DesktopNotifications::Mentions);
    let nothing = ConversationPrefs {
        level: NotifyLevel::Nothing,
        muted: false,
    };
    assert!(run(&s, &reply("hm"), Some(&nothing), None, true, false).is_none());
    let muted = ConversationPrefs {
        level: NotifyLevel::Default,
        muted: true,
    };
    assert!(run(&s, &reply("hm"), Some(&muted), None, true, false).is_none());
}

#[test]
fn sound_off_makes_the_notification_silent() {
    let s = settings(DesktopNotifications::All);
    let m = msg("c1", "1.1", "hi");
    assert!(!run(&s, &m, None, None, false, false).unwrap().silent);
    let off = NotifyExtras {
        sound_off: true,
        ..Default::default()
    };
    assert!(run(&s, &m, None, Some(&off), false, false).unwrap().silent);
}

#[test]
fn timed_mute_holds_until_its_deadline() {
    let s = settings(DesktopNotifications::All);
    let m = msg("c1", "1.1", "hi");
    // Muted: only a direct mention gets through.
    assert!(run(&s, &m, None, Some(&at(NOW + 60)), false, false).is_none());
    // At the deadline the mute is over.
    assert!(run(&s, &m, None, Some(&at(NOW)), false, false).is_some());
    assert!(run(&s, &m, None, Some(&at(NOW - 1)), false, false).is_some());
}

#[test]
fn set_extras_persists_and_default_removes() {
    let mut st = WorkspaceState::new(ws(), 100);
    let e = at(NOW + 3600);
    let fx = reduce(
        &mut st,
        Input::SetNotifyExtras {
            conversation: cid("c1"),
            extras: e,
        },
    );
    assert_eq!(st.local_notify_extras.get(&cid("c1")), Some(&e));
    assert!(
        fx.iter()
            .any(|f| matches!(f, Effect::Changed(Changed::NotifyPrefs)))
    );
    assert!(fx.iter().any(|f| matches!(
        f,
        Effect::Persist(PersistOp::LocalNotifyExtrasPut { extras, .. }) if *extras == e
    )));
    // The same again changes nothing.
    let fx = reduce(
        &mut st,
        Input::SetNotifyExtras {
            conversation: cid("c1"),
            extras: e,
        },
    );
    assert!(fx.is_empty());
    reduce(
        &mut st,
        Input::SetNotifyExtras {
            conversation: cid("c1"),
            extras: NotifyExtras::default(),
        },
    );
    assert!(st.local_notify_extras.is_empty());
}

#[test]
fn loaded_extras_replace_the_map() {
    let mut st = WorkspaceState::new(ws(), 100);
    let all = HashMap::from([(cid("c1"), at(NOW + 5))]);
    reduce(&mut st, Input::NotifyExtrasLoaded(all.clone()));
    assert_eq!(st.local_notify_extras, all);
}

#[test]
fn next_deadline_is_the_earliest_and_the_tick_clears_expired() {
    let mut st = WorkspaceState::new(ws(), 100);
    assert_eq!(next_mute_deadline(&st), None);
    st.local_notify_extras.insert(cid("a"), at(NOW + 100));
    st.local_notify_extras.insert(
        cid("b"),
        NotifyExtras {
            sound_off: true,
            mute_until: Some(NOW + 10),
            ..Default::default()
        },
    );
    assert_eq!(next_mute_deadline(&st), Some(NOW + 10));
    let fx = reduce(&mut st, Input::NotifyMuteTick { now: NOW + 10 });
    assert!(
        fx.iter()
            .any(|f| matches!(f, Effect::Changed(Changed::NotifyPrefs)))
    );
    // b keeps its sound choice, a keeps its deadline.
    assert_eq!(
        st.local_notify_extras.get(&cid("b")),
        Some(&NotifyExtras {
            sound_off: true,
            ..Default::default()
        })
    );
    assert_eq!(next_mute_deadline(&st), Some(NOW + 100));
    let fx = reduce(&mut st, Input::NotifyMuteTick { now: NOW + 100 });
    assert!(fx.iter().any(|f| matches!(
        f,
        Effect::Persist(PersistOp::LocalNotifyExtrasPut { conversation, .. }) if *conversation == cid("a")
    )));
    assert!(!st.local_notify_extras.contains_key(&cid("a")));
    // Nothing expired: nothing changes.
    let fx = reduce(&mut st, Input::NotifyMuteTick { now: NOW + 101 });
    assert!(fx.is_empty());
}
