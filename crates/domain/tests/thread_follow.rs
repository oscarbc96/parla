//! Following a thread: the user's toggle, the provider's word and replying.

use domain::test_support::{cid, cmid, conv, mid, msg, ts, uid};
use domain::{
    Capabilities, Changed, Effect, HistoryPage, Input, Message, ProviderCall, ProviderError,
    ProviderEvent, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state(follow: bool) -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.self_user = Some(uid("me"));
    s.caps = Capabilities {
        threads: true,
        thread_follow: follow,
        ..Capabilities::default()
    };
    s.conversations.insert(cid("c1"), conv("c1", "general"));
    s
}

fn root() -> domain::MessageId {
    mid("c1", "1.000000")
}

fn reply_by(t: &str, author: &str) -> Message {
    let mut m = msg("c1", t, "r");
    m.author = Some(uid(author));
    m.thread_root = Some(ts("1.000000"));
    m
}

fn load(s: &mut WorkspaceState, replies: Vec<Message>) {
    reduce(s, Input::OpenThread(root()));
    let mut messages = vec![msg("c1", "1.000000", "root")];
    messages.extend(replies);
    reduce(
        s,
        Input::RepliesLoaded {
            root: root(),
            page: HistoryPage {
                messages,
                has_more: false,
            },
        },
    );
}

fn toggle(s: &mut WorkspaceState, follow: bool) -> Vec<Effect> {
    reduce(
        s,
        Input::SetThreadFollowed {
            root: root(),
            follow,
        },
    )
}

#[test]
fn a_thread_the_user_wrote_in_is_followed() {
    let mut s = state(true);
    load(&mut s, vec![reply_by("2.000000", "u3")]);
    assert!(!s.thread_followed(&root()));
    let mut s = state(true);
    load(&mut s, vec![reply_by("2.000000", "me")]);
    assert!(s.thread_followed(&root()));
}

#[test]
fn toggle_applies_at_once_and_asks_the_provider() {
    let mut s = state(true);
    load(&mut s, vec![]);
    let fx = toggle(&mut s, true);
    assert!(s.thread_followed(&root()));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Thread(r)) if *r == root()))
    );
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Call(ProviderCall::SetThreadFollowed { root: r, follow: true, last_read: None }) if *r == root()
    )));
}

#[test]
fn a_refused_toggle_rolls_back() {
    let mut s = state(true);
    load(&mut s, vec![]);
    toggle(&mut s, true);
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::SetThreadFollowed {
                root: root(),
                follow: true,
                last_read: None,
            },
            result: Err(ProviderError::Network("down".into())),
        },
    );
    assert!(!s.thread_followed(&root()));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Thread(r)) if *r == root()))
    );
}

#[test]
fn without_the_capability_nothing_happens() {
    let mut s = state(false);
    load(&mut s, vec![]);
    let fx = toggle(&mut s, true);
    assert!(!s.thread_followed(&root()));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Call(_))));
}

#[test]
fn the_provider_reports_follow_changes() {
    let mut s = state(true);
    load(&mut s, vec![reply_by("2.000000", "me")]);
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::ThreadFollowChanged {
            root: root(),
            followed: false,
        }),
    );
    assert!(!s.thread_followed(&root()));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Thread(r)) if *r == root()))
    );
}

#[test]
fn replying_follows_again() {
    let mut s = state(true);
    load(&mut s, vec![]);
    toggle(&mut s, false);
    reply(&mut s, "x1");
    sent(&mut s, "x1", true);
    assert!(s.thread_followed(&root()));
}

#[test]
fn known_states_stay_bounded() {
    let mut s = state(true);
    for i in 0..5_000 {
        reduce(
            &mut s,
            Input::Provider(ProviderEvent::ThreadFollowChanged {
                root: mid("c1", &format!("{i}.000000")),
                followed: true,
            }),
        );
    }
    assert!(s.thread_follow_len() <= domain::MAX_THREAD_FOLLOW);
}

fn finish(s: &mut WorkspaceState, follow: bool, ok: bool) -> Vec<Effect> {
    reduce(
        s,
        Input::CallFinished {
            call: ProviderCall::SetThreadFollowed {
                root: root(),
                follow,
                last_read: None,
            },
            result: if ok {
                Ok(domain::CallOutput::None)
            } else {
                Err(ProviderError::Network("down".into()))
            },
        },
    )
}

