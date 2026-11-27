//! User groups: loaded once per bootstrap, kept bounded, persisted, and a
//! mention of a group the user belongs to counts as a mention of them.

use std::sync::Arc;

use domain::notify::{NotifyContext, decide};
use domain::test_support::{FakeProvider, cid, conv, msg, uid};
use domain::{
    Bootstrap, ChatProvider as _, Effect, Input, MAX_USER_GROUPS, Mention, Message, PersistOp,
    ProviderEvent, UserGroup, WorkspaceState, reduce,
};
use futures::executor::block_on;
use settings::{DesktopNotifications, NotificationSettings, WorkspaceKey};

fn group(id: &str, handle: &str, members: u32, mine: bool) -> UserGroup {
    UserGroup {
        id: id.into(),
        handle: handle.into(),
        name: format!("{handle} team").into(),
        member_count: members,
        is_member: mine,
    }
}

fn boot() -> Bootstrap {
    Bootstrap {
        self_user: uid("me"),
        workspace_name: Arc::from("W"),
        conversations: vec![conv("c1", "general")],
        members: vec![],
    }
}

fn state(groups: bool) -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.caps.user_groups = groups;
    s
}

fn mentioning(mut m: Message, x: Mention) -> Message {
    m.mentions = Arc::from([x]);
    m
}

#[test]
fn bootstrap_fetches_user_groups_only_when_supported() {
    let mut s = state(false);
    let fx = reduce(&mut s, Input::Bootstrapped(boot()));
    assert!(!fx.iter().any(|e| matches!(e, Effect::FetchUserGroups)));
    let mut s = state(true);
    let fx = reduce(&mut s, Input::Bootstrapped(boot()));
    assert!(fx.iter().any(|e| matches!(e, Effect::FetchUserGroups)));
}

#[test]
fn loaded_groups_replace_the_set_are_bounded_and_persisted() {
    let mut s = state(true);
    reduce(
        &mut s,
        Input::UserGroupsLoaded(vec![group("S0", "old", 1, false)]),
    );
    let many: Vec<_> = (0..MAX_USER_GROUPS + 5)
        .map(|i| group(&format!("S{i:05}"), &format!("g{i:05}"), 2, false))
        .collect();
    let fx = reduce(&mut s, Input::UserGroupsLoaded(many));
    assert_eq!(s.user_groups.len(), MAX_USER_GROUPS);
    assert!(!s.user_groups.contains_key("S0"), "the set is replaced");
    let persisted = fx.iter().find_map(|e| match e {
        Effect::Persist(PersistOp::UserGroups(g)) => Some(g.len()),
        _ => None,
    });
    assert_eq!(persisted, Some(MAX_USER_GROUPS));
}

#[test]
fn cached_groups_paint_without_persisting_or_overriding() {
    let mut s = state(true);
    let fx = reduce(
        &mut s,
        Input::UserGroupsCached(vec![group("S1", "eng", 3, true)]),
    );
    assert!(s.user_groups.contains_key("S1"));
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::UserGroups(_))))
    );
    reduce(
        &mut s,
        Input::UserGroupsLoaded(vec![group("S2", "design", 4, false)]),
    );
    reduce(
        &mut s,
        Input::UserGroupsCached(vec![group("S1", "eng", 3, true)]),
    );
    assert!(
        !s.user_groups.contains_key("S1"),
        "a late cache read never replaces the provider's answer"
    );
}

#[test]
fn a_mention_of_my_group_counts_as_a_mention() {
    let mut s = state(true);
    reduce(&mut s, Input::Bootstrapped(boot()));
    reduce(
        &mut s,
        Input::UserGroupsLoaded(vec![
            group("S1", "eng", 3, true),
            group("S2", "design", 4, false),
        ]),
    );
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(mentioning(
            msg("c1", "5.000000", "a"),
            Mention::Group("S2".into()),
        ))),
    );
    assert_eq!(s.conversations[&cid("c1")].mentions, 0);
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(mentioning(
            msg("c1", "6.000000", "b"),
            Mention::Group("S1".into()),
        ))),
    );
    assert_eq!(s.conversations[&cid("c1")].mentions, 1);
}

