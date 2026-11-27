use std::sync::Arc;

use domain::test_support::ts;
use domain::test_support::{cid, cmid, msg, uid};
use domain::{
    ClientMsgId, Connection, DeliveryState, Effect, Input, Message, OutboxItem, OutboxState,
    ProviderError, ProviderEvent, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state(online: bool) -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.self_user = Some(uid("me"));
    if online {
        reduce(
            &mut s,
            Input::Provider(ProviderEvent::Connected { reconnect: false }),
        );
        assert_eq!(s.connection, Connection::Online);
    }
    s
}
fn send(s: &mut WorkspaceState, id: &str, secs: u32) -> Vec<Effect> {
    reduce(
        s,
        Input::Send {
            conversation: cid("c1"),
            text: Arc::from(format!("t-{id}")),
            thread: None,
            id: cmid(id),
            now: ts(&format!("{secs}.000000")),
            now_ms: i64::from(secs) * 1000,
        },
    )
}
fn connect(s: &mut WorkspaceState) -> Vec<Effect> {
    reduce(
        s,
        Input::Provider(ProviderEvent::Connected { reconnect: false }),
    )
}
fn calls(fx: &[Effect]) -> Vec<ClientMsgId> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::CallSend { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect()
}
fn finish(s: &mut WorkspaceState, id: &str, r: Result<Message, ProviderError>) -> Vec<Effect> {
    reduce(
        s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid(id),
            result: r,
        },
    )
}
fn delivered(id: &str, t: &str) -> Message {
    let mut m = msg("c1", t, &format!("t-{id}"));
    m.author = Some(uid("me"));
    m.client_msg_id = Some(cmid(id));
    m
}
fn item(s: &WorkspaceState, id: &str) -> Option<OutboxItem> {
    s.outbox().iter().find(|i| i.id == cmid(id)).cloned()
}
fn removed(fx: &[Effect], id: &str) -> usize {
    fx.iter()
        .filter(|e| matches!(e, Effect::Outbox(domain::OutboxOp::Remove(i)) if *i == cmid(id)))
        .count()
}

#[test]
fn offline_send_queues_without_call() {
    let mut s = state(false);
    let fx = send(&mut s, "a", 10);
    assert!(calls(&fx).is_empty());
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Outbox(domain::OutboxOp::Put(i)) if i.id == cmid("a") && i.state == OutboxState::Queued
    )));
    assert_eq!(item(&s, "a").unwrap().state, OutboxState::Queued);
    assert_eq!(
        s.windows[&cid("c1")].messages()[0].state,
        DeliveryState::Pending
    );
}

#[test]
fn connected_flushes_in_order_one_at_a_time() {
    let mut s = state(false);
    send(&mut s, "b", 20);
    send(&mut s, "a", 10);
    send(&mut s, "c", 30);
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::Connected { reconnect: false }),
    );
    assert_eq!(calls(&fx), [cmid("a")]);
    assert_eq!(item(&s, "a").unwrap().state, OutboxState::Sending);
    let fx = finish(&mut s, "a", Ok(delivered("a", "101.000001")));
    assert_eq!(removed(&fx, "a"), 1);
    assert_eq!(calls(&fx), [cmid("b")]);
    let fx = finish(&mut s, "b", Ok(delivered("b", "102.000001")));
    assert_eq!(calls(&fx), [cmid("c")]);
    let fx = finish(&mut s, "c", Ok(delivered("c", "103.000001")));
    assert!(calls(&fx).is_empty());
    assert!(s.outbox().is_empty());
}

#[test]
fn online_send_while_in_flight_goes_at_once() {
    let mut s = state(true);
    assert_eq!(calls(&send(&mut s, "a", 10)), [cmid("a")]);
    assert_eq!(calls(&send(&mut s, "b", 20)), [cmid("b")]);
    assert_eq!(s.outbox_inflight(), [cmid("a"), cmid("b")]);
}

