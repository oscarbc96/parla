use slack_api::CredentialError;
use slack_auth::*;

// Fixtures hold `{{TOKEN_*}}` markers so no token-shaped string is committed; the
// synthetic tokens are filled in here.
const TOKEN: &str = "xoxc-0000000001-0000000002-0000000003-aaaa";
const TOKEN_B: &str = "xoxc-0000000001-0000000002-0000000003-bbbb";

fn fill(s: &str) -> String {
    s.replace("{{TOKEN_A}}", TOKEN)
        .replace("{{TOKEN_B}}", TOKEN_B)
        .replace("{{TOKEN_E}}", "xoxc-0000000001-0000000002-0000000003-eeee")
}

fn two() -> String {
    fill(include_str!("fixtures/local_config_two_teams.json"))
}
fn ent() -> String {
    fill(include_str!("fixtures/local_config_enterprise.json"))
}

#[test]
fn local_config_lists_all_teams() {
    let found = parse_local_config(&two()).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].name, "Acme");
    assert_eq!(found[0].workspace.id.0, "T0000001");
    assert_eq!(found[0].url.as_str(), "https://workspace1.slack.com/");
    assert_eq!(found[0].token.expose(), TOKEN);
    assert_eq!(found[1].name, "Beta Org");
    assert!(found[0].enterprise_id.is_none());
}

#[test]
fn local_config_enterprise_marked() {
    let found = parse_local_config(&ent()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].workspace.id.0, "T0000001");
    assert_eq!(found[0].enterprise_id.as_deref(), Some("E0000001"));
}

#[test]
fn local_config_without_tokens_is_no_teams() {
    let j = r#"{"teams":{"T0000003":{"id":"T0000003","name":"x","url":"https://x.example.slack.com/"}}}"#;
    assert!(matches!(parse_local_config(j), Err(AuthError::NoTeams)));
    assert!(matches!(
        parse_local_config("nope"),
        Err(AuthError::BadConfig)
    ));
}

#[test]
fn manual_accepts_tokens_on_lines_and_spaces() {
    let t = "xoxc-0000000001-0000000002-0000000003-aaaa\nxoxc-0000000001-0000000002-0000000003-bbbb  xoxc-0000000001-0000000002-0000000003-aaaa\n";
    let m = parse_manual("xoxd-abc", t).unwrap();
    assert_eq!(m.tokens.len(), 2);
    assert!(m.cookie.header_value().starts_with("d=xoxd-abc"));
}

#[test]
fn manual_rejects_bot_token() {
    let e = parse_manual("xoxd-abc", "xoxb-1-2-3").unwrap_err();
    assert!(matches!(e, AuthError::Credential(CredentialError::NotXoxc)));
    assert!(!e.to_string().contains("xoxb-1"));
}

#[test]
fn script_is_read_only() {
    for bad in [
        "setItem",
        "removeItem",
        "fetch",
        "XMLHttpRequest",
        "=",
        "clear",
    ] {
        assert!(!READ_TOKENS_SCRIPT.contains(bad), "script contains {bad}");
    }
    assert!(READ_TOKENS_SCRIPT.contains("localConfig_v2"));
    assert!(MANUAL_CONSOLE_SNIPPET.contains("localConfig_v2"));
}

#[test]
fn local_config_skips_workspace_urls_off_slack() {
    for bad in [
        "http://x.slack.com/",
        "https://evil.example/",
        "https://slack.com.evil.example/",
        "https://x.slack.com@evil.example/",
        "https://x.slack.com:8443/",
    ] {
        let j = format!(
            r#"{{"teams":{{"T0000003":{{"id":"T0000003","name":"x","url":"{bad}","token":"{TOKEN}"}}}}}}"#
        );
        assert!(
            matches!(parse_local_config(&j), Err(AuthError::NoTeams)),
            "{bad}"
        );
    }
    let ok = format!(
        r#"{{"teams":{{"T0000003":{{"id":"T0000003","name":"x","url":"https://x.enterprise.slack.com/","token":"{TOKEN}"}}}}}}"#
    );
    assert_eq!(parse_local_config(&ok).unwrap().len(), 1);
}
