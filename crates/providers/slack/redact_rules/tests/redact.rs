//! Synthetic data only. Every "real" value below is made up.

use std::sync::Arc;

use redact::{IdMap, LeakKind, Redactor, check_text};
use serde_json::{Value, json};
use slack_redact::Rules;

fn redactor() -> Redactor {
    Redactor::new(IdMap::new(), Arc::new(Rules::default()))
}

#[test]
fn tokens_and_cookies_removed() {
    let mut r = redactor();
    let out = r.redact_str("auth xoxc-1-2-3 cookie d=xoxd-abc%2Fdef; x=1 user xoxp-9-8-7-aa");
    assert!(!out.contains("xox"), "{out}");
    assert!(out.contains("REDACTED"));
    assert!(out.contains("x=1"));

    let mut form = vec![
        ("token".to_string(), "xoxc-1-2-3".to_string()),
        ("channel".to_string(), "C0123456789".to_string()),
    ];
    r.redact_form(&mut form);
    assert_eq!(form[0].1, "REDACTED");
    assert_eq!(form[1].1, "C0000001");

    let mut doc = json!({"token": "plain-secret", "nested": {"api_token": "abc", "ok": true}});
    r.redact_json(&mut doc);
    assert_eq!(doc["token"], "REDACTED");
    assert_eq!(doc["nested"]["api_token"], "REDACTED");
    assert_eq!(doc["nested"]["ok"], true);

    let url = r
        .redact_url("https://app.slack.com/api/x.y?token=xoxc-1-2-3&_x_id=abc&channel=C0123456789");
    assert!(
        !url.contains("xoxc") && !url.contains("C0123456789"),
        "{url}"
    );
    assert!(url.contains("token=REDACTED"), "{url}");
    assert!(url.starts_with("https://app.slack.com/api/x.y?"), "{url}");
}

#[test]
fn workspace_subdomains_rewritten() {
    let mut r = redactor();
    let out = r.redact_str(
        "https://acme.slack.com/x https://globex.enterprise.slack.com/y https://app.slack.com/z \
         wss://wss-primary.slack.com/link https://edgeapi.slack.com/a https://api.slack.com/b \
         https://enterprise.slack.com/c https://slack.com/d https://acme.slack.com/again",
    );
    assert!(out.contains("https://workspace.slack.com/x"), "{out}");
    assert!(
        out.contains("https://workspace2.enterprise.slack.com/y"),
        "{out}"
    );
    assert!(out.contains("https://workspace.slack.com/again"), "{out}");
    for kept in [
        "https://app.slack.com/z",
        "wss://wss-primary.slack.com/link",
        "https://edgeapi.slack.com/a",
        "https://api.slack.com/b",
        "https://enterprise.slack.com/c",
        "https://slack.com/d",
    ] {
        assert!(out.contains(kept), "{kept} missing in {out}");
    }
    assert!(!out.contains("acme") && !out.contains("globex"));
    // Idempotent on already-synthetic hosts.
    assert_eq!(r.redact_str(&out), out);

    let mut custom = redactor().with_host_suffix("chat.example.org");
    assert_eq!(
        custom.redact_str("https://corp.chat.example.org/q"),
        "https://workspace.chat.example.org/q"
    );
}

#[test]
fn pii_in_json_form_query() {
    let mut r = redactor();
    let mut doc = json!({
        "user": {"id": "U024BE7LH", "real_name": "Ada Lovelace", "email": "ada@corp.io",
                 "image_72": "https://avatars.slack-edge.com/2020/ada_72.png"},
        "message": {"text": "hello there <@U024BE7LH> friend", "permalink": "https://acme.slack.com/archives/C024BE7LH/p1700000000000100"},
        "U024BE7LH": {"x": 1},
    });
    r.redact_json(&mut doc);
    let s = doc.to_string();
    for secret in [
        "Ada",
        "Lovelace",
        "ada@corp.io",
        "slack-edge",
        "acme",
        "hello there",
        "C024BE7LH",
        "p1700000000000100",
    ] {
        assert!(!s.contains(secret), "{secret} left in {s}");
    }
    assert_eq!(doc["user"]["id"], "U0000001");
    assert_eq!(doc["U0000001"]["x"], 1, "object keys are mapped too");
    // Same word count, mention kept (mapped).
    let text = doc["message"]["text"].as_str().unwrap();
    assert_eq!(text.split_whitespace().count(), 4);
    assert!(text.contains("<@U0000001>"), "{text}");

    let mut form = vec![
        ("text".to_string(), "secret plan here".to_string()),
        ("email".to_string(), "bob@corp.io".to_string()),
        ("real_name".to_string(), "Bob Builder".to_string()),
        (
            "image_72".to_string(),
            "https://x.slack-edge.com/a.png".to_string(),
        ),
    ];
    r.redact_form(&mut form);
    assert_eq!(form[0].1.split_whitespace().count(), 3);
    assert!(!form[0].1.contains("secret"));
    assert!(form[1].1.ends_with("@example.test"));
    assert!(!form[2].1.contains("Bob"));
    assert!(form[3].1.starts_with("https://files.example.test/"));

    let url = r.redact_url(
        "/api/users.profile.set?text=top+secret&real_name=Eve%20Adams&email=eve%40corp.io",
    );
    assert!(
        !url.contains("secret") && !url.contains("Eve") && !url.contains("corp.io"),
        "{url}"
    );
}

