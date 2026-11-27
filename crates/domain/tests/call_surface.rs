use std::sync::Arc;

use domain::test_support::{Call, FakeProvider, cid, uid};
use domain::{CallSurface, Capabilities, ChatProvider, ProviderError, SecretText, SurfaceCookie};
use futures::executor::block_on;

fn surface(secret: &str) -> CallSurface {
    CallSurface {
        url: Arc::from("https://app.example.test/huddle/c1"),
        title: Arc::from("Huddle in #design"),
        cookies: vec![SurfaceCookie {
            name: Arc::from("session"),
            value: SecretText::new(secret),
            domain: Arc::from(".example.test"),
            path: Arc::from("/"),
            secure: true,
            http_only: true,
        }],
        allowed_host_suffixes: Arc::from([Arc::from("example.test")]),
        signed_out_paths: Arc::from([Arc::from("/signin")]),
        user_agent: None,
    }
}

#[test]
fn secret_text_debug_is_redacted() {
    let s = SecretText::new("abc123");
    let d = format!("{s:?}");
    assert!(!d.contains("abc123"));
    assert_eq!(d, "SecretText(<redacted>)");
    assert_eq!(s.expose(), "abc123");
}

#[test]
fn surface_debug_is_redacted() {
    let d = format!("{:?}", surface("v4lue"));
    assert!(!d.contains("v4lue"));
    assert!(d.contains("redacted"));
}

#[test]
fn default_call_surface_is_unsupported() {
    let f = FakeProvider::new("w1", uid("me"));
    f.set_call_surface(|| Ok(surface("x")));
    let r = block_on(f.call_surface(&cid("c1")));
    assert!(matches!(r, Err(ProviderError::Unsupported)));
    assert!(f.calls().is_empty());
}

#[test]
fn default_leave_is_ok() {
    let f = FakeProvider::new("w1", uid("me"));
    assert!(block_on(f.leave_huddle(&cid("c1"))).is_ok());
}

#[test]
fn fake_scripts_surface_and_logs_calls() {
    let f = FakeProvider::new("w1", uid("me")).with_capabilities(Capabilities {
        call_surface: true,
        ..Capabilities::default()
    });
    f.set_call_surface(|| Ok(surface("v4lue")));
    let a = block_on(f.call_surface(&cid("c1"))).unwrap();
    let b = block_on(f.call_surface(&cid("c2"))).unwrap();
    assert_eq!(a.cookies[0].value.expose(), "v4lue");
    assert_eq!(b.title.as_ref(), "Huddle in #design");
    block_on(f.leave_huddle(&cid("c1"))).unwrap();
    assert_eq!(
        f.calls(),
        vec![
            Call::CallSurface(cid("c1")),
            Call::CallSurface(cid("c2")),
            Call::LeaveHuddle(cid("c1")),
        ]
    );
    f.set_call_surface(|| Err(ProviderError::Other("boom".into())));
    assert!(block_on(f.call_surface(&cid("c1"))).is_err());
}

#[test]
fn provider_stays_object_safe() {
    let p: Arc<dyn ChatProvider> = Arc::new(FakeProvider::new("w1", uid("me")));
    assert_eq!(p.workspace().id.0, "w1");
}
