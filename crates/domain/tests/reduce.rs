use std::sync::Arc;

use domain::test_support::{cid, cmid, conv, member, mid, msg, ts, uid};
use domain::{
    Bootstrap, Changed, Connection, DeliveryState, Effect, HistoryPage, Input, Message, PersistOp,
    ProviderEvent, Session, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state(cap: usize) -> WorkspaceState {
    WorkspaceState::new(WorkspaceKey::new("fake", "w1"), cap)
}
fn texts(s: &WorkspaceState, c: &str) -> Vec<String> {
    s.windows[&cid(c)]
        .messages()
        .iter()
        .map(|m| m.text.to_string())
        .collect()
}
fn changed(effects: &[Effect], want: &Changed) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Changed(c) if c == want))
}
fn send(s: &mut WorkspaceState, text: &str, id: &str) -> Vec<Effect> {
    reduce(
        s,
        Input::Send {
            conversation: cid("c1"),
            text: Arc::from(text),
            thread: None,
            id: cmid(id),
            now: ts("100.000000"),
            now_ms: 100_000,
        },
    )
}
fn echo(text: &str, t: &str, id: &str) -> Message {
    let mut m = msg("c1", t, text);
    m.author = Some(uid("me"));
    m.client_msg_id = Some(cmid(id));
    m
}

#[test]
fn bootstrap_fills_sidebar() {
    let mut s = state(200);
    let fx = reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("Acme"),
            conversations: vec![conv("c2", "random"), conv("c1", "general")],
            members: vec![member("u2", "bo")],
        }),
    );
    assert_eq!(s.self_user, Some(uid("me")));
    assert_eq!(&*s.name, "Acme");
    let names: Vec<_> = s
        .conversations
        .values()
        .map(|c| c.name.to_string())
        .collect();
    assert_eq!(names, ["general", "random"]);
    assert!(s.members.contains_key(&uid("u2")));
    assert!(changed(&fx, &Changed::Conversations));
    assert!(changed(&fx, &Changed::Members));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::Conversations(c)) if c.len() == 2))
    );
}

#[test]
fn send_echo_is_deduped() {
    let mut s = state(200);
    s.connection = Connection::Online;
    let fx = send(&mut s, "hi", "k1");
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::CallSend { id, .. } if *id == cmid("k1")))
    );
    assert_eq!(
        s.windows[&cid("c1")].messages()[0].state,
        DeliveryState::Pending
    );

    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(echo("hi", "101.000001", "k1"))),
    );
    let w = s.windows[&cid("c1")].messages();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].state, DeliveryState::Delivered);

    reduce(
        &mut s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid("k1"),
            result: Ok(echo("hi", "101.000001", "k1")),
        },
    );
    let w = s.windows[&cid("c1")].messages();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].state, DeliveryState::Delivered);
}

/// A realtime echo without the client id (the server may not keep ours) still
/// settles the pending row in place: no second row shows next to it.
#[test]
fn echo_without_client_id_settles_the_pending_row() {
    let mut s = state(200);
    s.self_user = Some(uid("me"));
    s.connection = Connection::Online;
    send(&mut s, "hi", "k1");
    let mut e = echo("hi", "101.000001", "k1");
    e.client_msg_id = None;
    reduce(&mut s, Input::Provider(ProviderEvent::MessagePosted(e)));
    let w = s.windows[&cid("c1")].messages();
    assert_eq!(w.len(), 1, "one row, never the pending one and its echo");
    assert_eq!(w[0].state, DeliveryState::Delivered);
    assert_eq!(w[0].client_msg_id, Some(cmid("k1")));
    assert!(s.outbox().is_empty());
}

/// The same text sent twice, the first response arriving before its echo:
/// the late id-less echo is the first message (already held), not the second
/// send, which stays pending and can still fail.
#[test]
fn late_echo_does_not_steal_the_next_send_of_the_same_text() {
    let mut s = state(200);
    s.self_user = Some(uid("me"));
    s.connection = Connection::Online;
    send(&mut s, "hi", "k1");
    send(&mut s, "hi", "k2");
    reduce(
        &mut s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid("k1"),
            result: Ok(echo("hi", "101.000001", "k1")),
        },
    );
    // k2 is now in flight.
    let mut late = echo("hi", "101.000001", "k1");
    late.client_msg_id = None;
    reduce(&mut s, Input::Provider(ProviderEvent::MessagePosted(late)));
    assert_eq!(s.outbox().len(), 1, "k2 is still queued");
    let w = s.windows[&cid("c1")].messages();
    assert_eq!(w.len(), 2);
    assert_eq!(w[0].state, DeliveryState::Delivered);
    assert_eq!(w[1].state, DeliveryState::Pending);
    reduce(
        &mut s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid("k2"),
            result: Err(domain::ProviderError::NotAllowed("no".into())),
        },
    );
    let w = s.windows[&cid("c1")].messages();
    assert!(matches!(w[1].state, DeliveryState::Failed(_)));
}

