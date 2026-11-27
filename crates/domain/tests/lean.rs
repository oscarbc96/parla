use std::sync::Arc;

use domain::test_support::{cid, conv, msg, uid};
use domain::{
    Bootstrap, Changed, Effect, Input, Mention, Message, ProviderEvent, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![conv("C1", "general"), conv("C2", "random")],
            members: vec![],
        }),
    );
    s
}
fn post(s: &mut WorkspaceState, m: Message) -> Vec<Effect> {
    reduce(s, Input::Provider(ProviderEvent::MessagePosted(m)))
}
fn mention(mut m: Message) -> Message {
    m.mentions = Arc::from([Mention::User(uid("me"))]);
    m.author = Some(uid("other"));
    m
}

#[test]
fn going_lean_drops_windows_keeps_counts() {
    let mut s = state();
    reduce(&mut s, Input::Open(cid("C1")));
    post(&mut s, mention(msg("C1", "1.000001", "hi")));
    assert!(!s.windows.is_empty());
    assert_eq!(s.conversations[&cid("C1")].mentions, 1);
    let fx = reduce(&mut s, Input::SetLean(true));
    assert!(s.lean);
    assert!(s.windows.is_empty() && s.threads.is_empty());
    assert_eq!(s.conversations.len(), 2);
    assert_eq!(s.conversations[&cid("C1")].mentions, 1);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Lean)))
    );
}

#[test]
fn lean_keeps_counts_and_notifies() {
    let mut s = state();
    reduce(&mut s, Input::SetLean(true));
    let fx = post(&mut s, mention(msg("C1", "1.000001", "hi")));
    assert_eq!(s.conversations[&cid("C1")].mentions, 1);
    assert!(s.conversations[&cid("C1")].unread);
    assert!(fx.iter().any(|e| matches!(e, Effect::MaybeNotify { .. })));
    assert!(s.windows.is_empty());
}

#[test]
fn lean_never_marks_read() {
    let mut s = state();
    reduce(
        &mut s,
        Input::Focus {
            conversation: Some(cid("C1")),
            at_bottom: true,
            window_active: true,
        },
    );
    reduce(&mut s, Input::SetLean(true));
    let fx = post(&mut s, mention(msg("C1", "1.000001", "hi")));
    assert!(!fx.iter().any(|e| matches!(e, Effect::MarkRead { .. })));
    assert!(s.conversations[&cid("C1")].unread);
}

#[test]
fn waking_reloads_focused_conversation() {
    let mut s = state();
    reduce(
        &mut s,
        Input::Focus {
            conversation: Some(cid("C1")),
            at_bottom: true,
            window_active: true,
        },
    );
    reduce(&mut s, Input::SetLean(true));
    let fx = reduce(&mut s, Input::SetLean(false));
    assert!(!s.lean);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Reload(c) if *c == cid("C1")))
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Lean)))
    );
    // No-op when already awake.
    assert!(reduce(&mut s, Input::SetLean(false)).is_empty());
}
