use std::sync::Arc;

use domain::test_support::{cid, cmid, conv, mid, msg, ts, uid};
use domain::{
    Connection, DeliveryState, Effect, HistoryPage, Input, Message, ProviderEvent, WorkspaceState,
    reduce,
};
use settings::WorkspaceKey;

fn state(online: bool) -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.self_user = Some(uid("me"));
    s.conversations.insert(cid("c1"), conv("c1", "general"));
    if online {
        reduce(
            &mut s,
            Input::Provider(ProviderEvent::Connected { reconnect: false }),
        );
        assert_eq!(s.connection, Connection::Online);
    }
    // The root sits in the conversation window.
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![msg("c1", "1.000000", "root")],
                has_more: false,
            },
        },
    );
    s
}

fn reply_msg(t: &str, broadcast: bool) -> Message {
    let mut m = msg("c1", t, "r");
    m.thread_root = Some(ts("1.000000"));
    m.broadcast = broadcast;
    m
}

fn post(s: &mut WorkspaceState, m: Message) -> Vec<Effect> {
    reduce(s, Input::Provider(ProviderEvent::MessagePosted(m)))
}

fn root() -> domain::MessageId {
    mid("c1", "1.000000")
}

fn open(s: &mut WorkspaceState) -> Vec<Effect> {
    reduce(s, Input::OpenThread(root()))
}

fn conv_root(s: &WorkspaceState) -> Message {
    s.windows[&cid("c1")].messages()[0].clone()
}

#[test]
fn reply_goes_to_thread_not_channel() {
    let mut s = state(true);
    open(&mut s);
    post(&mut s, reply_msg("2.000000", false));
    assert_eq!(s.threads[&root()].replies.messages().len(), 1);
    let ch: Vec<_> = s.windows[&cid("c1")].messages().iter().collect();
    assert_eq!(ch.len(), 1, "non-broadcast reply stays out of the channel");
    assert_eq!(s.conversations[&cid("c1")].latest, None);
}

#[test]
fn broadcast_reply_in_both() {
    let mut s = state(true);
    open(&mut s);
    post(&mut s, reply_msg("2.000000", true));
    assert_eq!(s.threads[&root()].replies.messages().len(), 1);
    assert_eq!(s.windows[&cid("c1")].messages().len(), 2);
}

#[test]
fn root_summary_updates_on_reply() {
    let mut s = state(true);
    open(&mut s);
    let mut r = reply_msg("2.000000", false);
    r.author = Some(uid("u3"));
    let fx = post(&mut s, r.clone());
    let m = conv_root(&s);
    assert_eq!(m.reply_count, 1);
    assert_eq!(m.latest_reply, Some(ts("2.000000")));
    assert_eq!(&*m.reply_users, &[uid("u3")]);
    assert_eq!(s.threads[&root()].root.as_ref().unwrap().reply_count, 1);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Persist(domain::PersistOp::ThreadSummary { reply_count: 1, .. })
    )));
    // The same reply again does not count twice.
    post(&mut s, r);
    assert_eq!(conv_root(&s).reply_count, 1);
}

#[test]
fn open_thread_fetches_and_marks_read() {
    let mut s = state(true);
    let fx = open(&mut s);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::FetchReplies { root: r, after: None } if *r == root()
    )));
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::MarkThreadRead { .. }))
    );
    reduce(
        &mut s,
        Input::RepliesLoaded {
            root: root(),
            page: HistoryPage {
                messages: vec![
                    msg("c1", "1.000000", "root"),
                    reply_msg("2.000000", false),
                    reply_msg("3.000000", false),
                ],
                has_more: false,
            },
        },
    );
    assert_eq!(s.threads[&root()].replies.messages().len(), 2);
    assert!(s.threads[&root()].root.is_some());
    let fx = open(&mut s);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::FetchReplies { after: Some(a), .. } if *a == ts("3.000000")
    )));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::MarkThreadRead { ts: t, .. } if *t == ts("3.000000")
    )));
    // Two opens (viewers) require two closes.
    reduce(&mut s, Input::CloseThread(root()));
    assert!(s.threads.contains_key(&root()));
    reduce(&mut s, Input::CloseThread(root()));
    assert!(!s.threads.contains_key(&root()));
}

