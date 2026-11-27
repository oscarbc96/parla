use std::sync::Arc;

use domain::test_support::{cid, conv, mid, msg, uid};
use domain::{
    Bootstrap, Effect, EmojiRef, HistoryPage, Input, MessageId, ProviderError, ProviderEvent,
    WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn thumbs() -> EmojiRef {
    EmojiRef {
        shortcode: Arc::from("+1"),
        unicode: Some(Arc::from("\u{1F44D}")),
        skin_tone: None,
    }
}

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.self_user = Some(uid("me"));
    s.conversations.insert(cid("c1"), conv("c1", "general"));
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![msg("c1", "1.000000", "a")],
                has_more: false,
            },
        },
    );
    s
}

fn id() -> MessageId {
    mid("c1", "1.000000")
}

fn react(s: &mut WorkspaceState, add: bool) -> Vec<Effect> {
    reduce(
        s,
        Input::React {
            id: id(),
            emoji: thumbs(),
            add,
        },
    )
}

fn changed(s: &mut WorkspaceState, user: &str, added: bool) -> Vec<Effect> {
    reduce(
        s,
        Input::Provider(ProviderEvent::ReactionChanged {
            id: id(),
            emoji: thumbs(),
            user: uid(user),
            added,
        }),
    )
}

fn rx(s: &WorkspaceState) -> Vec<domain::Reaction> {
    s.windows[&cid("c1")].messages()[0].reactions.to_vec()
}

#[test]
fn reaction_toggle_optimistic_and_rollback() {
    let mut s = state();
    let fx = react(&mut s, true);
    let r = rx(&s);
    assert_eq!((r[0].count, r[0].me), (1, true));
    assert_eq!(&*r[0].users, &[uid("me")]);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::CallReact { add: true, .. }))
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(domain::PersistOp::Reactions { .. })))
    );
    reduce(
        &mut s,
        Input::ReactFinished {
            id: id(),
            emoji: thumbs(),
            add: true,
            result: Err(ProviderError::Network("x".into())),
        },
    );
    assert!(rx(&s).is_empty(), "failed add is rolled back");

    // Remove then fail: restored.
    react(&mut s, true);
    react(&mut s, false);
    assert!(rx(&s).is_empty());
    reduce(
        &mut s,
        Input::ReactFinished {
            id: id(),
            emoji: thumbs(),
            add: false,
            result: Err(ProviderError::Other("x".into())),
        },
    );
    assert_eq!((rx(&s)[0].count, rx(&s)[0].me), (1, true));
}

#[test]
fn reaction_echo_is_idempotent() {
    let mut s = state();
    react(&mut s, true);
    changed(&mut s, "me", true);
    let r = rx(&s);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].count, 1);
    assert_eq!(r[0].users.len(), 1);
    // Echo of own removal after an optimistic removal is also a no-op.
    react(&mut s, false);
    changed(&mut s, "me", false);
    assert!(rx(&s).is_empty());
}

#[test]
fn reaction_count_never_negative() {
    let mut s = state();
    changed(&mut s, "u2", false);
    changed(&mut s, "me", false);
    assert!(rx(&s).is_empty());
}

#[test]
fn others_reactions_add_users() {
    let mut s = state();
    changed(&mut s, "u2", true);
    changed(&mut s, "u3", true);
    changed(&mut s, "u3", true);
    let r = rx(&s);
    assert_eq!((r[0].count, r[0].me), (2, false));
    assert_eq!(&*r[0].users, &[uid("u2"), uid("u3")]);
    changed(&mut s, "u2", false);
    assert_eq!(rx(&s)[0].count, 1);
    changed(&mut s, "u3", false);
    assert!(rx(&s).is_empty(), "count 0 removes the reaction");
}

#[test]
fn custom_emoji_changed_refetches() {
    let mut s = state();
    let fx = reduce(&mut s, Input::Provider(ProviderEvent::CustomEmojiChanged));
    assert!(fx.iter().any(|e| matches!(e, Effect::FetchCustomEmoji)));
    reduce(&mut s, Input::CustomEmojiLoaded(vec![]));
    assert!(s.custom_emoji.is_empty());
}

#[test]
fn bootstrap_loads_custom_emoji_when_supported() {
    let boot = || Bootstrap {
        self_user: uid("me"),
        workspace_name: Arc::from("Acme"),
        conversations: vec![],
        members: vec![],
    };
    let mut s = state();
    let fx = reduce(&mut s, Input::Bootstrapped(boot()));
    assert!(!fx.iter().any(|e| matches!(e, Effect::FetchCustomEmoji)));
    let mut s = state();
    s.caps.custom_emoji = true;
    let fx = reduce(&mut s, Input::Bootstrapped(boot()));
    assert!(fx.iter().any(|e| matches!(e, Effect::FetchCustomEmoji)));
}
