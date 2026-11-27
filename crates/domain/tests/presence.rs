//! Other people's presence: the watched set, `Presence` events and opening a
//! direct message with someone we have none with.

use domain::test_support::{conv, member, uid};
use domain::{
    CallOutput, Capabilities, Changed, ConversationKind, Effect, Input, ProviderCall,
    ProviderEvent, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state() -> WorkspaceState {
    let mut st = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 100);
    st.self_user = Some(uid("u1"));
    st.members.insert(uid("u1"), member("u1", "Me"));
    st.caps = Capabilities {
        presence: true,
        presence_watch: true,
        open_direct: true,
        ..Default::default()
    };
    st
}

fn watch(st: &mut WorkspaceState, users: &[&str]) -> Vec<Effect> {
    reduce(
        st,
        Input::WatchPresence(users.iter().map(|u| uid(u)).collect()),
    )
}

fn calls_watch(fx: &[Effect]) -> Option<Vec<domain::UserId>> {
    fx.iter().find_map(|e| match e {
        Effect::Call(ProviderCall::WatchPresence(v)) => Some(v.clone()),
        _ => None,
    })
}

fn presence_event(st: &mut WorkspaceState, u: &str, active: bool) -> Vec<Effect> {
    reduce(
        st,
        Input::Provider(ProviderEvent::Presence {
            user: uid(u),
            active,
        }),
    )
}

#[test]
fn watching_asks_the_provider_once_per_change() {
    let mut st = state();
    let fx = watch(&mut st, &["u3", "u2", "u2"]);
    assert_eq!(calls_watch(&fx), Some(vec![uid("u2"), uid("u3")]));
    // The same set again is a no-op.
    assert!(calls_watch(&watch(&mut st, &["u2", "u3"])).is_none());
    // Without the capability nothing goes out and nothing is kept.
    let mut off = state();
    off.caps.presence_watch = false;
    assert!(calls_watch(&watch(&mut off, &["u2"])).is_none());
}

#[test]
fn presence_events_are_kept_for_watched_users_only() {
    let mut st = state();
    watch(&mut st, &["u2"]);
    let fx = presence_event(&mut st, "u2", true);
    assert_eq!(st.presence.get(&uid("u2")), Some(&true));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Presence)))
    );
    // Same value: no change signal.
    let fx = presence_event(&mut st, "u2", true);
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Presence)))
    );
    // Flipping signals again.
    let fx = presence_event(&mut st, "u2", false);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Presence)))
    );
    // An unwatched user is ignored (the map stays bounded).
    presence_event(&mut st, "u9", true);
    assert!(!st.presence.contains_key(&uid("u9")));
}

#[test]
fn leaving_the_watched_set_forgets_presence() {
    let mut st = state();
    watch(&mut st, &["u2", "u3"]);
    presence_event(&mut st, "u2", true);
    presence_event(&mut st, "u3", true);
    let fx = watch(&mut st, &["u3"]);
    assert!(!st.presence.contains_key(&uid("u2")));
    assert!(st.presence.contains_key(&uid("u3")));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Presence)))
    );
}

#[test]
fn opened_direct_message_joins_the_conversations() {
    let mut st = state();
    let mut dm = conv("d9", "");
    dm.kind = ConversationKind::Direct;
    dm.peer = Some(uid("u2"));
    let id = dm.id.clone();
    let fx = reduce(
        &mut st,
        Input::CallFinished {
            call: ProviderCall::OpenDirect(uid("u2")),
            result: Ok(CallOutput::Conversation(dm)),
        },
    );
    let c = st.conversations.get(&id).expect("conversation inserted");
    assert!(c.is_open && c.peer == Some(uid("u2")));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Conversations)))
    );
}
