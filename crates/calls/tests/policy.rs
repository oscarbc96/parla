use std::sync::Arc;

use calls::{NavDecision, decide, host_allowed, is_signed_out, permit_media};
use url::Url;

fn sfx() -> Vec<Arc<str>> {
    vec![Arc::from("example.test")]
}
fn u(s: &str) -> Url {
    Url::parse(s).unwrap()
}

#[test]
fn allowlisted_https_loads() {
    let s = sfx();
    assert_eq!(decide(&u("https://example.test/x"), &s), NavDecision::Allow);
    assert_eq!(
        decide(&u("https://app.example.test/x"), &s),
        NavDecision::Allow
    );
    assert_eq!(
        decide(&u("https://A.B.Example.TEST/x?q=1"), &s),
        NavDecision::Allow
    );
}

#[test]
fn lookalike_hosts_open_externally() {
    let s = sfx();
    for url in [
        "https://evilexample.test/",
        "https://example.test.evil.test/",
        "https://xn--xample-9ua.test/",
        "https://example.test@evil.test/",
        "https://example.test.:443/",
        "https://user:pw@app.example.test/",
        "https://notexample.test/",
    ] {
        assert_eq!(decide(&u(url), &s), NavDecision::OpenExternally, "{url}");
    }
}

#[test]
fn non_https_is_blocked() {
    let s = sfx();
    for url in [
        "javascript:alert(1)",
        "data:text/html,hi",
        "file:///etc/passwd",
        "blob:https://app.example.test/abc",
        "ftp://app.example.test/",
        "about:srcdoc",
    ] {
        assert_eq!(decide(&u(url), &s), NavDecision::Block, "{url}");
    }
    assert_eq!(
        decide(&u("http://app.example.test/"), &s),
        NavDecision::OpenExternally
    );
    assert_eq!(
        decide(&u("mailto:a@b.test"), &s),
        NavDecision::OpenExternally
    );
}

#[test]
fn about_blank_allowed() {
    assert_eq!(decide(&u("about:blank"), &sfx()), NavDecision::Allow);
}

#[test]
fn empty_and_odd_suffixes_match_nothing() {
    let s: Vec<Arc<str>> = vec![Arc::from(""), Arc::from(".example.test"), Arc::from("a.")];
    assert!(!host_allowed("example.test", &s));
    assert!(!host_allowed("a", &s));
    assert!(!host_allowed("app.example.test.", &sfx()));
    assert!(!host_allowed("", &sfx()));
}

#[test]
fn signed_out_paths_match_prefix_only_on_allowlisted_host() {
    let s = sfx();
    let p: Vec<Arc<str>> = vec![Arc::from("/signin"), Arc::from("/workspace-signin")];
    assert!(is_signed_out(&u("https://app.example.test/signin"), &s, &p));
    assert!(is_signed_out(
        &u("https://app.example.test/signin/x?a=1"),
        &s,
        &p
    ));
    assert!(is_signed_out(
        &u("https://example.test/workspace-signin"),
        &s,
        &p
    ));
    assert!(!is_signed_out(
        &u("https://app.example.test/signinx"),
        &s,
        &p
    ));
    assert!(!is_signed_out(&u("https://app.example.test/other"), &s, &p));
    assert!(!is_signed_out(&u("https://evil.test/signin"), &s, &p));
    assert!(!is_signed_out(&u("http://app.example.test/signin"), &s, &p));
}

#[test]
fn media_only_for_allowlisted_origin() {
    let s = sfx();
    assert!(permit_media(&u("https://app.example.test"), &s));
    assert!(!permit_media(&u("https://evilexample.test"), &s));
    assert!(!permit_media(&u("http://app.example.test"), &s));
    assert!(!permit_media(&u("about:blank"), &s));
}