#[test]
fn a_mention_of_my_group_notifies_on_mentions_only() {
    let s = NotificationSettings {
        desktop: DesktopNotifications::Mentions,
        ..Default::default()
    };
    let me = uid("u1");
    let groups: domain::UserGroups = [
        ("S1".into(), group("S1", "eng", 3, true)),
        ("S2".into(), group("S2", "design", 4, false)),
    ]
    .into_iter()
    .collect();
    let ctx = NotifyContext {
        settings: &s,
        self_user: &me,
        now: jiff::civil::time(12, 0, 0, 0),
        now_unix: 0,
        weekday: jiff::civil::Weekday::Monday,
        dnd: None,
        prefs: None,
        extras: None,
        thread_followed: false,
        thread_started_by_me: false,
        focused: None,
        window_active: false,
        user_groups: Some(&groups),
    };
    let ws = WorkspaceKey::new("fake", "w1");
    let c = conv("c1", "general");
    let mine = mentioning(msg("c1", "1.1", "hi"), Mention::Group("S1".into()));
    let other = mentioning(msg("c1", "1.2", "hi"), Mention::Group("S2".into()));
    assert!(decide(&ctx, &ws, &c, &mine, "Ann").is_some());
    assert!(decide(&ctx, &ws, &c, &other, "Ann").is_none());
}

#[test]
fn the_fake_provider_lists_its_groups() {
    let p = FakeProvider::new("w1", uid("me"));
    assert!(block_on(p.user_groups()).unwrap().is_empty());
    p.add_user_group(group("S1", "eng", 3, true));
    let all = block_on(p.user_groups()).unwrap();
    assert_eq!(all, vec![group("S1", "eng", 3, true)]);
}

fn groups(list: &[UserGroup]) -> domain::UserGroups {
    list.iter().map(|g| (g.id.clone(), g.clone())).collect()
}

#[test]
fn group_ids_in_plain_text_read_as_handles() {
    let gs = groups(&[group("S1", "eng", 3, true), group("S10", "ops", 2, false)]);
    let mentions = [
        Mention::Group("S1".into()),
        Mention::Group("S10".into()),
        Mention::Group("S2".into()),
    ];
    assert_eq!(
        domain::resolve_group_text("hi @S1, @S10 and @S2 (@S1x)", &mentions, &gs),
        "hi @eng, @ops and @S2 (@S1x)"
    );
    assert_eq!(
        domain::resolve_group_text("no groups @S1", &[], &gs),
        "no groups @S1",
        "only ids the message mentions are replaced"
    );
}

#[test]
fn notification_text_and_keywords_use_the_group_handle() {
    let s = NotificationSettings {
        desktop: DesktopNotifications::Mentions,
        keywords: vec!["design".into()],
        ..Default::default()
    };
    let me = uid("u1");
    let gs = groups(&[group("S2", "design", 4, false)]);
    let ctx = NotifyContext {
        settings: &s,
        self_user: &me,
        now: jiff::civil::time(12, 0, 0, 0),
        now_unix: 0,
        weekday: jiff::civil::Weekday::Monday,
        dnd: None,
        prefs: None,
        extras: None,
        thread_followed: false,
        thread_started_by_me: false,
        focused: None,
        window_active: false,
        user_groups: Some(&gs),
    };
    let mut m = mentioning(msg("c1", "1.1", "x"), Mention::Group("S2".into()));
    m.plain = "ship it @S2".into();
    let n = decide(
        &ctx,
        &WorkspaceKey::new("fake", "w1"),
        &conv("c1", "general"),
        &m,
        "Ann",
    )
    .expect("the keyword matches the handle");
    assert_eq!(&*n.text, "ship it @design");
}

#[test]
fn groups_arriving_recount_mentions_of_held_messages() {
    let mut s = state(true);
    let mut c = conv("c1", "general");
    c.last_read = Some(domain::test_support::ts("1.000000"));
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            conversations: vec![c],
            ..boot()
        }),
    );
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: domain::HistoryPage {
                messages: vec![msg("c1", "1.000000", "old")],
                has_more: false,
            },
        },
    );
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(mentioning(
            msg("c1", "5.000000", "a"),
            Mention::Group("S1".into()),
        ))),
    );
    let c = &s.conversations[&cid("c1")];
    assert!(c.unread);
    assert_eq!(c.mentions, 0, "the group is not known yet");
    reduce(
        &mut s,
        Input::UserGroupsLoaded(vec![group("S1", "eng", 3, true)]),
    );
    assert_eq!(s.conversations[&cid("c1")].mentions, 1);
}