#[test]
fn an_older_refusal_does_not_undo_a_newer_toggle() {
    let mut s = state(true);
    load(&mut s, vec![]);
    toggle(&mut s, true);
    toggle(&mut s, false);
    toggle(&mut s, true);
    finish(&mut s, true, false);
    assert!(s.thread_followed(&root()), "the newest toggle stands");
    finish(&mut s, false, true);
    finish(&mut s, true, true);
    assert!(s.thread_followed(&root()));
}

#[test]
fn a_refusal_rolls_back_to_the_last_known_state() {
    let mut s = state(true);
    load(&mut s, vec![]);
    toggle(&mut s, true);
    toggle(&mut s, false);
    finish(&mut s, true, true);
    finish(&mut s, false, false);
    assert!(
        s.thread_followed(&root()),
        "the server follows after the first call"
    );
}

#[test]
fn refused_then_accepted() {
    let mut s = state(true);
    load(&mut s, vec![]);
    toggle(&mut s, true);
    finish(&mut s, true, false);
    assert!(!s.thread_followed(&root()));
    toggle(&mut s, true);
    finish(&mut s, true, true);
    assert!(s.thread_followed(&root()));
}

#[test]
fn a_provider_event_during_the_call_wins_over_its_refusal() {
    let mut s = state(true);
    load(&mut s, vec![]);
    toggle(&mut s, true);
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::ThreadFollowChanged {
            root: root(),
            followed: true,
        }),
    );
    finish(&mut s, true, false);
    assert!(s.thread_followed(&root()));
}

#[test]
fn the_bound_drops_the_oldest_and_keeps_calls_in_flight() {
    let mut s = state(true);
    load(&mut s, vec![]);
    toggle(&mut s, true);
    for i in 2..600 {
        reduce(
            &mut s,
            Input::Provider(ProviderEvent::ThreadFollowChanged {
                root: mid("c9", &format!("{i}.000000")),
                followed: true,
            }),
        );
    }
    assert!(s.thread_follow_len() <= domain::MAX_THREAD_FOLLOW);
    assert!(s.thread_follow_len() >= domain::MAX_THREAD_FOLLOW - 1);
    assert!(s.thread_followed(&root()), "the toggle in flight is kept");
    // The newest closed states are kept, the oldest dropped.
    let newest = mid("c9", "599.000000");
    let oldest = mid("c9", "2.000000");
    assert!(s.thread_follow_known(&newest));
    assert!(!s.thread_follow_known(&oldest));
    finish(&mut s, true, false);
    assert!(!s.thread_followed(&root()));
}

fn reply(s: &mut WorkspaceState, id: &str) {
    reduce(
        s,
        Input::Reply {
            root: root(),
            text: "me too".into(),
            broadcast: false,
            id: cmid(id),
            now: ts("5.000000"),
            now_ms: 5_000,
        },
    );
}

fn sent(s: &mut WorkspaceState, id: &str, ok: bool) {
    let mut m = reply_by("6.000000", "me");
    m.client_msg_id = Some(cmid(id));
    reduce(
        s,
        Input::SendFinished {
            conversation: cid("c1"),
            id: cmid(id),
            result: if ok {
                Ok(m)
            } else {
                Err(ProviderError::NotAllowed("no".into()))
            },
        },
    );
}

#[test]
fn a_reply_follows_again_only_once_it_is_sent() {
    let mut s = state(true);
    load(&mut s, vec![]);
    toggle(&mut s, false);
    finish(&mut s, false, true);
    reply(&mut s, "x2");
    assert!(!s.thread_followed(&root()), "not before the send succeeds");
    sent(&mut s, "x2", false);
    assert!(!s.thread_followed(&root()), "a failed send does not follow");
    reduce(&mut s, Input::Discard(cmid("x2")));
    assert!(!s.thread_followed(&root()));
    reply(&mut s, "x3");
    sent(&mut s, "x3", true);
    assert!(s.thread_followed(&root()));
}

#[test]
fn the_toggle_carries_the_thread_read_cursor() {
    let mut s = state(true);
    load(
        &mut s,
        vec![reply_by("2.000000", "u3"), reply_by("3.000000", "u3")],
    );
    let fx = toggle(&mut s, true);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Call(ProviderCall::SetThreadFollowed { last_read: Some(t), .. })
            if *t == ts("3.000000")
    )));
}