/// Sends within one second keep their order in the stored outbox, which
/// sorts by `created_ms` alone (client ids are random).
#[test]
fn sends_in_one_second_get_increasing_created_ms() {
    let mut s = state(200);
    for id in ["b", "a", "c"] {
        send(&mut s, id, id);
    }
    let ms: Vec<i64> = s.outbox().iter().map(|i| i.created_ms).collect();
    assert!(ms.windows(2).all(|w| w[0] < w[1]), "{ms:?}");
}

/// `created_ms` keeps the millisecond part of the send time: the pending
/// timestamp shown after a reload must not collapse to whole seconds.
#[test]
fn created_ms_keeps_the_millisecond_part() {
    let mut s = state(200);
    for (id, now, now_ms) in [("a", "5.100000", 5_100), ("b", "5.900000", 5_900)] {
        reduce(
            &mut s,
            Input::Send {
                conversation: cid("c1"),
                text: Arc::from(id),
                thread: None,
                id: cmid(id),
                now: ts(now),
                now_ms,
            },
        );
    }
    let ms: Vec<i64> = s.outbox().iter().map(|i| i.created_ms).collect();
    assert_eq!(ms, [5_100, 5_900]);
}

#[test]
fn send_finished_first_then_echo_is_noop() {
    let mut s = state(200);
    send(&mut s, "hi", "k1");
    reduce(
        &mut s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid("k1"),
            result: Ok(echo("hi", "101.000001", "k1")),
        },
    );
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(echo("hi", "101.000001", "k1"))),
    );
    assert_eq!(s.windows[&cid("c1")].messages().len(), 1);
}

#[test]
fn send_failure_keeps_text() {
    let mut s = state(200);
    send(&mut s, "hello", "k1");
    reduce(
        &mut s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid("k1"),
            result: Err(domain::ProviderError::Other("boom".into())),
        },
    );
    let m = &s.windows[&cid("c1")].messages()[0];
    assert_eq!(&*m.text, "hello");
    assert_eq!(m.state, DeliveryState::Failed("boom".into()));
}

#[test]
fn out_of_order_insert_sorted() {
    let mut s = state(200);
    reduce(&mut s, Input::Open(cid("c1")));
    for (t, x) in [("30.000000", "c"), ("10.000000", "a"), ("20.000000", "b")] {
        reduce(
            &mut s,
            Input::Provider(ProviderEvent::MessagePosted(msg("c1", t, x))),
        );
    }
    assert_eq!(texts(&s, "c1"), ["a", "b", "c"]);
}

#[test]
fn window_caps_and_flags_more() {
    let mut s = state(3);
    reduce(&mut s, Input::Open(cid("c1")));
    for i in 1..=5 {
        reduce(
            &mut s,
            Input::Provider(ProviderEvent::MessagePosted(msg(
                "c1",
                &format!("{i}.000000"),
                &i.to_string(),
            ))),
        );
    }
    assert_eq!(texts(&s, "c1"), ["3", "4", "5"]);
    assert!(s.windows[&cid("c1")].has_more_before());
}

#[test]
fn edit_and_delete_in_window() {
    let mut s = state(200);
    reduce(&mut s, Input::Open(cid("c1")));
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(msg("c1", "1.000000", "a"))),
    );
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(msg("c1", "2.000000", "b"))),
    );
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessageEdited {
            id: mid("c1", "1.000000"),
            text: Arc::from("a2"),
            plain: Arc::from("a2"),
            edited: ts("3.000000"),
        }),
    );
    assert_eq!(texts(&s, "c1"), ["a2", "b"]);
    assert_eq!(
        s.windows[&cid("c1")].messages()[0].edited,
        Some(ts("3.000000"))
    );
    assert!(changed(&fx, &Changed::Timeline(cid("c1"))));
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessageDeleted(mid("c1", "2.000000"))),
    );
    assert_eq!(texts(&s, "c1"), ["a2"]);
}

#[test]
fn edit_outside_window_only_persists() {
    let mut s = state(200);
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessageEdited {
            id: mid("c9", "1.000000"),
            text: Arc::from("x"),
            plain: Arc::from("x"),
            edited: ts("2.000000"),
        }),
    );
    assert_eq!(fx.len(), 1);
    assert!(matches!(
        fx[0],
        Effect::Persist(PersistOp::EditMessage { .. })
    ));
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessageDeleted(mid("c9", "1.000000"))),
    );
    assert_eq!(fx.len(), 1);
    assert!(matches!(
        fx[0],
        Effect::Persist(PersistOp::DeleteMessage(_))
    ));
}

