use std::sync::Arc;

use domain::test_support::{Call, FakeProvider, cid, uid};
use domain::{Capabilities, ChatProvider, NativeCallJoin, ProviderError};
use futures::executor::block_on;

#[test]
fn capability_is_off_by_default() {
    assert!(!Capabilities::default().native_calls);
}

#[test]
fn default_native_call_join_is_unsupported() {
    let f = FakeProvider::new("w1", uid("me"));
    let r = block_on(f.native_call_join(&cid("c1")));
    assert!(matches!(r, Err(ProviderError::Unsupported)));
    assert!(f.calls().is_empty());
}

#[test]
fn fake_returns_deterministic_join_and_logs_calls() {
    let f = FakeProvider::new("w1", uid("me")).with_capabilities(Capabilities {
        native_calls: true,
        ..Capabilities::default()
    });
    let a = block_on(f.native_call_join(&cid("c1"))).unwrap();
    let b = block_on(f.native_call_join(&cid("c2"))).unwrap();
    assert_eq!(a.meeting_id.as_ref(), "meeting-c1");
    assert_eq!(a.attendee_id.as_ref(), "attendee-me");
    assert_eq!(a.external_user_id.as_ref(), "me");
    assert_eq!(a.join_token.expose(), "join-token-c1");
    assert_eq!(b.join_token.expose(), "join-token-c2");
    assert_eq!(
        a.signaling_url.as_ref(),
        "wss://signal.example.test/control/meeting-c1"
    );
    assert_eq!(a.audio_host_url.as_ref(), "audio.example.test:443");
    assert_eq!(
        a.turn_control_url.as_deref(),
        Some("https://turn.example.test/v2/turn_sessions")
    );
    assert_eq!(
        f.calls(),
        vec![
            Call::NativeCallJoin(cid("c1")),
            Call::NativeCallJoin(cid("c2")),
        ]
    );
}

#[test]
fn join_debug_hides_the_token() {
    let j = NativeCallJoin {
        meeting_id: Arc::from("m"),
        attendee_id: Arc::from("a"),
        external_user_id: Arc::from("e"),
        join_token: domain::SecretText::new("t0ken-value"),
        signaling_url: Arc::from("wss://s.example.test/control/m"),
        audio_host_url: Arc::from("a.example.test:443"),
        turn_control_url: None,
    };
    let d = format!("{j:?}");
    assert!(!d.contains("t0ken-value"));
    assert!(d.contains("redacted"));
}
