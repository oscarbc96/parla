use slack_auth::webview::huddle_url;

#[test]
fn huddle_url_matches_captured_huddle_link() {
    let url = huddle_url("T0000001", "C0000005").unwrap();
    assert_eq!(
        url.as_str(),
        "https://app.slack.com/huddle/T0000001/C0000005"
    );
}

#[test]
fn huddle_url_rejects_ids_that_are_not_ids() {
    assert!(huddle_url("T1/../x", "C1").is_none());
    assert!(huddle_url("T1", "").is_none());
    assert!(huddle_url("t1", "C1").is_none());
}

#[test]
fn capture_toggle_keys_are_documented() {
    let src = include_str!("../examples/call_probe.rs");
    let doc: String = src
        .lines()
        .take_while(|l| l.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");
    for key in ["  M ", "  C ", "  s "] {
        assert!(doc.contains(key), "probe module doc lacks key {key:?}");
    }
    assert!(doc.contains("CALL_PROBE_PIPED"));
}