#[test]
fn workspace_rename_updates_name_only_when_changed() {
    let mut s = state(200);
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("Acme"),
            conversations: vec![],
            members: vec![],
        }),
    );
    assert_eq!(&*s.name, "Acme");

    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::WorkspaceRenamed {
            name: Arc::from("Renamed"),
        }),
    );
    assert_eq!(&*s.name, "Renamed");
    assert!(changed(&fx, &Changed::Conversations));

    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::WorkspaceRenamed {
            name: Arc::from("Renamed"),
        }),
    );
    assert_eq!(&*s.name, "Renamed");
    assert!(!changed(&fx, &Changed::Conversations));
}

#[test]
fn open_fetches_and_marks_read() {
    let mut s = state(200);
    s.conversations.insert(cid("c1"), conv("c1", "general"));
    let fx = reduce(&mut s, Input::Open(cid("c1")));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::FetchHistory { before: None, .. }))
    );

    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![msg("c1", "1.000000", "a"), msg("c1", "2.000000", "b")],
                has_more: true,
            },
        },
    );
    assert!(s.windows[&cid("c1")].has_more_before());
    let fx = reduce(&mut s, Input::Open(cid("c1")));
    assert!(!fx.iter().any(|e| matches!(e, Effect::FetchHistory { .. })));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::MarkRead { ts: t, .. } if *t == ts("2.000000")))
    );
}

#[test]
fn session_expired_sets_state() {
    let mut s = state(200);
    let fx = reduce(&mut s, Input::Provider(ProviderEvent::SessionExpired));
    assert_eq!(s.session, Session::Expired);
    assert!(changed(&fx, &Changed::Session));
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::Connected { reconnect: false }),
    );
    assert_eq!(s.connection, Connection::Online);
    assert!(changed(&fx, &Changed::Connection));
}

#[test]
fn cache_loaded_paints_without_persisting() {
    let mut s = state(50);
    let fx = reduce(
        &mut s,
        Input::CacheLoaded {
            conversations: vec![conv("c1", "general")],
            members: vec![member("u1", "Me")],
            conversation: Some(cid("c1")),
            messages: vec![msg("c1", "1.000000", "hi")],
        },
    );
    assert_eq!(texts(&s, "c1"), vec!["hi"]);
    assert!(s.windows[&cid("c1")].has_more_before());
    assert!(s.conversations.contains_key(&cid("c1")));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Persist(_))));
}

#[test]
fn an_empty_history_page_marks_the_window_loaded_but_empty() {
    use domain::test_support::{cid, conv};
    let mut s = WorkspaceState::new(settings::WorkspaceKey::new("fake", "w1"), 200);
    s.conversations.insert(cid("c1"), conv("c1", "general"));
    reduce(&mut s, Input::Open(cid("c1")));
    let w = &s.windows[&cid("c1")];
    assert!(w.is_empty() && !w.is_loaded(), "still loading after open");
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: domain::HistoryPage {
                messages: vec![],
                has_more: false,
            },
        },
    );
    let w = &s.windows[&cid("c1")];
    assert!(w.is_empty() && w.is_loaded(), "loaded and empty");
}

#[test]
fn conversation_meta_changed_merges_only_metadata() {
    use domain::ConversationMeta;
    let mut s = state(200);
    let mut c = conv("c1", "general");
    c.unread = true;
    c.mentions = 3;
    c.is_muted = true;
    c.last_read = Some(ts("4.000000"));
    c.latest = Some(ts("9.000000"));
    c.topic = Arc::from("old");
    s.conversations.insert(cid("c1"), c);
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::ConversationMetaChanged(ConversationMeta {
            id: cid("c1"),
            name: Some(Arc::from("renamed")),
            topic: None,
            is_archived: Some(true),
            is_member: Some(false),
            member_count: Some(7),
            purpose: None,
        })),
    );
    let c = &s.conversations[&cid("c1")];
    assert_eq!(&*c.name, "renamed");
    assert_eq!(&*c.topic, "old");
    assert!(c.is_archived && !c.is_member);
    assert_eq!(c.member_count, Some(7));
    assert!(c.unread && c.is_muted);
    assert_eq!(c.mentions, 3);
    assert_eq!(c.last_read, Some(ts("4.000000")));
    assert_eq!(c.latest, Some(ts("9.000000")));
    assert!(changed(&fx, &Changed::Conversations));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::Conversations(v)) if v.len() == 1))
    );
}

#[test]
fn conversation_meta_changed_ignores_unknown_and_unchanged() {
    use domain::ConversationMeta;
    let mut s = state(200);
    s.conversations.insert(cid("c1"), conv("c1", "general"));
    let meta = |id: &str, name: &str| {
        Input::Provider(ProviderEvent::ConversationMetaChanged(ConversationMeta {
            id: cid(id),
            name: Some(Arc::from(name)),
            topic: None,
            is_archived: None,
            is_member: None,
            member_count: None,
            purpose: None,
        }))
    };
    assert!(reduce(&mut s, meta("nope", "x")).is_empty());
    assert!(!s.conversations.contains_key(&cid("nope")));
    assert!(reduce(&mut s, meta("c1", "general")).is_empty());
}
