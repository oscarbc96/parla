//! Synthetic HAR only. Every "real" value below is made up.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use recon::{extract, har, output, summary};
use redact::{IdMap, Redactor};
use serde_json::{Value, json};
use slack_redact::Rules;

const TOKEN: &str = "xoxc-111111-222222-333333-aabbccddee";
const COOKIE: &str = "xoxd-abc%2Fdef";

fn sample_har() -> Value {
    json!({"log": {"version": "1.2", "entries": [
        {
            "request": {
                "method": "POST",
                "url": format!("https://acme.slack.com/api/conversations.history?_x_id=abc&token={TOKEN}"),
                "headers": [
                    {"name": "Cookie", "value": format!("d={COOKIE}; b=1")},
                    {"name": "Authorization", "value": format!("Bearer {TOKEN}")},
                    {"name": "Content-Type", "value": "application/x-www-form-urlencoded"},
                    {"name": "User-Agent", "value": "Mozilla/5.0 (Macintosh)"},
                    {"name": "Origin", "value": "https://app.slack.com"}
                ],
                "queryString": [{"name": "token", "value": TOKEN}],
                "postData": {
                    "mimeType": "application/x-www-form-urlencoded",
                    "text": "ignored",
                    "params": [
                        {"name": "token", "value": TOKEN},
                        {"name": "channel", "value": "C024BE7LH"},
                        {"name": "text", "value": "private launch plans"},
                        {"name": "blocks", "value": "[{\"type\":\"rich_text\",\"elements\":[{\"type\":\"text\",\"text\":\"hush hush\"},{\"type\":\"user\",\"user_id\":\"U024BE7LH\"}]}]"}
                    ]
                }
            },
            "response": {
                "status": 200,
                "headers": [{"name": "Set-Cookie", "value": format!("d={COOKIE}")}],
                "content": {"mimeType": "application/json", "text": json!({
                    "ok": true,
                    "messages": [{"user": "U024BE7LH", "text": "secret roadmap review", "ts": "1700000000.000100"}],
                    "profile": {"real_name": "Ada Lovelace", "email": "ada@corp.io",
                                "image_72": "https://avatars.slack-edge.com/ada.png",
                                "permalink": "https://acme.slack.com/archives/C024BE7LH/p1700000000000100"}
                }).to_string()}
            }
        },
        {
            "request": {"method": "GET", "url": "wss://wss-primary.slack.com/?token=xoxc-111111-222222-333333-aabbccddee&team=T024BE7LD"},
            "response": {"status": 101},
            "_resourceType": "websocket",
            "_webSocketMessages": [
                {"type": "receive", "time": 1.0, "opcode": 1, "data": "{\"type\":\"user_typing\",\"channel\":\"C024BE7LH\",\"user\":\"U024BE7LH\"}"},
                {"type": "send", "time": 2.0, "opcode": 1, "data": "{\"id\":1,\"type\":\"ping\"}"},
                {"type": "receive", "time": 3.0, "opcode": 1, "data": "not json from U024BE7LH"}
            ]
        }
    ]}})
}

fn read_all(dir: &Path) -> String {
    let mut all = String::new();
    for e in std::fs::read_dir(dir).unwrap() {
        all.push_str(&std::fs::read_to_string(e.unwrap().path()).unwrap());
    }
    all
}

#[test]
fn har_roundtrip_keeps_ws_frames() {
    let parsed = har::parse(&sample_har().to_string()).unwrap();
    let mut r = Redactor::new(IdMap::new(), Arc::new(Rules::default()));
    let built = output::build(&parsed, &mut r);
    assert!(built.leaks.is_empty(), "{:?}", built.leaks);
    let dir = tempfile::tempdir().unwrap();
    output::write(dir.path(), &built).unwrap();

    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "0001-conversations.history.json",
            "index.json",
            "ws-0001-user_typing.json",
            "ws-0002-ping.json",
            "ws-0003-receive.json",
        ]
    );
    let frame: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("ws-0001-user_typing.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(frame["data"]["json"]["user"], "U0000001");
    assert_eq!(frame["direction"], "receive");
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("index.json")).unwrap())
            .unwrap();
    assert_eq!(index.as_array().unwrap().len(), 4);
}