/// Past the in-flight cap a live send queues, and goes when the slots drain.
#[test]
fn sends_past_the_in_flight_cap_wait_their_turn() {
    let mut s = state(true);
    let ids = ["a", "b", "c", "d", "e", "f"];
    let mut started = Vec::new();
    for (k, id) in ids.iter().enumerate() {
        started.extend(calls(&send(&mut s, id, 10 + k as u32)));
    }
    assert_eq!(started.len(), domain::MAX_IN_FLIGHT);
    assert_eq!(item(&s, "f").unwrap().state, OutboxState::Queued);
    for (k, id) in ids[..5].iter().enumerate() {
        let fx = finish(&mut s, id, Ok(delivered(id, &format!("100.00000{k}"))));
        let expect: &[ClientMsgId] = if k == 4 { &[cmid("f")][..] } else { &[] };
        assert_eq!(calls(&fx), expect, "after {id}");
    }
}

/// A live send typed behind a queued backlog waits behind it, in order.
#[test]
fn live_send_behind_a_backlog_keeps_its_place() {
    let mut s = state(false);
    send(&mut s, "a", 10);
    send(&mut s, "b", 11);
    assert_eq!(calls(&connect(&mut s)), [cmid("a")]);
    assert!(calls(&send(&mut s, "c", 12)).is_empty());
    assert_eq!(item(&s, "c").unwrap().state, OutboxState::Queued);
    let fx = finish(&mut s, "a", Ok(delivered("a", "100.000001")));
    assert_eq!(calls(&fx), [cmid("b")]);
}

#[test]
fn network_failure_requeues() {
    let mut s = state(true);
    send(&mut s, "a", 10);
    let fx = finish(&mut s, "a", Err(ProviderError::Network("down".into())));
    let i = item(&s, "a").unwrap();
    assert_eq!(i.state, OutboxState::Queued);
    assert_eq!(i.attempts, 1);
    assert!(calls(&fx).is_empty());
    assert_eq!(
        s.windows[&cid("c1")].messages()[0].state,
        DeliveryState::Pending
    );
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::Connected { reconnect: true }),
    );
    assert_eq!(calls(&fx), [cmid("a")]);
}

#[test]
fn transient_failure_while_online_arms_a_retry() {
    let mut s = state(true);
    send(&mut s, "a", 10);
    let fx = finish(&mut s, "a", Err(ProviderError::Network("reset".into())));
    assert!(fx.iter().any(|e| matches!(e, Effect::RetryOutbox { .. })));
    assert!(calls(&fx).is_empty());
    let fx = reduce(&mut s, Input::OutboxRetry);
    assert_eq!(calls(&fx), [cmid("a")]);
}

#[test]
fn transient_failure_while_offline_waits_for_the_reconnect() {
    let mut s = state(true);
    send(&mut s, "a", 10);
    reduce(&mut s, Input::Provider(ProviderEvent::Disconnected));
    let fx = finish(&mut s, "a", Err(ProviderError::Network("down".into())));
    assert!(!fx.iter().any(|e| matches!(e, Effect::RetryOutbox { .. })));
}

#[test]
fn other_failure_marks_failed_and_continues_with_next() {
    let mut s = state(false);
    send(&mut s, "a", 10);
    send(&mut s, "b", 20);
    connect(&mut s); // a backlog drains one at a time
    let fx = finish(&mut s, "a", Err(ProviderError::NotAllowed("no".into())));
    assert!(matches!(
        item(&s, "a").unwrap().state,
        OutboxState::Failed(_)
    ));
    assert_eq!(calls(&fx), [cmid("b")]);
    assert!(matches!(
        s.windows[&cid("c1")].messages()[0].state,
        DeliveryState::Failed(_)
    ));
}

