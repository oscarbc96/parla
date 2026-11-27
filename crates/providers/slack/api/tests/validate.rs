use slack_api::validate::{is_conversation_id, is_id, is_slack_host, is_team_id};

#[test]
fn ids_are_prefixed_upper_case_alphanumerics() {
    assert!(is_team_id("T0000001"));
    assert!(!is_team_id("T"));
    assert!(!is_team_id("t0000001"));
    assert!(!is_team_id("T1/../x"));
    assert!(is_id("E123", 'E'));
    assert!(!is_id("T123", 'E'));
}

#[test]
fn conversation_ids_accept_any_prefix_but_not_empty_or_symbols() {
    assert!(is_conversation_id("C0000005"));
    assert!(is_conversation_id("D1"));
    assert!(!is_conversation_id(""));
    assert!(!is_conversation_id("c1"));
    assert!(!is_conversation_id("C1/x"));
}

#[test]
fn slack_hosts_are_the_domain_and_its_subdomains() {
    assert!(is_slack_host("slack.com"));
    assert!(is_slack_host("app.slack.com"));
    assert!(!is_slack_host("evilslack.com"));
    assert!(!is_slack_host("slack.com.evil.example"));
}
