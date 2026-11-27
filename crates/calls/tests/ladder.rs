use calls::{JoinRoute, ladder};
use settings::JoinWith;

fn w() -> Option<String> {
    Some("https://web.example.test/h".into())
}
fn a() -> Option<String> {
    Some("fake-app://h".into())
}
fn b() -> JoinRoute {
    JoinRoute::Browser("https://web.example.test/h".into())
}
fn p() -> JoinRoute {
    JoinRoute::ProviderApp("fake-app://h".into())
}

#[test]
fn ladder_order_per_setting() {
    assert_eq!(
        ladder(JoinWith::Window, false, true, w(), a()),
        vec![JoinRoute::Window, b(), p()]
    );
    assert_eq!(
        ladder(JoinWith::Window, false, false, w(), a()),
        vec![b(), p()]
    );
    assert_eq!(
        ladder(JoinWith::Browser, false, true, w(), a()),
        vec![b(), p()]
    );
    assert_eq!(
        ladder(JoinWith::ProviderApp, false, true, w(), a()),
        vec![p(), b()]
    );
    assert_eq!(
        ladder(JoinWith::Window, false, true, None, a()),
        vec![JoinRoute::Window, p()]
    );
    assert_eq!(
        ladder(JoinWith::ProviderApp, false, false, w(), None),
        vec![b()]
    );
}

#[test]
fn empty_ladder_when_nothing_available() {
    assert!(ladder(JoinWith::Window, false, false, None, None).is_empty());
    assert_eq!(
        ladder(JoinWith::Window, false, true, None, None),
        vec![JoinRoute::Window]
    );
    assert!(ladder(JoinWith::Browser, false, true, None, None).is_empty());
}

#[test]
fn native_leads_and_falls_to_window_browser_app() {
    assert_eq!(
        ladder(JoinWith::Native, true, true, w(), a()),
        vec![JoinRoute::Native, JoinRoute::Window, b(), p()]
    );
    assert_eq!(
        ladder(JoinWith::Native, false, true, w(), a()),
        vec![JoinRoute::Window, b(), p()]
    );
    assert_eq!(
        ladder(JoinWith::Native, true, false, w(), None),
        vec![JoinRoute::Native, b()]
    );
    // Other settings never pick native.
    assert_eq!(
        ladder(JoinWith::Window, true, true, w(), a()),
        vec![JoinRoute::Window, b(), p()]
    );
    assert_eq!(
        ladder(JoinWith::Browser, true, true, w(), a()),
        vec![b(), p()]
    );
}
