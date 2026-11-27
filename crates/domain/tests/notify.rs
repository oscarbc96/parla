use std::sync::Arc;

use domain::notify::{NotifyContext, decide};
use domain::test_support::{cid, conv, mid, msg, uid};
use domain::{
    Conversation, ConversationKind, Effect, FileId, HistoryPage, Input, Mention, Message,
    ProviderEvent, WorkspaceState, reduce,
};
use settings::{DesktopNotifications, NotificationSettings, QuietHours, WorkspaceKey};

fn ws() -> WorkspaceKey {
    WorkspaceKey::new("fake", "w1")
}
fn settings(d: DesktopNotifications) -> NotificationSettings {
    NotificationSettings {
        desktop: d,
        ..Default::default()
    }
}
fn at(h: i8, m: i8) -> jiff::civil::Time {
    jiff::civil::time(h, m, 0, 0)
}
fn run(
    s: &NotificationSettings,
    c: &Conversation,
    m: &Message,
    focused: Option<&domain::ConversationId>,
    active: bool,
    now: jiff::civil::Time,
) -> Option<domain::notify::Notification> {
    run_on(s, c, m, focused, active, now, jiff::civil::Weekday::Monday)
}
fn run_on(
    s: &NotificationSettings,
    c: &Conversation,
    m: &Message,
    focused: Option<&domain::ConversationId>,
    active: bool,
    now: jiff::civil::Time,
    weekday: jiff::civil::Weekday,
) -> Option<domain::notify::Notification> {
    let me = uid("u1");
    let ctx = NotifyContext {
        settings: s,
        self_user: &me,
        now,
        now_unix: 0,
        weekday,
        dnd: None,
        prefs: None,
        extras: None,
        thread_followed: false,
        thread_started_by_me: false,
        focused,
        window_active: active,
        user_groups: None,
    };
    decide(&ctx, &ws(), c, m, "Ann")
}
fn mention(mut m: Message, u: &str) -> Message {
    m.mentions = Arc::from([Mention::User(uid(u))]);
    m
}
fn dm() -> Conversation {
    let mut c = conv("d1", "ann");
    c.kind = ConversationKind::Direct;
    c
}

#[test]
fn none_never_notifies() {
    let s = settings(DesktopNotifications::None);
    let m = mention(msg("c1", "1.1", "hi"), "u1");
    assert!(run(&s, &conv("c1", "gen"), &m, None, false, at(12, 0)).is_none());
}

#[test]
fn own_message_never_notifies() {
    let s = settings(DesktopNotifications::All);
    let mut m = msg("c1", "1.1", "hi");
    m.author = Some(uid("u1"));
    assert!(run(&s, &conv("c1", "gen"), &m, None, false, at(12, 0)).is_none());
}

#[test]
fn focused_active_conversation_does_not_notify() {
    let s = settings(DesktopNotifications::All);
    let m = msg("c1", "1.1", "hi");
    let c = conv("c1", "gen");
    let f = cid("c1");
    assert!(run(&s, &c, &m, Some(&f), true, at(12, 0)).is_none());
    assert!(run(&s, &c, &m, Some(&f), false, at(12, 0)).is_some());
}

#[test]
fn mentions_mode_dm_mention_keyword() {
    let mut s = settings(DesktopNotifications::Mentions);
    s.keywords = vec!["deploy".into()];
    let ch = conv("c1", "gen");
    assert!(run(&s, &dm(), &msg("d1", "1.1", "hi"), None, false, at(12, 0)).is_some());
    assert!(run(&s, &ch, &msg("c1", "1.1", "hi"), None, false, at(12, 0)).is_none());
    assert!(
        run(
            &s,
            &ch,
            &msg("c1", "1.1", "Deploy done"),
            None,
            false,
            at(12, 0)
        )
        .is_some()
    );
    assert!(
        run(
            &s,
            &ch,
            &msg("c1", "1.1", "deployment"),
            None,
            false,
            at(12, 0)
        )
        .is_none()
    );
    let m = mention(msg("c1", "1.1", "yo"), "u1");
    assert!(run(&s, &ch, &m, None, false, at(12, 0)).is_some());
}

#[test]
fn all_mode_skips_thread_replies_without_mention() {
    let s = settings(DesktopNotifications::All);
    let ch = conv("c1", "gen");
    let mut r = msg("c1", "2.1", "reply");
    r.thread_root = Some(domain::test_support::ts("1.1"));
    assert!(run(&s, &ch, &r, None, false, at(12, 0)).is_none());
    let r = mention(r, "u1");
    assert!(run(&s, &ch, &r, None, false, at(12, 0)).is_some());
    assert!(run(&s, &ch, &msg("c1", "3.1", "top"), None, false, at(12, 0)).is_some());
}

#[test]
fn muted_only_direct_mention() {
    let s = settings(DesktopNotifications::All);
    let mut ch = conv("c1", "gen");
    ch.is_muted = true;
    assert!(run(&s, &ch, &msg("c1", "1.1", "hi"), None, false, at(12, 0)).is_none());
    let m = mention(msg("c1", "1.1", "hi"), "u1");
    assert!(run(&s, &ch, &m, None, false, at(12, 0)).is_some());
}

#[test]
fn quiet_hours_wrap_midnight() {
    let mut s = settings(DesktopNotifications::All);
    s.quiet_hours = Some(QuietHours {
        from: "20:00".into(),
        to: "08:00".into(),
        days: vec![],
    });
    let (c, m) = (conv("c1", "gen"), msg("c1", "1.1", "hi"));
    assert!(run(&s, &c, &m, None, false, at(23, 0)).is_none());
    assert!(run(&s, &c, &m, None, false, at(7, 59)).is_none());
    assert!(run(&s, &c, &m, None, false, at(8, 0)).is_some());
    s.quiet_hours = Some(QuietHours {
        from: "bad".into(),
        to: "08:00".into(),
        days: vec![],
    });
    assert!(run(&s, &c, &m, None, false, at(23, 0)).is_some());
}

