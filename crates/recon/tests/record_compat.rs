//! A directory written by `redact::record::Recorder` loads in recon like any capture.

use std::sync::Arc;

use recon::shape::ExchangeKind;
use redact::FakeRules as Rules;
use redact::record::{Direction, Recorder};

#[test]
fn recon_diff_reads_recording() {
    let dir = tempfile::tempdir().unwrap();
    let rules = Arc::new(Rules::default());
    let rec = Recorder::new(dir.path(), rules.clone()).unwrap();
    rec.record_http(
        "POST",
        "https://acme.chat.example.test/rpc/conversations.history?_x=1",
        &[("content-type".into(), "multipart/form-data".into())],
        &[
            ("token".into(), "fake-abc123".into()),
            ("channel".into(), "C0123ABCD45".into()),
        ],
        200,
        r#"{"ok":true,"messages":[{"ts":"1.2","text":"hi"}]}"#,
    );
    rec.record_ws(Direction::Receive, r#"{"type":"hello"}"#);

    let groups = recon::extract::extract(dir.path()).unwrap();
    let http = groups
        .iter()
        .find(|g| g.name == "conversations.history")
        .unwrap();
    assert_eq!(http.kind, ExchangeKind::Http);
    assert!(http.params.contains("channel"));
    assert!(!http.params.contains("token"));
    assert!(
        groups
            .iter()
            .any(|g| g.name == "hello" && g.kind == ExchangeKind::Ws)
    );
    assert!(
        recon::extract::leaks_in(&groups, &*rules)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn record_disabled_in_release() {
    const ON: bool = redact::record::enabled();
    assert_eq!(ON, cfg!(debug_assertions));
    let rules = Arc::new(Rules::default());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    assert_eq!(
        Recorder::from_value(Some(path), rules.clone()).is_some(),
        cfg!(debug_assertions)
    );
    assert!(Recorder::from_value(None, rules.clone()).is_none());
    assert!(Recorder::from_value(Some(""), rules).is_none());
}
