//! Synthetic data only. Every "real" value below is made up.

use std::sync::Arc;

use redact::{FakeRules, IdMap, LeakKind, Redactor, check_text};
use serde_json::{Value, json};

fn redactor() -> Redactor {
    Redactor::new(IdMap::new(), Arc::new(FakeRules::default()))
}

#[test]
fn fake_rules_redact_fake_tokens() {
    let mut r = redactor();
    let out =
        r.redact_str("auth fake-aa-bb-cc and id A024BE7LH on https://acme.chat.example.test/x");
    assert!(
        !out.contains("fake-") && !out.contains("A024BE7LH") && !out.contains("acme"),
        "{out}"
    );
    assert!(
        out.contains("REDACTED") && out.contains("A0000001"),
        "{out}"
    );
    assert_eq!(r.rules().id(), "fake");
}

#[test]
fn tokens_and_cookies_removed() {
    let mut r = redactor();
    let out = r.redact_str("auth fake-1-2-3 cookie d=fake-abc%2Fdef; x=1 user fake-9-8-7-aa");
    assert!(!out.contains("fake-"), "{out}");
    assert!(out.contains("REDACTED"));
    assert!(out.contains("x=1"));

    let mut form = vec![
        ("token".to_string(), "fake-1-2-3".to_string()),
        ("channel".to_string(), "B0123456789".to_string()),
    ];
    r.redact_form(&mut form);
    assert_eq!(form[0].1, "REDACTED");
    assert_eq!(form[1].1, "B0000001");

    let mut doc = json!({"token": "plain-secret", "nested": {"api_token": "abc", "ok": true}});
    r.redact_json(&mut doc);
    assert_eq!(doc["token"], "REDACTED");
    assert_eq!(doc["nested"]["api_token"], "REDACTED");
    assert_eq!(doc["nested"]["ok"], true);

    let url = r.redact_url(
        "https://app.chat.example.test/api/x.y?token=fake-1-2-3&_x_id=abc&channel=B0123456789",
    );
    assert!(
        !url.contains("fake-") && !url.contains("B0123456789"),
        "{url}"
    );
    assert!(url.contains("token=REDACTED"), "{url}");
    assert!(
        url.starts_with("https://app.chat.example.test/api/x.y?"),
        "{url}"
    );
}