#[test]
fn body_truncates_on_char_boundary() {
    let s = settings(DesktopNotifications::All);
    let text = "é".repeat(300);
    let n = run(
        &s,
        &conv("c1", "gen"),
        &msg("c1", "1.1", &text),
        None,
        false,
        at(12, 0),
    )
    .unwrap();
    assert_eq!(n.text.chars().count(), 200);
    assert_eq!(&*n.author, "Ann");
    assert_eq!(n.channel.as_deref(), Some("gen"));
    let mut f = msg("c1", "1.2", "");
    f.files = Arc::from(vec![domain::FileRef {
        id: FileId(Arc::from("f1")),
        name: Arc::from("a.png"),
        title: None,
        mime: Arc::from("image/png"),
        size: 1,
        kind: domain::FileKind::Image,
        thumbs: Arc::from([]),
        download: None,
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: Default::default(),
    }]);
    let n = run(&s, &conv("c1", "gen"), &f, None, false, at(12, 0)).unwrap();
    assert_eq!((&*n.text, n.file_count), ("", 1));
    let n = run(&s, &dm(), &msg("d1", "1.1", "x"), None, false, at(12, 0)).unwrap();
    assert_eq!((&*n.author, n.channel.as_deref()), ("Ann", None));
    assert!(!n.silent);
}

#[test]
fn only_live_posts_ask_to_notify() {
    let mut st = WorkspaceState::new(ws(), 100);
    st.conversations.insert(cid("c1"), conv("c1", "gen"));
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::MessagePosted(msg("c1", "1.1", "hi"))),
    );
    assert!(fx.iter().any(|e| matches!(e,
        Effect::MaybeNotify { conversation, message } if *conversation == cid("c1") && *message == mid("c1", "1.1"))));
    let fx = reduce(
        &mut st,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![msg("c1", "0.5", "old")],
                has_more: false,
            },
        },
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::MaybeNotify { .. })));
}

fn quiet(from: &str, to: &str, days: &[settings::Weekday]) -> NotificationSettings {
    let mut s = settings(DesktopNotifications::All);
    s.quiet_hours = Some(QuietHours {
        from: from.into(),
        to: to.into(),
        days: days.to_vec(),
    });
    s
}

#[test]
fn quiet_hours_days_same_day_window() {
    use jiff::civil::Weekday as W;
    let s = quiet(
        "09:00",
        "17:00",
        &[settings::Weekday::Mon, settings::Weekday::Tue],
    );
    let (c, m) = (conv("c1", "gen"), msg("c1", "1.1", "hi"));
    assert!(run_on(&s, &c, &m, None, false, at(10, 0), W::Monday).is_none());
    assert!(run_on(&s, &c, &m, None, false, at(10, 0), W::Wednesday).is_some());
    assert!(run_on(&s, &c, &m, None, false, at(18, 0), W::Monday).is_some());
}

#[test]
fn quiet_hours_days_across_midnight_belong_to_the_start_day() {
    use jiff::civil::Weekday as W;
    // Friday 20:00 to Saturday 08:00, started on Friday only.
    let s = quiet("20:00", "08:00", &[settings::Weekday::Fri]);
    let (c, m) = (conv("c1", "gen"), msg("c1", "1.1", "hi"));
    assert!(run_on(&s, &c, &m, None, false, at(23, 0), W::Friday).is_none());
    // After midnight the window is still Friday's.
    assert!(run_on(&s, &c, &m, None, false, at(3, 0), W::Saturday).is_none());
    // Friday morning belongs to Thursday's window, which is not selected.
    assert!(run_on(&s, &c, &m, None, false, at(3, 0), W::Friday).is_some());
    // Saturday evening starts a Saturday window: not selected.
    assert!(run_on(&s, &c, &m, None, false, at(23, 0), W::Saturday).is_some());
    // Sunday wraps to Monday.
    let s = quiet("20:00", "08:00", &[settings::Weekday::Sun]);
    assert!(run_on(&s, &c, &m, None, false, at(3, 0), W::Monday).is_none());
}

#[test]
fn quiet_hours_empty_days_is_every_day() {
    use jiff::civil::Weekday as W;
    let s = quiet("20:00", "08:00", &[]);
    let (c, m) = (conv("c1", "gen"), msg("c1", "1.1", "hi"));
    for d in [W::Monday, W::Thursday, W::Sunday] {
        assert!(run_on(&s, &c, &m, None, false, at(23, 0), d).is_none());
        assert!(run_on(&s, &c, &m, None, false, at(3, 0), d).is_none());
    }
}

#[test]
fn preview_off_keeps_only_sender_and_channel() {
    let mut s = settings(DesktopNotifications::All);
    s.show_preview = false;
    let n = run(
        &s,
        &conv("c1", "gen"),
        &msg("c1", "1.1", "secret"),
        None,
        false,
        at(12, 0),
    )
    .unwrap();
    assert_eq!(&*n.author, "Ann");
    assert_eq!(n.channel.as_deref(), Some("gen"));
    assert!(n.text.is_empty());
    assert_eq!(n.file_count, 0);
    s.show_preview = true;
    let n = run(
        &s,
        &conv("c1", "gen"),
        &msg("c1", "1.1", "secret"),
        None,
        false,
        at(12, 0),
    )
    .unwrap();
    assert_eq!(&*n.text, "secret");
}