#[test]
fn thread_reply_offline_queues() {
    let mut s = state(false);
    let fx = reduce(
        &mut s,
        Input::Reply {
            root: root(),
            text: Arc::from("hi"),
            broadcast: true,
            id: cmid("a"),
            now: ts("10.000000"),
            now_ms: 10_000,
        },
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::CallReply { .. })));
    let it = s.outbox().iter().find(|i| i.id == cmid("a")).unwrap();
    assert_eq!(it.thread, Some(ts("1.000000")));
    assert!(it.broadcast);
    let pending = &s.threads[&root()].replies.messages()[0];
    assert_eq!(pending.state, DeliveryState::Pending);
    assert!(pending.broadcast);
    // Broadcast pending shows in the channel too.
    assert_eq!(s.windows[&cid("c1")].messages().len(), 2);

    // Going online sends it as a reply, then the result lands in the thread.
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::Connected { reconnect: false }),
    );
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::CallReply { root: r, broadcast: true, id, .. } if *r == root() && *id == cmid("a")
    )));
    let mut done = msg("c1", "11.000000", "hi");
    done.thread_root = Some(ts("1.000000"));
    done.broadcast = true;
    reduce(
        &mut s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid("a"),
            result: Ok(done),
        },
    );
    // Nobody views the thread, so no window is kept for the delivered reply.
    assert!(!s.threads.contains_key(&root()));
    assert_eq!(conv_root(&s).reply_count, 1);
}

fn send_reply(s: &mut WorkspaceState) {
    reduce(
        s,
        Input::Reply {
            root: root(),
            text: Arc::from("hi"),
            broadcast: false,
            id: cmid("a"),
            now: ts("10.000000"),
            now_ms: 10_000,
        },
    );
    let mut done = msg("c1", "11.000000", "hi");
    done.thread_root = Some(ts("1.000000"));
    reduce(
        s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid("a"),
            result: Ok(done),
        },
    );
}

#[test]
fn delivered_reply_keeps_the_window_while_viewed() {
    let mut s = state(true);
    open(&mut s);
    send_reply(&mut s);
    let r = s.threads[&root()].replies.messages();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].state, DeliveryState::Delivered);
}

#[test]
fn send_with_the_panel_closed_leaves_no_window() {
    let mut s = state(true);
    send_reply(&mut s);
    assert!(!s.threads.contains_key(&root()));
    assert_eq!(conv_root(&s).reply_count, 1);
}

#[test]
fn late_replies_loaded_after_close_leave_no_window() {
    let mut s = state(true);
    open(&mut s);
    reduce(&mut s, Input::CloseThread(root()));
    reduce(
        &mut s,
        Input::RepliesLoaded {
            root: root(),
            page: HistoryPage {
                messages: vec![reply_msg("2.000000", false)],
                has_more: false,
            },
        },
    );
    assert!(!s.threads.contains_key(&root()));
}

#[test]
fn deleted_reply_leaves_the_open_thread() {
    let mut s = state(true);
    open(&mut s);
    post(&mut s, reply_msg("2.000000", false));
    assert_eq!(s.threads[&root()].replies.messages().len(), 1);
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessageDeleted(mid("c1", "2.000000"))),
    );
    assert!(s.threads[&root()].replies.messages().is_empty());
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(domain::Changed::Thread(r)) if *r == root()))
    );
}

/// Rapid replies: a confirmed reply with a later server ts stays above the
/// replies still pending, which keep their send order.
#[test]
fn pending_thread_replies_stay_below_confirmed_ones() {
    let mut s = state(true);
    open(&mut s);
    for (id, t, ms) in [
        ("a", "10.000000", 10_000),
        ("b", "11.000000", 11_000),
        ("c", "12.000000", 12_000),
    ] {
        reduce(
            &mut s,
            Input::Reply {
                root: root(),
                text: Arc::from(format!("t-{id}")),
                broadcast: false,
                id: cmid(id),
                now: ts(t),
                now_ms: ms,
            },
        );
    }
    let mut m = reply_msg("20.000001", false);
    m.text = Arc::from("t-a");
    m.author = Some(uid("me"));
    m.client_msg_id = Some(cmid("a"));
    reduce(
        &mut s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid("a"),
            result: Ok(m),
        },
    );
    let r: Vec<(String, bool)> = s.threads[&root()]
        .replies
        .messages()
        .iter()
        .map(|m| (m.text.to_string(), m.state == DeliveryState::Delivered))
        .collect();
    let want: Vec<(String, bool)> = [("t-a", true), ("t-b", false), ("t-c", false)]
        .iter()
        .map(|(t, d)| (t.to_string(), *d))
        .collect();
    assert_eq!(r, want);
}
