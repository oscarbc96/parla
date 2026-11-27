use std::sync::Arc;

use domain::test_support::{cid, conv, msg, ts, uid};
use domain::{
    Bootstrap, Broadcast, ConversationCounts, ConversationKind, Effect, HistoryPage, Input,
    Mention, Message, ProviderEvent, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state_with(convs: Vec<domain::Conversation>) -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: convs,
            members: vec![],
        }),
    );
    s
}
fn post(s: &mut WorkspaceState, m: Message) -> Vec<Effect> {
    reduce(s, Input::Provider(ProviderEvent::MessagePosted(m)))
}
fn mentioning(mut m: Message, x: Mention) -> Message {
    m.mentions = Arc::from([x]);
    m
}
fn mark_reads(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::MarkRead { ts, .. } => Some(ts.0.to_string()),
            _ => None,
        })
        .collect()
}
fn focus(s: &mut WorkspaceState, c: Option<&str>, bottom: bool, active: bool) -> Vec<Effect> {
    reduce(
        s,
        Input::Focus {
            conversation: c.map(cid),
            at_bottom: bottom,
            window_active: active,
        },
    )
}

#[test]
fn counts_loaded_sets_state() {
    let mut s = state_with(vec![conv("c1", "g")]);
    reduce(
        &mut s,
        Input::CountsLoaded(vec![ConversationCounts {
            conversation: cid("c1"),
            latest: Some(ts("9.000000")),
            last_read: Some(ts("5.000000")),
            unread: true,
            mentions: 2,
        }]),
    );
    let c = &s.conversations[&cid("c1")];
    assert!(c.unread);
    assert_eq!(c.mentions, 2);
    assert_eq!(c.last_read, Some(ts("5.000000")));
    assert_eq!(c.latest, Some(ts("9.000000")));
}

#[test]
fn own_message_never_unread() {
    let mut s = state_with(vec![conv("c1", "g")]);
    let mut m = msg("c1", "5.000000", "hi");
    m.author = Some(uid("me"));
    post(&mut s, mentioning(m, Mention::User(uid("me"))));
    let c = &s.conversations[&cid("c1")];
    assert!(!c.unread);
    assert_eq!(c.mentions, 0);
}

#[test]
fn dm_message_counts_as_mention() {
    let mut d = conv("d1", "ann");
    d.kind = ConversationKind::Direct;
    let mut s = state_with(vec![d]);
    post(&mut s, msg("d1", "5.000000", "yo"));
    let c = &s.conversations[&cid("d1")];
    assert!(c.unread);
    assert_eq!(c.mentions, 1);
}

#[test]
fn self_mention_counts_broadcast_in_muted_does_not() {
    let mut muted = conv("c1", "g");
    muted.is_muted = true;
    let mut s = state_with(vec![muted]);
    post(
        &mut s,
        mentioning(
            msg("c1", "5.000000", "a"),
            Mention::Broadcast(Broadcast::Channel),
        ),
    );
    assert_eq!(s.conversations[&cid("c1")].mentions, 0);
    post(
        &mut s,
        mentioning(msg("c1", "6.000000", "b"), Mention::User(uid("me"))),
    );
    assert_eq!(s.conversations[&cid("c1")].mentions, 1);

    let mut s = state_with(vec![conv("c2", "h")]);
    post(
        &mut s,
        mentioning(
            msg("c2", "5.000000", "a"),
            Mention::Broadcast(Broadcast::Here),
        ),
    );
    assert_eq!(s.conversations[&cid("c2")].mentions, 1);
}

#[test]
fn focused_at_bottom_marks_read_once() {
    let mut s = state_with(vec![conv("c1", "g")]);
    focus(&mut s, Some("c1"), true, true);
    let fx = post(&mut s, msg("c1", "5.000000", "a"));
    assert_eq!(mark_reads(&fx), vec!["5.000000"]);
    let fx = focus(&mut s, Some("c1"), true, true);
    assert!(mark_reads(&fx).is_empty());
    assert!(!s.conversations[&cid("c1")].unread);
}

#[test]
fn inactive_window_does_not_mark_read() {
    let mut s = state_with(vec![conv("c1", "g")]);
    focus(&mut s, Some("c1"), true, false);
    let fx = post(&mut s, msg("c1", "5.000000", "a"));
    assert!(mark_reads(&fx).is_empty());
    assert!(s.conversations[&cid("c1")].unread);
    let fx = focus(&mut s, Some("c1"), false, true);
    assert!(mark_reads(&fx).is_empty());
    let fx = focus(&mut s, Some("c1"), true, true);
    assert_eq!(mark_reads(&fx), vec!["5.000000"]);
}

#[test]
fn mark_unread_sets_marker_before_message_and_sticks() {
    let mut s = state_with(vec![conv("c1", "g"), conv("c2", "h")]);
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![msg("c1", "1.000000", "a"), msg("c1", "2.000000", "b")],
                has_more: false,
            },
        },
    );
    focus(&mut s, Some("c1"), true, true);
    let fx = reduce(
        &mut s,
        Input::MarkUnread(domain::test_support::mid("c1", "2.000000")),
    );
    assert_eq!(mark_reads(&fx), vec!["1.000000"]);
    let c = &s.conversations[&cid("c1")];
    assert!(c.unread);
    assert_eq!(c.last_read, Some(ts("1.000000")));
    // sticky
    let fx = focus(&mut s, Some("c1"), true, true);
    assert!(mark_reads(&fx).is_empty());
    focus(&mut s, Some("c2"), true, true);
    let fx = focus(&mut s, Some("c1"), true, true);
    assert_eq!(mark_reads(&fx), vec!["2.000000"]);

    // first message: falls back to just_before
    reduce(
        &mut s,
        Input::MarkUnread(domain::test_support::mid("c1", "1.000000")),
    );
    assert_eq!(s.conversations[&cid("c1")].last_read, Some(ts("0.999999")));
}

