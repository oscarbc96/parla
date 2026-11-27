//! Who is typing where (incoming `Typing` events, their expiry and bounds) and
//! the throttle on the outbound typing signal.

use std::time::{Duration, Instant};

use domain::test_support::{cid, member, msg, ts, uid};
use domain::{
    Capabilities, Changed, Connection, Effect, Input, ProviderEvent, TYPERS_MAX, TYPING_PLACES_MAX,
    TYPING_SEND_EVERY, TYPING_TTL, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state() -> WorkspaceState {
    let mut st = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 100);
    st.self_user = Some(uid("u1"));
    st.members.insert(uid("u1"), member("u1", "Me"));
    st.caps = Capabilities {
        typing: true,
        ..Default::default()
    };
    st.connection = Connection::Online;
    st
}

/// The shell sends the clock before each provider batch.
fn typing_at(
    st: &mut WorkspaceState,
    now: Instant,
    c: &str,
    u: &str,
    thread: Option<&str>,
) -> Vec<Effect> {
    reduce(st, Input::Clock { unix: 1, mono: now });
    reduce(
        st,
        Input::Provider(ProviderEvent::Typing {
            conversation: cid(c),
            user: uid(u),
            thread: thread.map(ts),
        }),
    )
}

fn who(st: &WorkspaceState, c: &str, thread: Option<&str>) -> Vec<String> {
    let t = thread.map(ts);
    st.typers(&cid(c), t.as_ref())
        .iter()
        .map(|t| t.user.0.to_string())
        .collect()
}

fn changed(fx: &[Effect], c: &str) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Changed(Changed::Typing(x)) if *x == cid(c)))
}

fn wake(fx: &[Effect]) -> Option<Option<Instant>> {
    fx.iter().find_map(|e| match e {
        Effect::TypingWakeAt(at) => Some(*at),
        _ => None,
    })
}

fn sends(fx: &[Effect]) -> Vec<(String, Option<String>)> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::SendTyping {
                conversation,
                thread,
            } => Some((
                conversation.0.to_string(),
                thread.as_ref().map(|t| t.0.to_string()),
            )),
            _ => None,
        })
        .collect()
}

fn typed(st: &mut WorkspaceState, c: &str, thread: Option<&str>, now: Instant) -> Vec<Effect> {
    reduce(
        st,
        Input::Typed {
            conversation: cid(c),
            thread: thread.map(ts),
            now,
        },
    )
}

#[test]
fn a_typer_is_kept_since_their_first_event_and_signals_the_conversation() {
    let mut st = state();
    let t0 = Instant::now();
    let fx = typing_at(&mut st, t0, "c1", "u2", None);
    assert_eq!(who(&st, "c1", None), ["u2"]);
    assert!(changed(&fx, "c1"));
    assert_eq!(wake(&fx), Some(Some(t0 + TYPING_TTL)));
    let typer = &st.typers(&cid("c1"), None)[0];
    assert_eq!(typer.since, t0);
    assert!(typer.active(t0 + TYPING_TTL - Duration::from_millis(1)));
    assert!(!typer.active(t0 + TYPING_TTL));
}

#[test]
fn a_repeated_event_keeps_since_moves_the_deadline_and_repaints_nothing() {
    let mut st = state();
    let t0 = Instant::now();
    typing_at(&mut st, t0, "c1", "u2", None);
    let t1 = t0 + Duration::from_secs(3);
    let fx = typing_at(&mut st, t1, "c1", "u2", None);
    assert!(!changed(&fx, "c1"), "the same typer shows the same line");
    assert_eq!(wake(&fx), Some(Some(t1 + TYPING_TTL)));
    let typer = &st.typers(&cid("c1"), None)[0];
    assert_eq!((typer.since, typer.last), (t0, t1));
}

#[test]
fn thread_typers_are_kept_apart_from_the_conversation() {
    let mut st = state();
    let t0 = Instant::now();
    typing_at(&mut st, t0, "c1", "u2", Some("5.000000"));
    typing_at(&mut st, t0, "c1", "u3", None);
    assert_eq!(who(&st, "c1", Some("5.000000")), ["u2"]);
    assert_eq!(who(&st, "c1", None), ["u3"]);
}

#[test]
fn own_typing_is_ignored() {
    let mut st = state();
    let fx = typing_at(&mut st, Instant::now(), "c1", "u1", None);
    assert!(who(&st, "c1", None).is_empty());
    assert!(fx.is_empty());
}