#[test]
fn sending_at_restart_becomes_failed_not_resent() {
    let mut s = state(true);
    let mk = |id: &str, ms: i64, st: OutboxState| OutboxItem {
        id: cmid(id),
        conversation: cid("c1"),
        thread: None,
        broadcast: false,
        text: Arc::from(format!("t-{id}")),
        created_ms: ms,
        attempts: 0,
        state: st,
    };
    let fx = reduce(
        &mut s,
        Input::OutboxLoaded(vec![
            mk("a", 10_000, OutboxState::Sending),
            mk("b", 20_000, OutboxState::Queued),
        ]),
    );
    assert!(matches!(
        item(&s, "a").unwrap().state,
        OutboxState::Failed(_)
    ));
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Outbox(domain::OutboxOp::Put(i)) if i.id == cmid("a")
                && matches!(&i.state, OutboxState::Failed(m) if m.contains("may have been sent")))
    ));
    assert_eq!(calls(&fx), [cmid("b")]);
    assert_eq!(s.windows[&cid("c1")].messages().len(), 2);
}

#[test]
fn echo_during_flush_dedupes() {
    let mut s = state(false);
    send(&mut s, "a", 10);
    send(&mut s, "b", 20);
    connect(&mut s); // a backlog drains one at a time
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(delivered("a", "101.000001"))),
    );
    assert_eq!(removed(&fx, "a"), 1);
    // The echo proves the server took "a": "b" goes without waiting for the
    // slow HTTP answer.
    assert_eq!(calls(&fx), [cmid("b")]);
    let fx = finish(&mut s, "a", Ok(delivered("a", "101.000001")));
    assert_eq!(removed(&fx, "a"), 0);
    assert!(calls(&fx).is_empty(), "b is already in flight");
    assert_eq!(
        s.windows[&cid("c1")]
            .messages()
            .iter()
            .filter(|m| m.client_msg_id == Some(cmid("a")))
            .count(),
        1
    );
}

#[test]
fn retry_and_discard() {
    let mut s = state(true);
    send(&mut s, "a", 10);
    send(&mut s, "b", 20);
    finish(&mut s, "a", Err(ProviderError::Other("x".into())));
    finish(&mut s, "b", Err(ProviderError::Other("y".into())));
    let fx = reduce(&mut s, Input::Retry(cmid("a")));
    assert_eq!(calls(&fx), [cmid("a")]);
    assert_eq!(
        s.windows[&cid("c1")].messages()[0].state,
        DeliveryState::Pending
    );
    finish(&mut s, "a", Err(ProviderError::Other("x".into())));
    let fx = reduce(&mut s, Input::Discard(cmid("b")));
    assert_eq!(removed(&fx, "b"), 1);
    assert!(item(&s, "b").is_none());
    assert_eq!(s.windows[&cid("c1")].messages().len(), 1);
}

#[test]
fn hard_failure_after_echo_still_flushes_the_queue() {
    let mut s = state(false);
    send(&mut s, "a", 10);
    send(&mut s, "b", 20);
    connect(&mut s); // a backlog drains one at a time
    // The echo removes `a` while its send is still in flight.
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(delivered("a", "101.000001"))),
    );
    assert_eq!(calls(&fx), [cmid("b")], "the echo frees the slot");
    let fx = finish(&mut s, "a", Err(ProviderError::NotAllowed("no".into())));
    assert!(calls(&fx).is_empty(), "b is already in flight");
}

#[test]
fn going_offline_queues_new_sends_until_the_reconnect() {
    let mut s = state(true);
    reduce(&mut s, Input::Provider(ProviderEvent::Disconnected));
    assert_eq!(s.connection, Connection::Offline);
    let fx = send(&mut s, "a", 10);
    assert!(calls(&fx).is_empty());
    assert_eq!(item(&s, "a").unwrap().state, OutboxState::Queued);
    // The row the timeline shows is Pending (it reads as "Queued" offline).
    let m = s.windows.get(&cid("c1")).and_then(|w| {
        w.messages()
            .iter()
            .find(|m| m.client_msg_id == Some(cmid("a")))
            .cloned()
    });
    assert!(m.is_none_or(|m| m.state == DeliveryState::Pending));
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::Connected { reconnect: true }),
    );
    assert_eq!(calls(&fx), vec![cmid("a")]);
    assert_eq!(item(&s, "a").unwrap().state, OutboxState::Sending);
}

fn texts(s: &WorkspaceState) -> Vec<(String, bool)> {
    s.windows[&cid("c1")]
        .messages()
        .iter()
        .map(|m| (m.text.to_string(), m.state == DeliveryState::Delivered))
        .collect()
}

