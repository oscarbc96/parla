//! `HelperCore` over a recording fake webview: no GPUI, no window.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use calls::helper::{CallWebview, HelperCore, HelperOut};
use calls::{CaptureState, HelperMsg, MediaKind, ParentMsg, WireCookie, WireSurface};
use domain::SecretText;
use url::Url;

#[derive(Default)]
struct Log {
    calls: Vec<String>,
    cookie_values: Vec<String>,
    fail_cookies: bool,
    mute_supported: bool,
}

#[derive(Clone, Default)]
struct FakeWebview(Rc<RefCell<Log>>);

impl CallWebview for FakeWebview {
    fn set_cookies(&self, c: &[WireCookie]) -> Result<(), String> {
        let mut l = self.0.borrow_mut();
        l.calls.push(format!("cookies:{}", c.len()));
        l.cookie_values
            .extend(c.iter().map(|c| c.value.expose().to_owned()));
        if l.fail_cookies {
            Err("denied".into())
        } else {
            Ok(())
        }
    }
    fn load(&self, url: &str) -> Result<(), String> {
        self.0.borrow_mut().calls.push(format!("load:{url}"));
        Ok(())
    }
    fn set_muted(&self, kind: MediaKind, muted: bool) -> bool {
        let mut l = self.0.borrow_mut();
        l.calls.push(format!("mute:{kind:?}:{muted}"));
        l.mute_supported
    }
}

fn surface() -> WireSurface {
    WireSurface {
        url: Arc::from("https://app.example.test/h"),
        title: Arc::from("Huddle"),
        cookies: vec![WireCookie {
            name: Arc::from("d"),
            value: SecretText::new("sekrit-value"),
            domain: Arc::from(".example.test"),
            path: Arc::from("/"),
            secure: true,
            http_only: true,
        }],
        allowed_host_suffixes: vec![Arc::from("example.test")],
        signed_out_paths: vec![Arc::from("/signin")],
        user_agent: None,
    }
}

fn opened() -> (HelperCore<FakeWebview>, FakeWebview) {
    let w = FakeWebview::default();
    let mut core = HelperCore::new(w.clone());
    let out = core.on_parent(ParentMsg::Open(surface()));
    assert!(out.contains(&HelperOut::Send(HelperMsg::Ready)), "{out:?}");
    (core, w)
}

fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[test]
fn open_sets_cookies_before_load() {
    let (_core, w) = opened();
    assert_eq!(
        w.0.borrow().calls,
        vec![
            "cookies:1".to_string(),
            "load:https://app.example.test/h".to_string()
        ]
    );
    assert_eq!(w.0.borrow().cookie_values, vec!["sekrit-value".to_string()]);
}

#[test]
fn cookie_failure_reports_and_exits() {
    let w = FakeWebview::default();
    w.0.borrow_mut().fail_cookies = true;
    let mut core = HelperCore::new(w.clone());
    let out = core.on_parent(ParentMsg::Open(surface()));
    assert!(matches!(
        out.as_slice(),
        [HelperOut::Send(HelperMsg::Error(_)), HelperOut::Exit]
    ));
    // The error text never carries the cookie.
    assert!(!format!("{out:?}").contains("sekrit-value"));
    assert!(!w.0.borrow().calls.iter().any(|c| c.starts_with("load:")));
}

#[test]
fn surface_secret_zeroized_after_set() {
    let (core, _w) = opened();
    // The core keeps the host policy only; the wire surface (and with it the
    // cookie values) was consumed by `Open`.
    assert!(!format!("{core:?}").contains("sekrit-value"));
}

#[test]
fn second_open_is_error() {
    let (mut core, w) = opened();
    let n = w.0.borrow().calls.len();
    let out = core.on_parent(ParentMsg::Open(surface()));
    assert!(matches!(
        out.as_slice(),
        [HelperOut::Send(HelperMsg::Error(_))]
    ));
    assert_eq!(
        w.0.borrow().calls.len(),
        n,
        "nothing is set or loaded again"
    );
}

#[test]
fn open_with_off_allowlist_url_is_error_and_exits() {
    let w = FakeWebview::default();
    let mut core = HelperCore::new(w.clone());
    let mut s = surface();
    s.url = Arc::from("https://evil.test/h");
    let out = core.on_parent(ParentMsg::Open(s));
    assert!(matches!(
        out.as_slice(),
        [HelperOut::Send(HelperMsg::Error(_)), HelperOut::Exit]
    ));
    assert!(w.0.borrow().calls.is_empty());
}