#[test]
fn the_expiry_timer_drops_typers_and_cancels_itself() {
    let mut st = state();
    let t0 = Instant::now();
    typing_at(&mut st, t0, "c1", "u2", None);
    typing_at(&mut st, t0 + Duration::from_secs(2), "c1", "u3", None);
    let fx = reduce(
        &mut st,
        Input::TypingExpired {
            now: t0 + TYPING_TTL,
        },
    );
    assert_eq!(who(&st, "c1", None), ["u3"]);
    assert!(changed(&fx, "c1"));
    assert_eq!(
        wake(&fx),
        Some(Some(t0 + Duration::from_secs(2) + TYPING_TTL))
    );
    let fx = reduce(
        &mut st,
        Input::TypingExpired {
            now: t0 + Duration::from_secs(2) + TYPING_TTL,
        },
    );
    assert!(who(&st, "c1", None).is_empty());
    assert!(changed(&fx, "c1"));
    assert!(
        matches!(wake(&fx), None | Some(None)),
        "nothing is left to wake for"
    );
}

#[test]
fn an_expired_typer_is_pruned_on_the_next_event() {
    let mut st = state();
    let t0 = Instant::now();
    typing_at(&mut st, t0, "c1", "u2", None);
    let fx = typing_at(&mut st, t0 + TYPING_TTL, "c2", "u3", None);
    assert!(who(&st, "c1", None).is_empty());
    assert!(changed(&fx, "c1"));
    assert_eq!(who(&st, "c2", None), ["u3"]);
}

#[test]
fn typers_are_bounded_per_place_and_in_places() {
    let mut st = state();
    let t0 = Instant::now();
    for i in 0..TYPERS_MAX + 4 {
        typing_at(&mut st, t0, "c1", &format!("u{}", i + 10), None);
    }
    let held = who(&st, "c1", None);
    assert_eq!(held.len(), TYPERS_MAX);
    assert_eq!(held[0], "u14", "the oldest typers go first");
    for i in 0..TYPING_PLACES_MAX + 4 {
        let at = t0 + Duration::from_millis(i as u64 + 1);
        typing_at(&mut st, at, &format!("c{}", i + 10), "u2", None);
    }
    assert!(
        who(&st, "c1", None).is_empty(),
        "the oldest place goes first"
    );
    assert_eq!(
        who(&st, &format!("c{}", TYPING_PLACES_MAX + 13), None),
        ["u2"]
    );
}

#[test]
fn a_posted_message_ends_its_authors_typing() {
    let mut st = state();
    let t0 = Instant::now();
    typing_at(&mut st, t0, "c1", "u2", None);
    typing_at(&mut st, t0, "c1", "u3", None);
    let mut m = msg("c1", "10.000000", "done");
    m.author = Some(uid("u2"));
    let fx = reduce(&mut st, Input::Provider(ProviderEvent::MessagePosted(m)));
    assert_eq!(who(&st, "c1", None), ["u3"]);
    assert!(changed(&fx, "c1"));
}

#[test]
fn disconnecting_and_lean_mode_clear_typers() {
    let mut st = state();
    let t0 = Instant::now();
    typing_at(&mut st, t0, "c1", "u2", None);
    let fx = reduce(&mut st, Input::Provider(ProviderEvent::Disconnected));
    assert!(who(&st, "c1", None).is_empty());
    assert!(changed(&fx, "c1"));
    assert_eq!(wake(&fx), Some(None));

    let mut st = state();
    typing_at(&mut st, t0, "c1", "u2", None);
    reduce(&mut st, Input::SetLean(true));
    assert!(who(&st, "c1", None).is_empty());
    let fx = typing_at(&mut st, t0, "c1", "u3", None);
    assert!(who(&st, "c1", None).is_empty(), "lean keeps no typers");
    assert!(fx.is_empty());
}

#[test]
fn outbound_typing_is_throttled_per_conversation() {
    let mut st = state();
    let t0 = Instant::now();
    assert_eq!(
        sends(&typed(&mut st, "c1", None, t0)),
        [("c1".into(), None)]
    );
    let t1 = t0 + Duration::from_secs(1);
    assert!(!st.typing_due(&cid("c1"), t1));
    assert!(st.typing_due(&cid("c2"), t1));
    assert!(sends(&typed(&mut st, "c1", None, t1)).is_empty());
    assert!(
        sends(&typed(&mut st, "c1", Some("5.000000"), t1)).is_empty(),
        "a thread shares its conversation's throttle"
    );
    assert_eq!(
        sends(&typed(&mut st, "c2", Some("5.000000"), t1)),
        [("c2".into(), Some("5.000000".into()))]
    );
    assert!(st.typing_due(&cid("c1"), t0 + TYPING_SEND_EVERY));
    assert_eq!(
        sends(&typed(&mut st, "c1", None, t0 + TYPING_SEND_EVERY)),
        [("c1".into(), None)]
    );
}

#[test]
fn outbound_typing_needs_the_capability_a_connection_and_a_shown_workspace() {
    let t0 = Instant::now();
    let mut st = state();
    st.caps.typing = false;
    assert!(sends(&typed(&mut st, "c1", None, t0)).is_empty());
    let mut st = state();
    st.connection = Connection::Offline;
    assert!(sends(&typed(&mut st, "c1", None, t0)).is_empty());
    let mut st = state();
    reduce(&mut st, Input::SetLean(true));
    assert!(sends(&typed(&mut st, "c1", None, t0)).is_empty());
}