/// Rapid sends: the server stamps each one when it gets there, later than the
/// local clock of the sends still queued behind it. Confirmed rows must not
/// jump below the pending ones; pending rows stay last, in send order.
#[test]
fn pending_rows_stay_below_confirmed_ones() {
    let mut s = state(true);
    for (i, id) in ["a", "b", "c", "d"].iter().enumerate() {
        send(&mut s, id, 10 + i as u32);
    }
    // `a` lands after `b`, `c` and `d` were typed.
    finish(&mut s, "a", Ok(delivered("a", "20.000001")));
    // `b` echoes over the socket before its HTTP reply.
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(delivered("b", "21.000001"))),
    );
    let want = |v: &[(&str, bool)]| -> Vec<(String, bool)> {
        v.iter().map(|(t, d)| (t.to_string(), *d)).collect()
    };
    assert_eq!(
        texts(&s),
        want(&[("t-a", true), ("t-b", true), ("t-c", false), ("t-d", false)])
    );
    finish(&mut s, "b", Ok(delivered("b", "21.000001")));
    finish(&mut s, "c", Ok(delivered("c", "22.000001")));
    assert_eq!(
        texts(&s),
        want(&[("t-a", true), ("t-b", true), ("t-c", true), ("t-d", false)])
    );
    // A message from someone else, newer than every pending local ts, also
    // goes above the pending row.
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(msg(
            "c1",
            "23.000000",
            "other",
        ))),
    );
    finish(&mut s, "d", Ok(delivered("d", "24.000001")));
    assert_eq!(
        texts(&s),
        want(&[
            ("t-a", true),
            ("t-b", true),
            ("t-c", true),
            ("other", true),
            ("t-d", true)
        ])
    );
}

fn row(s: &WorkspaceState, text: &str) -> (String, DeliveryState) {
    let m = s.windows[&cid("c1")]
        .messages()
        .iter()
        .find(|m| &*m.text == text)
        .cloned()
        .unwrap();
    (m.text.to_string(), m.state)
}

fn order(s: &WorkspaceState) -> Vec<String> {
    s.windows[&cid("c1")]
        .messages()
        .iter()
        .map(|m| m.text.to_string())
        .collect()
}

/// A 429 inside a burst: the provider already waited out the server's hold
/// (and holds the next call too), so the outbox resends at once, in order,
/// with no timer of its own on top.
#[test]
fn rate_limited_send_retries_at_once_in_order() {
    let mut s = state(false);
    send(&mut s, "a", 10);
    send(&mut s, "b", 11);
    connect(&mut s); // a backlog drains one at a time
    let fx = finish(&mut s, "a", Err(ProviderError::RateLimited));
    assert!(!fx.iter().any(|e| matches!(e, Effect::RetryOutbox { .. })));
    assert_eq!(calls(&fx), [cmid("a")], "a goes again before b");
    assert_eq!(item(&s, "a").unwrap().state, OutboxState::Sending);
    let fx = finish(&mut s, "a", Ok(delivered("a", "20.000001")));
    assert_eq!(calls(&fx), [cmid("b")]);
}

/// A provider that keeps answering "rate limited" at once does not spin:
/// after a few immediate resends the outbox falls back to its timer.
#[test]
fn repeated_rate_limits_fall_back_to_the_timer() {
    let mut s = state(true);
    send(&mut s, "a", 10);
    let mut armed = false;
    for _ in 0..4 {
        let fx = finish(&mut s, "a", Err(ProviderError::RateLimited));
        if fx.iter().any(|e| matches!(e, Effect::RetryOutbox { .. })) {
            assert!(calls(&fx).is_empty());
            armed = true;
            break;
        }
    }
    assert!(armed);
}

#[test]
fn failed_row_stays_below_new_deliveries() {
    let mut s = state(true);
    send(&mut s, "a", 10);
    finish(&mut s, "a", Err(ProviderError::NotAllowed("no".into())));
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(msg(
            "c1",
            "20.000000",
            "other",
        ))),
    );
    assert_eq!(order(&s), ["other", "t-a"]);
    assert!(matches!(row(&s, "t-a").1, DeliveryState::Failed(_)));
}

