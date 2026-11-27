use std::sync::Arc;

use calls::{
    CaptureState, HelperMsg, MAX_LINE, MediaKind, ParentMsg, ProtocolError, WireSurface,
    decode_helper, decode_parent, encode_helper, encode_parent,
};
use domain::{CallSurface, SecretText, SurfaceCookie};
use url::Url;

fn surface() -> WireSurface {
    CallSurface {
        url: Arc::from("https://app.example.test/huddle/c1"),
        title: Arc::from("Huddle in #design"),
        cookies: vec![SurfaceCookie {
            name: Arc::from("session"),
            value: SecretText::new("c00kie-v4lue"),
            domain: Arc::from(".example.test"),
            path: Arc::from("/"),
            secure: true,
            http_only: true,
        }],
        allowed_host_suffixes: Arc::from([Arc::from("example.test")]),
        signed_out_paths: Arc::from([Arc::from("/signin")]),
        user_agent: Some(Arc::from("UA/1.0")),
    }
    .into()
}

#[test]
fn every_message_round_trips() {
    for m in [
        ParentMsg::Open(surface()),
        ParentMsg::SetMuted {
            kind: MediaKind::Camera,
            muted: true,
        },
        ParentMsg::Show,
        ParentMsg::Leave,
    ] {
        let line = encode_parent(&m);
        assert!(!line.contains('\n'));
        let back = decode_parent(&line).unwrap();
        assert_eq!(encode_parent(&back), line);
    }
    let ParentMsg::Open(s) = decode_parent(&encode_parent(&ParentMsg::Open(surface()))).unwrap()
    else {
        panic!("open")
    };
    assert_eq!(s.cookies[0].value.expose(), "c00kie-v4lue");
    assert_eq!(s.user_agent.as_deref(), Some("UA/1.0"));

    for m in [
        HelperMsg::Ready,
        HelperMsg::Loaded {
            host: "a.example.test".into(),
            path: "/x".into(),
        },
        HelperMsg::SignedOut,
        HelperMsg::Capture {
            kind: MediaKind::Microphone,
            state: CaptureState::Muted,
        },
        HelperMsg::WindowClosed,
        HelperMsg::Error("boom".into()),
    ] {
        assert_eq!(decode_helper(&encode_helper(&m)).unwrap(), m);
    }
}

#[test]
fn open_message_debug_is_redacted() {
    let m = ParentMsg::Open(surface());
    let d = format!("{m:?}");
    assert!(!d.contains("c00kie-v4lue"), "{d}");
    assert!(d.contains("redacted"));
}

#[test]
fn helper_messages_never_carry_query() {
    let url = Url::parse("https://app.example.test/huddle/c1?token=x#frag").unwrap();
    let line = encode_helper(&HelperMsg::loaded(&url));
    assert!(!line.contains("token"), "{line}");
    assert!(!line.contains("frag"), "{line}");
    assert_eq!(
        HelperMsg::loaded(&url),
        HelperMsg::Loaded {
            host: "app.example.test".into(),
            path: "/huddle/c1".into()
        }
    );
}

#[test]
fn oversized_line_is_rejected() {
    let big = format!(r#"{{"type":"error","data":"{}"}}"#, "a".repeat(MAX_LINE));
    assert_eq!(decode_helper(&big), Err(ProtocolError::TooLong));
    assert!(matches!(decode_parent(&big), Err(ProtocolError::TooLong)));
}

#[test]
fn unknown_type_is_unknown_not_panic() {
    assert_eq!(
        decode_helper(r#"{"type":"future_thing"}"#),
        Err(ProtocolError::Unknown("future_thing".into()))
    );
    assert!(matches!(
        decode_parent(r#"{"type":"future_thing","x":1}"#),
        Err(ProtocolError::Unknown(t)) if t == "future_thing"
    ));
    assert_eq!(decode_helper("not json"), Err(ProtocolError::Invalid));
    assert_eq!(
        decode_helper(r#"{"type":"loaded"}"#),
        Err(ProtocolError::Invalid)
    );
}