#[test]
fn redact_then_check_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let parsed = har::parse(&sample_har().to_string()).unwrap();
    let mut r = Redactor::new(IdMap::new(), Arc::new(Rules::default()));
    let built = output::build(&parsed, &mut r);
    let out = tmp.path().join("out");
    output::write(&out, &built).unwrap();

    let (files, leaks) =
        summary::check_paths(std::slice::from_ref(&out), &Rules::default()).unwrap();
    assert!(leaks.is_empty() && files == 5, "{leaks:?} {files}");

    // None of the original values survive anywhere in the output.
    let all = read_all(&out);
    for original in [
        TOKEN,
        COOKIE,
        "xox",
        "acme",
        "C024BE7LH",
        "U024BE7LH",
        "T024BE7LD",
        "Ada",
        "Lovelace",
        "corp.io",
        "slack-edge",
        "private launch",
        "hush",
        "roadmap",
        "Mozilla",
    ] {
        assert!(!all.contains(original), "{original} survived");
    }
    assert!(all.contains("wss-primary.slack.com") || all.contains("conversations.history"));

    let text = summary::summarize(&out).unwrap();
    assert!(
        text.contains("conversations.history")
            && text.contains("messages")
            && text.contains("user_typing"),
        "{text}"
    );

    // check flags a planted token and never prints the value.
    let leaky = tmp.path().join("leaky");
    std::fs::create_dir(&leaky).unwrap();
    std::fs::write(leaky.join("a.json"), format!("{{\"t\":\"{TOKEN}\"}}")).unwrap();
    let (_, bad) = summary::check_paths(&[leaky], &Rules::default()).unwrap();
    assert_eq!(bad.len(), 1);
    let printed = format!("{} {:?}", bad[0].1, bad[0].1);
    assert!(
        printed.contains("token") && !printed.contains("xoxc"),
        "{printed}"
    );

    // The map is stable: a second run maps the same real IDs to the same synthetic ones.
    let mut again = Redactor::new(r.into_map(), Arc::new(Rules::default()));
    let built2 = output::build(&parsed, &mut again);
    let first = built
        .files
        .iter()
        .find(|f| f.name.starts_with("ws-0001"))
        .unwrap();
    let second = built2
        .files
        .iter()
        .find(|f| f.name.starts_with("ws-0001"))
        .unwrap();
    assert_eq!(first.text, second.text);
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extract")
}

#[test]
fn extract_fixtures_copy_clean_files() {
    let groups = extract::extract(&fixtures()).unwrap();
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["conversations.history", "message", "user_typing"]);
    let history = &groups[0];
    assert_eq!(history.files.len(), 2);
    let want: std::collections::BTreeSet<String> = ["_x_id", "channel", "cursor", "limit"]
        .map(String::from)
        .into();
    assert_eq!(history.params, want);
    let rules = Rules::default();
    assert!(extract::leaks_in(&groups, &rules).unwrap().is_empty());
    let dest = tempfile::tempdir().unwrap();
    let leaks = extract::copy_fixtures(&groups, dest.path(), &rules).unwrap();
    assert!(leaks.is_empty());
    assert!(dest.path().join("0001-conversations.history.json").exists());
    assert!(dest.path().join("ws-0002-user_typing.json").exists());
    let draft = extract::render("slack", "messages", &groups);
    assert!(
        draft.starts_with("<!-- draft for docs/providers/slack/web-client/messages.md -->"),
        "{draft}"
    );
}

#[test]
fn extract_fixtures_refuse_leak() {
    let src = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(fixtures()).unwrap() {
        let p = e.unwrap().path();
        std::fs::copy(&p, src.path().join(p.file_name().unwrap())).unwrap();
    }
    let planted = src.path().join("0001-conversations.history.json");
    let text = std::fs::read_to_string(&planted)
        .unwrap()
        .replace("\"cursor-2\"", "\"xoxc-1-2\"");
    std::fs::write(&planted, text).unwrap();
    let groups = extract::extract(src.path()).unwrap();
    let rules = Rules::default();
    assert_eq!(extract::leaks_in(&groups, &rules).unwrap().len(), 1);
    let dest = tempfile::tempdir().unwrap();
    let leaks = extract::copy_fixtures(&groups, dest.path(), &rules).unwrap();
    assert_eq!(leaks.len(), 1);
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 0);
}

#[test]
fn markdown_snapshot() {
    let groups = extract::extract(&fixtures()).unwrap();
    let g = groups
        .iter()
        .find(|g| g.name == "conversations.history")
        .unwrap();
    let md = recon::shape::render_markdown(&g.name, g.kind, &g.params, &g.shape);
    let snap =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/conversations.history.md");
    if std::env::var_os("UPDATE_SNAPSHOT").is_some() {
        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        std::fs::write(&snap, &md).unwrap();
    }
    let want =
        std::fs::read_to_string(&snap).expect("snapshot missing; run with UPDATE_SNAPSHOT=1");
    assert_eq!(md, want);
}