#[test]
fn retry_keeps_send_order() {
    let mut s = state(true);
    send(&mut s, "a", 10);
    send(&mut s, "b", 11);
    finish(&mut s, "a", Err(ProviderError::NotAllowed("no".into())));
    // b is in flight now; a waits to be retried.
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(msg(
            "c1",
            "30.000000",
            "other",
        ))),
    );
    reduce(&mut s, Input::Retry(cmid("a")));
    assert_eq!(order(&s), ["other", "t-a", "t-b"]);
    finish(&mut s, "b", Ok(delivered("b", "31.000001")));
    assert_eq!(order(&s), ["other", "t-b", "t-a"]);
    finish(&mut s, "a", Ok(delivered("a", "32.000001")));
    assert_eq!(order(&s), ["other", "t-b", "t-a"]);
}

#[test]
fn restored_after_restart_row_stays_at_the_bottom() {
    let mut s = state(false);
    reduce(
        &mut s,
        Input::OutboxLoaded(vec![OutboxItem {
            id: cmid("a"),
            conversation: cid("c1"),
            thread: None,
            broadcast: false,
            text: Arc::from("t-a"),
            created_ms: 10_000,
            attempts: 0,
            state: OutboxState::Sending,
        }]),
    );
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: domain::HistoryPage {
                messages: vec![msg("c1", "5.000000", "old"), msg("c1", "20.000000", "new")],
                has_more: false,
            },
        },
    );
    assert_eq!(order(&s), ["old", "new", "t-a"]);
    assert!(matches!(row(&s, "t-a").1, DeliveryState::Failed(_)));
}

fn no_retry(fx: &[Effect]) -> bool {
    !fx.iter().any(|e| matches!(e, Effect::RetryOutbox { .. }))
}

/// After a's echo frees the slot and b goes out, a's late HTTP answer (any
/// outcome) must not touch b's slot or start c.
#[test]
fn late_answer_of_an_echoed_send_leaves_the_next_in_flight() {
    for late in [
        Ok(delivered("a", "101.000001")),
        Err(ProviderError::Network("down".into())),
        Err(ProviderError::NotAllowed("no".into())),
    ] {
        let mut s = state(false);
        send(&mut s, "a", 10);
        send(&mut s, "b", 20);
        send(&mut s, "c", 30);
        connect(&mut s); // a backlog drains one at a time
        let fx = reduce(
            &mut s,
            Input::Provider(ProviderEvent::MessagePosted(delivered("a", "101.000001"))),
        );
        assert_eq!(calls(&fx), [cmid("b")]);
        let fx = finish(&mut s, "a", late);
        assert!(calls(&fx).is_empty());
        assert!(no_retry(&fx));
        assert_eq!(item(&s, "c").unwrap().state, OutboxState::Queued);
        assert_eq!(s.outbox_inflight(), [cmid("b")]);
    }
}

/// A duplicate echo of a settled send, without a client id, is not b's echo.
/// With a window that is not live the echo is never held as a delivered copy,
/// so only the settled record tells the duplicate apart.
#[test]
fn duplicate_echo_of_a_settled_send_does_not_settle_the_next() {
    let mut s = state(true);
    for (id, secs) in [("a", 10), ("b", 20)] {
        reduce(
            &mut s,
            Input::Send {
                conversation: cid("c1"),
                text: Arc::from("same"),
                thread: None,
                id: cmid(id),
                now: ts(&format!("{secs}.000000")),
                now_ms: i64::from(secs) * 1000,
            },
        );
    }
    s.windows
        .get_mut(&cid("c1"))
        .unwrap()
        .set_has_more_after(true);
    let mut echo = delivered("a", "101.000001");
    echo.text = Arc::from("same");
    let mut bare = echo.clone();
    bare.client_msg_id = None;
    reduce(&mut s, Input::Provider(ProviderEvent::MessagePosted(echo)));
    assert_eq!(item(&s, "b").unwrap().state, OutboxState::Sending);
    let fx = reduce(&mut s, Input::Provider(ProviderEvent::MessagePosted(bare)));
    assert_eq!(removed(&fx, "b"), 0);
    assert_eq!(item(&s, "b").unwrap().state, OutboxState::Sending);
}