#[test]
fn ids_map_stably_keep_prefix() {
    let mut map = IdMap::new();
    assert_eq!(map.id("T024BE7LD"), "T0000001");
    assert_eq!(map.id("T024BE7LD"), "T0000001");
    assert_eq!(map.id("U024BE7LH"), "U0000001");
    assert_eq!(map.id("U024BE7LJ"), "U0000002");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("idmap.json");
    map.save(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let mut loaded = IdMap::load(&path).unwrap();
    assert_eq!(loaded.id("U024BE7LJ"), "U0000002");
    assert_eq!(loaded.id("U024BE7ZZ"), "U0000003");
    assert!(
        IdMap::load(&dir.path().join("missing.json"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn workspace_subdomains_rewritten() {
    let mut r = redactor();
    let out = r.redact_str(
        "https://acme.chat.example.test/x https://globex.org.chat.example.test/y https://app.chat.example.test/z \
         wss://ws.chat.example.test/link https://api.chat.example.test/b \
         https://org.chat.example.test/c https://chat.example.test/d https://acme.chat.example.test/again",
    );
    assert!(
        out.contains("https://workspace.chat.example.test/x"),
        "{out}"
    );
    assert!(
        out.contains("https://workspace2.org.chat.example.test/y"),
        "{out}"
    );
    assert!(
        out.contains("https://workspace.chat.example.test/again"),
        "{out}"
    );
    for kept in [
        "https://app.chat.example.test/z",
        "wss://ws.chat.example.test/link",
        "https://api.chat.example.test/b",
        "https://org.chat.example.test/c",
        "https://chat.example.test/d",
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
        "user": {"id": "A024BE7LH", "real_name": "Ada Lovelace", "email": "ada@corp.io",
                 "image_72": "https://cdn.example.test/2020/ada_72.png"},
        "message": {"text": "hello there <@A024BE7LH> friend", "permalink": "https://acme.chat.example.test/archives/B024BE7LH/p1700000000000100"},
        "A024BE7LH": {"x": 1},
    });
    r.redact_json(&mut doc);
    let s = doc.to_string();
    for secret in [
        "Ada",
        "Lovelace",
        "ada@corp.io",
        "cdn.example",
        "acme",
        "hello there",
        "B024BE7LH",
        "p1700000000000100",
    ] {
        assert!(!s.contains(secret), "{secret} left in {s}");
    }
    assert_eq!(doc["user"]["id"], "A0000001");
    assert_eq!(doc["A0000001"]["x"], 1, "object keys are mapped too");
    // Same word count, mention kept (mapped).
    let text = doc["message"]["text"].as_str().unwrap();
    assert_eq!(text.split_whitespace().count(), 4);
    assert!(text.contains("<@A0000001>"), "{text}");

    let mut form = vec![
        ("text".to_string(), "secret plan here".to_string()),
        ("email".to_string(), "bob@corp.io".to_string()),
        ("real_name".to_string(), "Bob Builder".to_string()),
        (
            "image_72".to_string(),
            "https://x.cdn.example.test/a.png".to_string(),
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
    let blocks = r#"[{"type":"rich_text","elements":[{"type":"rich_text_section","elements":[{"type":"text","text":"launch codes inside"},{"type":"user","user_id":"A024BE7LH"}]}]}]"#;
    let mut form = vec![("blocks".to_string(), blocks.to_string())];
    r.redact_form(&mut form);
    let out = &form[0].1;
    assert!(
        !out.contains("launch") && !out.contains("A024BE7LH"),
        "{out}"
    );
    let parsed: Value = serde_json::from_str(out).expect("still valid JSON");
    let section = &parsed[0]["elements"][0]["elements"];
    assert_eq!(section[1]["user_id"], "A0000001");
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
    let rules = FakeRules::default();
    let planted = [
        ("a fake-1-2-3 b", LeakKind::Token),
        ("id B024BE7LD here", LeakKind::RealId),
        ("mail someone@corp.io now", LeakKind::Email),
        ("img https://cdn.example.test/x.png", LeakKind::FileUrl),
        (
            "host https://leak.chat.example.test/x",
            LeakKind::WorkspaceHost,
        ),
        (
            "host https://leak.org.chat.example.test/x",
            LeakKind::WorkspaceHost,
        ),
    ];
    for (text, kind) in planted {
        let leaks = check_text(text, &rules);
        assert_eq!(leaks.len(), 1, "{text}: {leaks:?}");
        assert_eq!(leaks[0].kind, kind, "{text}");
        let shown = format!("{} {:?}", leaks[0], leaks[0]);
        for value in ["fake-", "B024BE7LD", "corp.io", "cdn.example", "leak"] {
            assert!(!shown.contains(value), "{value} printed: {shown}");
        }
    }
    let clean = "T0000001 U0000002 user1@example.test https://workspace.chat.example.test/x https://app.chat.example.test \
                 https://workspace2.org.chat.example.test fake-... REDACTED https://files.example.test/file/1";
    assert!(
        check_text(clean, &rules).is_empty(),
        "{:?}",
        check_text(clean, &rules)
    );
    let multi = check_text("ok\nsecond fake-1-2-3", &rules);
    assert_eq!((multi[0].line, multi[0].col), (2, 8));
}

fn link_metadata() -> Value {
    json!({"attachments": [{
        "title": "Quarterly tracker rollup for the platform team",
        "title_link": "https://tracker.invalid/issues/42?x=1",
        "from_url": "https://tracker.invalid/issues/42?x=1",
        "original_url": "https://docs.invalid/page",
        "footer": "Tracker board for platform work",
        "author_name": "Pat Example",
        "service_name": "Tracker Service",
        "fallback": "Quarterly tracker rollup for the platform team",
        "footer_icon": "https://icons.invalid/t.png",
        "blocks": [{"type": "section", "elements": [
            {"type": "link", "url": "https://tracker.invalid/issues/42?x=1"}
        ]}]
    }]})
}

#[test]
fn link_metadata_is_scrubbed() {
    let mut r = redactor();
    let mut v = link_metadata();
    r.redact_json(&mut v);
    let text = v.to_string();
    for needle in [
        "tracker",
        "docs.invalid",
        "icons.invalid",
        "platform",
        "Pat",
        "Service",
    ] {
        assert!(!text.contains(needle), "{needle} survived: {text}");
    }
    let a = &v["attachments"][0];
    // Stable per original within a capture.
    assert_eq!(a["title_link"], a["from_url"]);
    assert_eq!(a["title_link"], a["blocks"][0]["elements"][0]["url"]);
    assert_ne!(a["title_link"], a["original_url"]);
    assert!(
        a["title_link"]
            .as_str()
            .unwrap()
            .starts_with("https://example.com/")
    );
    assert!(check_text(&text, &FakeRules::default()).is_empty());
}

#[test]
fn check_flags_foreign_urls_and_free_text() {
    let rules = FakeRules::default();
    let urls = check_text("see https://tracker.invalid.io/x and more", &rules);
    assert_eq!(urls.len(), 1, "{urls:?}");
    assert_eq!(urls[0].kind, LeakKind::ForeignUrl);
    let raw = serde_json::to_string_pretty(&link_metadata()).unwrap();
    let kinds: Vec<_> = check_text(&raw, &rules).iter().map(|l| l.kind).collect();
    assert!(kinds.contains(&LeakKind::FreeText), "{kinds:?}");
    let ok = r#"{"title": "lorem ipsum dolor sit amet consectetur", "u": "https://example.com/3 http://localhost:3000/x https://a.example.test/x"}"#;
    assert!(
        check_text(ok, &rules).is_empty(),
        "{:?}",
        check_text(ok, &rules)
    );
}

#[test]
fn url_userinfo_is_redacted_and_flagged_on_loopback_hosts() {
    let text = "see https://user:secret@127.0.0.1:8080/x and http://bob:pw@localhost/y";
    let out = redactor().redact_str(text);
    assert!(!out.contains("secret") && !out.contains("bob:pw"), "{out}");
    let leaks = check_text(text, &FakeRules::default());
    assert_eq!(
        leaks
            .iter()
            .filter(|l| l.kind == LeakKind::ForeignUrl)
            .count(),
        2,
        "{leaks:?}"
    );
    // Plain loopback URLs stay allowed.
    let ok = "http://127.0.0.1:8080/x";
    assert_eq!(redactor().redact_str(ok), ok);
    assert!(check_text(ok, &FakeRules::default()).is_empty());
}

#[test]
fn escaped_and_encoded_workspace_hosts_are_rewritten() {
    let forms = [
        "redirect_uri=https%3A%2F%2Facme.chat.example.test%2Fsignin%2Fend&x=1",
        "app_domain%3Dhttps://acme.chat.example.test&y",
        r"https:\/\/acme.chat.example.test\/signin",
        r"\x3dhttps:\/\/acme.chat.example.test",
        "&quot;https://acme.chat.example.test&quot;",
        "redirect_uri=https%253A%252F%252Facme.chat.example.test%252Fsignin",
        "a=%2Facme.chat.example.test%2Fb",
        "a=%2facme.org.chat.example.test",
    ];
    for form in forms {
        let out = redactor().redact_str(form);
        assert!(!out.contains("acme"), "{form} -> {out}");
        let leaks = check_text(&out, &FakeRules::default());
        assert!(leaks.is_empty(), "{form} -> {out}: {leaks:?}");
    }
    // The rewrite keeps the surrounding encoding intact.
    let out = redactor().redact_str("u=https%3A%2F%2Facme.chat.example.test%2Fsignin");
    assert_eq!(out, "u=https%3A%2F%2Fworkspace.chat.example.test%2Fsignin");
}

#[test]
fn percent_escape_before_host_only_counts_for_non_label_characters() {
    let rules = FakeRules::default();
    // Leaks the first review found: hex that is really label text must not be an escape.
    for raw in [
        "x=%deapi.chat.example.test",
        "x=%25abapp.chat.example.test",
        "x=%25acme.chat.example.test",
        "x=100%acme.chat.example.test",
        "x=%2525acme.chat.example.test",
        "x=%41cme.chat.example.test",
    ] {
        assert!(
            check_text(raw, &rules)
                .iter()
                .any(|l| l.kind == LeakKind::WorkspaceHost),
            "checker misses {raw}"
        );
        let out = redactor().redact_str(raw);
        assert!(
            !out.contains("acme") && !out.contains("cme"),
            "{raw} -> {out}"
        );
        assert!(
            !out.contains("api.chat") && !out.contains("app.chat"),
            "{raw} -> {out}"
        );
        let leaks = check_text(&out, &rules);
        assert!(leaks.is_empty(), "{raw} -> {out}: {leaks:?}");
    }
}

#[test]
fn keep_list_host_after_escape_stays() {
    for raw in [
        "u=%2Fapp.chat.example.test%2Fx",
        "u=%252Fapi.chat.example.test",
    ] {
        assert_eq!(redactor().redact_str(raw), raw);
        assert!(check_text(raw, &FakeRules::default()).is_empty(), "{raw}");
    }
}

#[test]
fn escape_does_not_change_the_workspace_label_key() {
    let mut r = redactor();
    let a = r.redact_str("a=%2Facme.chat.example.test");
    let b = r.redact_str("b=acme.chat.example.test");
    assert_eq!(a, "a=%2Fworkspace.chat.example.test");
    assert_eq!(b, "b=workspace.chat.example.test");
    // Labels that start like hex stay whole: `2fa` and `ab` are their own workspaces.
    let c = r.redact_str("%2F2fa.chat.example.test %2Fab.chat.example.test");
    assert_eq!(
        c,
        "%2Fworkspace2.chat.example.test %2Fworkspace3.chat.example.test"
    );
}

#[test]
fn check_text_flags_raw_escaped_workspace_host() {
    let leaks = check_text(
        "redirect_uri=https%3A%2F%2Facme.chat.example.test%2Fend",
        &FakeRules::default(),
    );
    assert!(
        leaks.iter().any(|l| l.kind == LeakKind::WorkspaceHost),
        "{leaks:?}"
    );
}

#[test]
fn magic_login_values_are_replaced_and_flagged() {
    let mut r = redactor();
    let mut doc = json!({"teams": [{
        "magic_login_code": "a1b2c3d4e5f6",
        "magic_login_url": "https://app.chat.example.test/t/acme/login/a1b2c3d4e5f6",
        "magic_login_browser_url": "https://app.chat.example.test/t/acme/login/a1b2c3d4e5f6?x=1",
        "magic_login_extra": 123,
    }]});
    let raw = serde_json::to_string_pretty(&doc).unwrap();
    let rules = FakeRules::default();
    assert!(
        check_text(&raw, &rules)
            .iter()
            .any(|l| l.kind == LeakKind::Token),
        "checker misses magic_login values"
    );
    r.redact_json(&mut doc);
    let out = serde_json::to_string_pretty(&doc).unwrap();
    assert!(
        !out.contains("a1b2c3d4e5f6") && !out.contains("acme"),
        "{out}"
    );
    assert_eq!(doc["teams"][0]["magic_login_code"], "REDACTED");
    assert_eq!(doc["teams"][0]["magic_login_url"], "REDACTED");
    let leaks = check_text(&out, &rules);
    assert!(leaks.is_empty(), "{out}: {leaks:?}");
}

#[test]
fn login_urls_rewrite_the_slug_and_drop_the_code() {
    let rules = FakeRules::default();
    let raw = "go https://app.chat.example.test/t/acme/login/a1b2c3d4e5f6 and \
               https%3A%2F%2Fapp.chat.example.test%2Ft%2Facme%2Flogin%2Fa1b2c3d4e5f6 \
               https://app.chat.example.test/t/acme/ready";
    assert!(!check_text(raw, &rules).is_empty());
    let mut r = redactor();
    let out = r.redact_str(raw);
    assert!(
        !out.contains("acme") && !out.contains("a1b2c3d4e5f6"),
        "{out}"
    );
    assert!(
        out.contains("app.chat.example.test/t/workspace/login/REDACTED"),
        "{out}"
    );
    assert!(
        out.contains("app.chat.example.test/t/workspace/ready"),
        "{out}"
    );
    // The slug shares its label with the workspace host.
    assert_eq!(
        r.redact_str("https://acme.chat.example.test/x"),
        "https://workspace.chat.example.test/x"
    );
    let leaks = check_text(&out, &rules);
    assert!(leaks.is_empty(), "{out}: {leaks:?}");
    // Already redacted input is left alone.
    assert_eq!(redactor().redact_str(&out), out);
}

#[test]
fn encoded_login_url_forms_are_flagged_and_rewritten() {
    let rules = FakeRules::default();
    let forms = [
        "https%253A%252F%252Fapp.chat.example.test%252Ft%252Facme%252Flogin%252Fabc123",
        "app.chat.example.test/t/acme/login/abc123",
        "app.chat.example.test&#47;t&#47;acme&#47;login&#47;abc123",
        "app.chat.example.test&#x2F;t&#x2f;acme&#X2F;login&#x2F;abc123",
        "app.chat.example.test\\/t\\/acme\\/login\\/abc123",
        "app.chat.example.test%25252Ft%25252Facme",
        "app.chat.example.test\\u002Ft\\u002facme\\u002Flogin\\u002Fabc123",
        "app.chat.example.test/t/acme/login/ab.c~1%2B2%2b3",
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
fn allowed_host_url_query_secrets_are_redacted() {
    let mut r = redactor();
    let text = "see https://app.chat.example.test/x?api_key=hunter22&sig=abc123&page=2 now";
    assert!(
        check_text(text, r.rules())
            .iter()
            .any(|l| l.kind == LeakKind::Token),
        "scanner must flag the query secret"
    );
    let out = r.redact_str(text);
    assert!(
        !out.contains("hunter22") && !out.contains("abc123"),
        "{out}"
    );
    assert!(out.contains("page=2"), "{out}");
    assert!(check_text(&out, r.rules()).is_empty(), "{out}");
    assert_eq!(r.redact_str(&out), out);
    let plain = "https://localhost/x?page=2&q=a%20b";
    assert_eq!(r.redact_str(plain), plain);
}

#[test]
fn credential_style_keys_are_secret() {
    let mut r = redactor();
    for key in [
        "tokens",
        "cookies",
        "api_key",
        "apikey",
        "api-key",
        "X-Api-Key",
        "signature",
        "sig",
        "passwd",
        "session",
        "credentials",
        "code",
        "client_secret",
        "session_id",
    ] {
        let mut doc = json!({ key: "plain-value-123" });
        r.redact_json(&mut doc);
        assert_eq!(doc[key], "REDACTED", "{key}");
    }
    let mut doc = json!({"channel_name": "general", "ok": true, "encoder": "x"});
    r.redact_json(&mut doc);
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["encoder"], "x");
}

#[test]
fn public_ip_urls_are_foreign_but_local_ones_stay() {
    let mut r = redactor();
    let public = "call http://203.0.113.7/admin?x=1 now";
    assert!(
        check_text(public, r.rules())
            .iter()
            .any(|l| l.kind == LeakKind::ForeignUrl),
        "public IP must be flagged"
    );
    assert!(!r.redact_str(public).contains("203.0.113.7"));
    for ok in [
        "http://127.0.0.1:8080/x",
        "http://192.168.1.5/x",
        "http://[::1]/x",
        "http://10.0.0.2/x",
    ] {
        assert_eq!(r.redact_str(ok), ok);
        assert!(check_text(ok, r.rules()).is_empty(), "{ok}");
    }
}