#[test]
fn read_marker_from_other_device_clears() {
    let mut d = conv("d1", "ann");
    d.kind = ConversationKind::Direct;
    let mut s = state_with(vec![d]);
    for t in ["1.000000", "2.000000", "3.000000"] {
        reduce(
            &mut s,
            Input::HistoryLoaded {
                conversation: cid("d1"),
                page: HistoryPage {
                    messages: vec![msg("d1", t, "x")],
                    has_more: false,
                },
            },
        );
        post(&mut s, msg("d1", t, "x"));
    }
    assert_eq!(s.conversations[&cid("d1")].mentions, 3);
    let marker = |t: &str| {
        Input::Provider(ProviderEvent::ReadMarker {
            conversation: cid("d1"),
            ts: ts(t),
        })
    };
    reduce(&mut s, marker("1.000000"));
    let c = &s.conversations[&cid("d1")];
    assert!(c.unread);
    assert_eq!(c.mentions, 2);
    reduce(&mut s, marker("3.000000"));
    let c = &s.conversations[&cid("d1")];
    assert!(!c.unread);
    assert_eq!(c.mentions, 0);
}

#[test]
fn totals_exclude_muted_unread() {
    let mut muted = conv("c2", "m");
    muted.is_muted = true;
    let mut s = state_with(vec![conv("c1", "g"), muted]);
    post(&mut s, msg("c1", "5.000000", "a"));
    post(&mut s, msg("c2", "5.000000", "a"));
    post(
        &mut s,
        mentioning(msg("c2", "6.000000", "a"), Mention::User(uid("me"))),
    );
    let t = s.totals();
    assert_eq!(t.conversations, 1);
    assert_eq!(t.mentions, 1);
}

fn read_markers(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Persist(domain::PersistOp::ReadMarker { ts, .. }) => Some(ts.0.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn auto_mark_read_persists_the_marker() {
    let mut s = state_with(vec![conv("c1", "g")]);
    focus(&mut s, Some("c1"), true, true);
    let fx = post(&mut s, msg("c1", "5.000000", "a"));
    assert_eq!(mark_reads(&fx), vec!["5.000000"]);
    assert_eq!(read_markers(&fx), vec!["5.000000"]);
}

#[test]
fn open_persists_the_marker() {
    let mut s = state_with(vec![conv("c1", "g")]);
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![msg("c1", "2.000000", "b")],
                has_more: false,
            },
        },
    );
    let fx = reduce(&mut s, Input::Open(cid("c1")));
    assert_eq!(mark_reads(&fx), vec!["2.000000"]);
    assert_eq!(read_markers(&fx), vec!["2.000000"]);
}

#[test]
fn opening_a_conversation_clears_its_badge_and_focus_marks_once() {
    let mut s = state_with(vec![conv("c1", "g")]);
    post(
        &mut s,
        mentioning(msg("c1", "5.000000", "hi"), Mention::User(uid("me"))),
    );
    let c = &s.conversations[&cid("c1")];
    assert!(c.unread && c.mentions == 1);
    let fx = reduce(&mut s, Input::Open(cid("c1")));
    assert_eq!(mark_reads(&fx), ["5.000000"]);
    let c = &s.conversations[&cid("c1")];
    assert!(!c.unread);
    assert_eq!(c.mentions, 0);
    assert_eq!(c.last_read, Some(ts("5.000000")));
    let fx = focus(&mut s, Some("c1"), true, true);
    assert!(
        mark_reads(&fx).is_empty(),
        "focus must not mark a second time"
    );
}

#[test]
fn a_stale_counts_snapshot_never_moves_the_markers_back() {
    let mut s = state_with(vec![conv("c1", "g")]);
    post(
        &mut s,
        mentioning(msg("c1", "9.000000", "hi"), Mention::User(uid("me"))),
    );
    reduce(&mut s, Input::Open(cid("c1")));
    reduce(
        &mut s,
        Input::CountsLoaded(vec![ConversationCounts {
            conversation: cid("c1"),
            latest: Some(ts("5.000000")),
            last_read: Some(ts("3.000000")),
            unread: true,
            mentions: 4,
        }]),
    );
    let c = &s.conversations[&cid("c1")];
    assert_eq!(c.latest, Some(ts("9.000000")));
    assert_eq!(c.last_read, Some(ts("9.000000")));
    assert!(!c.unread);
    assert_eq!(c.mentions, 0);
}

/// A failed row held at the bottom carries an older local ts: it is never
/// the message the marker moves back to.
#[test]
fn mark_unread_skips_unconfirmed_rows() {
    let mut s = state_with(vec![conv("c1", "g")]);
    s.self_user = Some(uid("me"));
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![msg("c1", "10.000000", "a"), msg("c1", "20.000000", "b")],
                has_more: false,
            },
        },
    );
    reduce(
        &mut s,
        Input::Send {
            conversation: cid("c1"),
            text: Arc::from("queued"),
            thread: None,
            id: domain::test_support::cmid("q"),
            now: ts("5.000000"),
            now_ms: 5_000,
        },
    );
    let last = s.windows[&cid("c1")].messages().last().unwrap().clone();
    assert_eq!(&*last.text, "queued", "the local row is held at the bottom");
    reduce(
        &mut s,
        Input::MarkUnread(domain::test_support::mid("c1", "20.000000")),
    );
    assert_eq!(s.conversations[&cid("c1")].last_read, Some(ts("10.000000")));
}