/// A send backlog while the socket is down waits for the provider's own
/// backoff reconnect (there is no effect that hurries it): nothing starts on a
/// send, a retry, the outbox timer or a requeue, and the first `Connected`
/// sends the oldest item. Hurrying after every drop made a server that hangs up
/// at once a reconnect storm; only the user's "Retry now" skips the wait.
#[test]
fn an_offline_backlog_waits_for_the_connected_event() {
    let mut s = state(true);
    reduce(&mut s, Input::Provider(ProviderEvent::Disconnected));
    assert!(calls(&send(&mut s, "a", 10)).is_empty());
    assert!(calls(&send(&mut s, "b", 11)).is_empty());
    assert!(calls(&reduce(&mut s, Input::OutboxRetry)).is_empty());
    assert_eq!(item(&s, "a").unwrap().state, OutboxState::Queued);
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::Connected { reconnect: true }),
    );
    assert_eq!(calls(&fx).len(), 1);
}

#[test]
fn an_offline_retry_waits_for_the_connected_event() {
    let mut s = state(true);
    send(&mut s, "a", 10);
    finish(&mut s, "a", Err(ProviderError::Other("x".into())));
    reduce(&mut s, Input::Provider(ProviderEvent::Disconnected));
    let fx = reduce(&mut s, Input::Retry(cmid("a")));
    assert!(calls(&fx).is_empty());
    assert_eq!(item(&s, "a").unwrap().state, OutboxState::Queued);
}

#[test]
fn offline_network_error_requeues_and_waits() {
    let mut s = state(true);
    send(&mut s, "a", 10);
    reduce(&mut s, Input::Provider(ProviderEvent::Disconnected));
    let fx = finish(&mut s, "a", Err(ProviderError::Network("down".into())));
    assert_eq!(item(&s, "a").unwrap().state, OutboxState::Queued);
    assert!(calls(&fx).is_empty());
}

/// Owner report: five messages typed back to back stayed pending (grey) for
/// seconds, each waiting for the round trip of the one before it. With the
/// server answering every send in 200 ms, all five turn sent about one round
/// trip after they were typed, not five round trips later.
#[test]
fn a_burst_of_five_sends_settles_in_about_one_round_trip() {
    const RTT_MS: u64 = 200;
    const GAP_MS: u64 = 10;
    let ids = ["a", "b", "c", "d", "e"];
    let mut s = state(true);
    // (answer due at, id) of every call the outbox started.
    let mut flying: Vec<(u64, ClientMsgId)> = Vec::new();
    for (k, id) in ids.iter().enumerate() {
        let now = k as u64 * GAP_MS;
        let fx = send(&mut s, id, 10);
        flying.extend(calls(&fx).into_iter().map(|c| (now + RTT_MS, c)));
    }
    let mut sent_at = std::collections::HashMap::new();
    let mut n = 0;
    while let Some(pos) = (0..flying.len()).min_by_key(|&i| flying[i].0) {
        let (now, id) = flying.remove(pos);
        n += 1;
        let name = ids.iter().find(|i| cmid(i) == id).unwrap();
        let fx = finish(&mut s, name, Ok(delivered(name, &format!("100.00000{n}"))));
        sent_at.insert(name.to_string(), now);
        flying.extend(calls(&fx).into_iter().map(|c| (now + RTT_MS, c)));
    }
    for id in ids {
        assert_eq!(
            row(&s, &format!("t-{id}")).1,
            DeliveryState::Delivered,
            "{id}"
        );
    }
    assert_eq!(order(&s), ["t-a", "t-b", "t-c", "t-d", "t-e"]);
    let last = sent_at.values().max().copied().unwrap();
    assert!(
        last <= 4 * GAP_MS + RTT_MS,
        "the last send settled at {last} ms: {sent_at:?}"
    );
}