#[test]
fn json_inside_form_string() {
    let mut r = redactor();
    let blocks = r#"[{"type":"rich_text","elements":[{"type":"rich_text_section","elements":[{"type":"text","text":"launch codes inside"},{"type":"user","user_id":"U024BE7LH"}]}]}]"#;
    let mut form = vec![("blocks".to_string(), blocks.to_string())];
    r.redact_form(&mut form);
    let out = &form[0].1;
    assert!(
        !out.contains("launch") && !out.contains("U024BE7LH"),
        "{out}"
    );
    let parsed: Value = serde_json::from_str(out).expect("still valid JSON");
    let section = &parsed[0]["elements"][0]["elements"];
    assert_eq!(section[1]["user_id"], "U0000001");
    assert_eq!(
        section[0]["text"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .count(),
        3
    );
}

#[test]
fn check_flags_leaks_without_values() {
    let planted = [
        ("a xoxc-1-2-3 b", LeakKind::Token),
        ("id T024BE7LD here", LeakKind::RealId),
        ("mail someone@corp.io now", LeakKind::Email),
        (
            "img https://avatars.slack-edge.com/x.png",
            LeakKind::FileUrl,
        ),
        ("dl https://files.slack.com/files-pri/x", LeakKind::FileUrl),
        ("host https://leak.slack.com/x", LeakKind::WorkspaceHost),
        (
            "host https://leak.enterprise.slack.com/x",
            LeakKind::WorkspaceHost,
        ),
    ];
    for (text, kind) in planted {
        let leaks = check_text(text, &Rules::default());
        assert_eq!(leaks.len(), 1, "{text}: {leaks:?}");
        assert_eq!(leaks[0].kind, kind, "{text}");
        let shown = format!("{} {:?}", leaks[0], leaks[0]);
        for value in ["xoxc", "T024BE7LD", "corp.io", "slack-edge", "leak"] {
            assert!(!shown.contains(value), "{value} printed: {shown}");
        }
    }
    let clean = "T0000001 U0000002 user1@example.test https://workspace.slack.com/x https://app.slack.com \
                 https://workspace2.enterprise.slack.com xoxc-... REDACTED https://files.example.test/file/1";
    assert!(
        check_text(clean, &Rules::default()).is_empty(),
        "{:?}",
        check_text(clean, &Rules::default())
    );
    let multi = check_text("ok\nsecond xoxp-1-2-3", &Rules::default());
    assert_eq!((multi[0].line, multi[0].col), (2, 8));
}

#[test]
fn rules_expose_slack_conventions() {
    use redact::RuleSet;
    let rules = Rules::default();
    assert_eq!(rules.id(), "slack");
    assert!(rules.keep_header("X-Slack-Req-Id") && !rules.keep_header("cookie"));
    let url = url::Url::parse("https://acme.slack.com/api/conversations.history?x=1").unwrap();
    assert_eq!(
        rules.method_name(&url).as_deref(),
        Some("conversations.history")
    );
    let other = url::Url::parse("https://acme.slack.com/archives/C1").unwrap();
    assert_eq!(rules.method_name(&other), None);
    let mention = rules.mention().unwrap();
    assert!(mention.is_match("<@U024BE7LH>") && mention.is_match("<#C024BE7LH|general>"));
    assert!(!mention.is_match("<@U024BE7LH>,"));
}

#[test]
fn reconnect_token_is_redacted_in_query_and_text() {
    let mut r = redactor();
    let frt = "Z9-synthetic_reconnect_token_0123456789abcdef%3D%3D%2F2";
    let url = r.redact_url(&format!(
        "wss://wss-primary.slack.com/?frt={frt}&sync_desync=1"
    ));
    assert!(!url.contains("synthetic_reconnect"), "{url}");
    assert!(url.contains("sync_desync=1"));

    let text = r.redact_str(&format!("wss://wss-primary.slack.com/?frt={frt}&x=1"));
    assert!(!text.contains("synthetic_reconnect"), "{text}");
    assert!(
        check_text(&format!("u ?frt={frt}&x=1"), &Rules::default())
            .iter()
            .any(|l| l.kind == LeakKind::Token)
    );
}

#[test]
fn camel_case_token_keys_are_redacted() {
    let mut r = redactor();
    let mut doc = json!({"attendee": {"JoinToken": "plain-secret", "joinToken": "abc"}});
    r.redact_json(&mut doc);
    assert_eq!(doc["attendee"]["JoinToken"], "REDACTED");
    assert_eq!(doc["attendee"]["joinToken"], "REDACTED");
}

#[test]
fn magic_login_values_and_slack_login_urls_are_redacted() {
    let rules = slack_redact::Rules::default();
    let mut doc = json!({"teams": [{
        "magic_login_code": "a1b2c3d4e5f6",
        "magic_login_url": "https://app.slack.com/t/acme/login/a1b2c3d4e5f6",
        "magic_login_browser_url": "https://app.slack.com/t/acme/login/a1b2c3d4e5f6",
    }]});
    let raw = serde_json::to_string_pretty(&doc).unwrap();
    assert!(
        check_text(&raw, &rules)
            .iter()
            .any(|l| l.kind == LeakKind::Token),
        "checker misses magic_login values"
    );
    let mut r = redactor();
    r.redact_json(&mut doc);
    let out = serde_json::to_string_pretty(&doc).unwrap();
    assert!(
        !out.contains("a1b2c3d4e5f6") && !out.contains("acme"),
        "{out}"
    );
    assert!(check_text(&out, &rules).is_empty(), "{out}");

    let text = "open https://app.slack.com/t/acme/login/a1b2c3d4e5f6 now";
    assert!(
        check_text(text, &rules)
            .iter()
            .any(|l| l.kind == LeakKind::WorkspaceHost || l.kind == LeakKind::Token)
    );
    let out = r.redact_str(text);
    assert_eq!(
        out,
        "open https://app.slack.com/t/workspace/login/REDACTED now"
    );
    assert!(check_text(&out, &rules).is_empty(), "{out}");
    // A keep-listed slug-like path such as a real app route stays readable.
    let ok = "https://app.slack.com/client/T0000001/C0000001";
    assert_eq!(r.redact_str(ok), ok);
}

#[test]
fn encoded_slack_login_url_forms_are_flagged_and_rewritten() {
    let rules = slack_redact::Rules::default();
    let forms = [
        "https%253A%252F%252Fapp.slack.com%252Ft%252Facme%252Flogin%252Fabc123",
        "app.slack.com/t/acme/login/abc123",
        "app.slack.com&#47;t&#47;acme&#47;login&#47;abc123",
        "app.slack.com&#x2F;t&#x2f;acme&#X2F;login&#x2F;abc123",
        "app.slack.com\\/t\\/acme\\/login\\/abc123",
        "app.slack.com%25252Ft%25252Facme",
        "app.slack.com\\u002Ft\\u002facme\\u002Flogin\\u002Fabc123",
        "app.slack.com/t/acme/login/ab.c~1%2B2%2b3",
    ];
    for raw in forms {
        assert!(!check_text(raw, &rules).is_empty(), "checker misses {raw}");
        let out = redactor().redact_str(raw);
        for secret in ["acme", "abc123", "ab.c", "~1", "2%2b3"] {
            assert!(!out.contains(secret), "{raw} -> {out}");
        }
        assert!(out.contains("workspace"), "{raw} -> {out}");
        let leaks = check_text(&out, &rules);
        assert!(leaks.is_empty(), "{out}: {leaks:?}");
    }
}

#[test]
fn raw_cookie_is_redacted_past_slash_plus_and_equals() {
    let mut r = redactor();
    for raw in [
        "xoxd-abc/def+ghi==",
        "xoxd-abc\\/def\\u002Bghi",
        "xoxd-a+b/c=d",
    ] {
        let text = format!("cookie={raw} end");
        let out = r.redact_str(&text);
        assert_eq!(out, "cookie=REDACTED end", "{raw}");
        assert!(!out.contains("def") && !out.contains("ghi"), "{out}");
    }
}