#[test]
fn navigation_follows_policy() {
    let (mut core, _w) = opened();
    let (allow, out) = core.on_navigation(&u("https://app.example.test/x?q=1"));
    assert!(allow && out.is_empty(), "{out:?}");

    let ext = u("https://other.test/page?secret=1");
    let (allow, out) = core.on_navigation(&ext);
    assert!(!allow);
    assert_eq!(out, vec![HelperOut::OpenExternal(ext)]);

    let (allow, out) = core.on_navigation(&u("javascript:alert(1)"));
    assert!(!allow && out.is_empty(), "{out:?}");
    let (allow, _) = core.on_navigation(&u("https://example.test.evil.test/"));
    assert!(!allow);
}

#[test]
fn signed_out_redirect_reported_once() {
    let (mut core, _w) = opened();
    let (allow, out) = core.on_navigation(&u("https://app.example.test/signin?next=1"));
    assert!(allow);
    assert_eq!(out, vec![HelperOut::Send(HelperMsg::SignedOut)]);
    let out = core.on_page_loaded(&u("https://app.example.test/signin"));
    assert!(out.is_empty(), "{out:?}");
    let (_, out) = core.on_navigation(&u("https://app.example.test/signin/x"));
    assert!(out.is_empty(), "{out:?}");
}

#[test]
fn loaded_reports_host_and_path_only() {
    let (mut core, _w) = opened();
    let out = core.on_page_loaded(&u("https://app.example.test/h/1?token=abc#frag"));
    assert_eq!(
        out,
        vec![HelperOut::Send(HelperMsg::Loaded {
            host: "app.example.test".into(),
            path: "/h/1".into()
        })]
    );
    assert!(!format!("{out:?}").contains("token"));
    // The blank start page and off-list pages are not "loaded".
    assert!(core.on_page_loaded(&u("about:blank")).is_empty());
    assert!(core.on_page_loaded(&u("https://other.test/")).is_empty());
}

#[test]
fn permission_denied_off_allowlist() {
    let (core, _w) = opened();
    assert!(core.on_permission(&u("https://app.example.test/")));
    assert!(!core.on_permission(&u("https://evilexample.test/")));
    assert!(!core.on_permission(&u("http://app.example.test/")));
}

#[test]
fn set_muted_reports_capture_or_unsupported() {
    let (mut core, w) = opened();
    w.0.borrow_mut().mute_supported = true;
    let out = core.on_parent(ParentMsg::SetMuted {
        kind: MediaKind::Microphone,
        muted: true,
    });
    assert!(
        out.is_empty(),
        "the capture observer reports the change: {out:?}"
    );
    w.0.borrow_mut().mute_supported = false;
    let out = core.on_parent(ParentMsg::SetMuted {
        kind: MediaKind::Microphone,
        muted: true,
    });
    assert_eq!(
        out,
        vec![HelperOut::Send(HelperMsg::Capture {
            kind: MediaKind::Microphone,
            state: CaptureState::Unsupported
        })]
    );
    let out = core.on_capture(MediaKind::Camera, CaptureState::Active);
    assert_eq!(
        out,
        vec![HelperOut::Send(HelperMsg::Capture {
            kind: MediaKind::Camera,
            state: CaptureState::Active
        })]
    );
}

#[test]
fn helper_exits_on_stdin_eof() {
    let (mut core, _w) = opened();
    assert_eq!(core.on_stdin_eof(), vec![HelperOut::Exit]);
    let mut fresh = HelperCore::new(FakeWebview::default());
    assert_eq!(fresh.on_stdin_eof(), vec![HelperOut::Exit]);
}

#[test]
fn leave_and_close_exit() {
    let (mut core, _w) = opened();
    assert_eq!(core.on_parent(ParentMsg::Leave), vec![HelperOut::Exit]);
    let (mut core, _w) = opened();
    assert_eq!(
        core.on_window_closed(),
        vec![HelperOut::Send(HelperMsg::WindowClosed), HelperOut::Exit]
    );
}

#[test]
fn show_activates() {
    let (mut core, _w) = opened();
    assert_eq!(core.on_parent(ParentMsg::Show), vec![HelperOut::Activate]);
}
