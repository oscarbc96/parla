//! Synthetic HAR only. Every "real" value below is made up.

use std::path::Path;
use std::process::Command;

use recon::{har, output};
use std::sync::Arc;

use redact::{FakeRules, IdMap, Redactor};
use serde_json::{Value, json};

const TOKEN: &str = "fake-111111-222222-333333-aabbccddee";
const COOKIE: &str = "fake-abc%2Fdef";

fn sample_har() -> Value {
    json!({"log": {"version": "1.2", "entries": [
        {
            "request": {
                "method": "POST",
                "url": format!("https://acme.chat.example.test/rpc/items.list?_x_id=abc&token={TOKEN}"),
                "headers": [
                    {"name": "Cookie", "value": format!("d={COOKIE}; b=1")},
                    {"name": "Authorization", "value": format!("Bearer {TOKEN}")},
                    {"name": "Content-Type", "value": "application/x-www-form-urlencoded"},
                    {"name": "User-Agent", "value": "Mozilla/5.0 (Macintosh)"},
                    {"name": "Origin", "value": "https://app.chat.example.test"},
                    {"name": "X-Chat-Request-Id", "value": "A024BE7LH"}
                ],
                "queryString": [{"name": "token", "value": TOKEN}],
                "postData": {
                    "mimeType": "application/x-www-form-urlencoded",
                    "text": "ignored",
                    "params": [
                        {"name": "token", "value": TOKEN},
                        {"name": "channel", "value": "B024BE7LH"},
                        {"name": "text", "value": "private launch plans"},
                        {"name": "blocks", "value": "[{\"type\":\"rich_text\",\"elements\":[{\"type\":\"text\",\"text\":\"hush hush\"},{\"type\":\"user\",\"user_id\":\"A024BE7LH\"}]}]"}
                    ]
                }
            },
            "response": {
                "status": 200,
                "headers": [{"name": "Set-Cookie", "value": format!("d={COOKIE}")}],
                "content": {"mimeType": "application/json", "text": json!({
                    "ok": true,
                    "items": [{"user": "A024BE7LH", "text": "secret roadmap review", "ts": "1700000000.000100"}],
                    "profile": {"real_name": "Ada Lovelace", "email": "ada@corp.io",
                                "image_72": "https://cdn.example.test/ada.png",
                                "permalink": "https://acme.chat.example.test/archives/B024BE7LH/p1700000000000100"}
                }).to_string()}
            }
        },
        {
            "request": {"method": "GET", "url": "wss://ws.chat.example.test/?token=fake-111111-222222-333333-aabbccddee&team=C024BE7LD"},
            "response": {"status": 101},
            "_resourceType": "websocket",
            "_webSocketMessages": [
                {"type": "receive", "time": 1.0, "opcode": 1, "data": "{\"type\":\"item_removed\",\"channel\":\"B024BE7LH\",\"user\":\"A024BE7LH\"}"},
                {"type": "send", "time": 2.0, "opcode": 1, "data": "{\"id\":1,\"type\":\"ping\"}"},
                {"type": "receive", "time": 3.0, "opcode": 1, "data": "not json from A024BE7LH"}
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
    let mut r = Redactor::new(IdMap::new(), Arc::new(FakeRules::default()));
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
            "0001-items.list.json",
            "index.json",
            "ws-0001-item_removed.json",
            "ws-0002-ping.json",
            "ws-0003-receive.json",
        ]
    );
    let frame: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("ws-0001-item_removed.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(frame["data"]["json"]["user"], "A0000001");
    assert_eq!(frame["direction"], "receive");
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("index.json")).unwrap())
            .unwrap();
    assert_eq!(index.as_array().unwrap().len(), 4);
}

#[test]
fn cli_redact_then_check_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let har_path = tmp.path().join("fixture.har");
    std::fs::write(&har_path, sample_har().to_string()).unwrap();
    let out = tmp.path().join("out");
    let map = tmp.path().join("cache").join("idmap.json");
    let bin = env!("CARGO_BIN_EXE_recon-fake");

    let redact = Command::new(bin)
        .args([
            "redact",
            har_path.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--map",
            map.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        redact.status.success(),
        "{}",
        String::from_utf8_lossy(&redact.stderr)
    );
    let stdout = String::from_utf8_lossy(&redact.stdout);
    assert!(
        stdout.contains("1 exchanges") && stdout.contains("3 websocket frames"),
        "{stdout}"
    );
    assert!(map.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "the capture dir is owner-only");
    }

    let check = Command::new(bin)
        .args(["check", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("clean"));

    // None of the original values survive anywhere in the output.
    let all = read_all(&out);
    for original in [
        TOKEN,
        COOKIE,
        "fake-",
        "acme",
        "B024BE7LH",
        "A024BE7LH",
        "C024BE7LD",
        "Ada",
        "Lovelace",
        "corp.io",
        "cdn.example",
        "private launch",
        "hush",
        "roadmap",
        "Mozilla",
    ] {
        assert!(!all.contains(original), "{original} survived");
    }
    assert!(all.contains("ws.chat.example.test") || all.contains("items.list"));

    let summary = Command::new(bin)
        .args(["summary", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(summary.status.success());
    let text = String::from_utf8_lossy(&summary.stdout);
    assert!(
        text.contains("items.list") && text.contains("items") && text.contains("item_removed"),
        "{text}"
    );

    // check exits 1 on a planted leak and never prints the value.
    let leaky = tmp.path().join("leaky");
    std::fs::create_dir(&leaky).unwrap();
    std::fs::write(leaky.join("a.json"), format!("{{\"t\":\"{TOKEN}\"}}")).unwrap();
    let bad = Command::new(bin)
        .args(["check", leaky.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(1));
    let printed = String::from_utf8_lossy(&bad.stdout);
    assert!(
        printed.contains("token") && !printed.contains("fake-"),
        "{printed}"
    );

    // The map is stable: a second run maps the same real IDs to the same synthetic ones.
    let out2 = tmp.path().join("out2");
    let again = Command::new(bin)
        .args([
            "redact",
            har_path.to_str().unwrap(),
            "--out",
            out2.to_str().unwrap(),
            "--map",
            map.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(again.status.success());
    assert_eq!(
        std::fs::read_to_string(out.join("ws-0001-item_removed.json")).unwrap(),
        std::fs::read_to_string(out2.join("ws-0001-item_removed.json")).unwrap()
    );
}

#[test]
fn cli_check_rejects_unscrubbed_link_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let raw = json!({"response": {"body": {"json": {"messages": [{"attachments": [{
        "title": "Quarterly tracker rollup for the platform team",
        "title_link": "https://tracker.invalid.io/issues/42"
    }]}]}}}});
    let file = tmp.path().join("0001-x.json");
    std::fs::write(&file, serde_json::to_string_pretty(&raw).unwrap()).unwrap();
    let bin = env!("CARGO_BIN_EXE_recon-fake");
    let out = Command::new(bin)
        .args(["check", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("foreign-url") && stdout.contains("free-text"),
        "{stdout}"
    );
    assert!(!stdout.contains("tracker"), "{stdout}");
}

#[test]
fn cli_redact_scrubs_link_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let har = json!({"log": {"version": "1.2", "entries": [{
        "request": {"method": "GET", "url": "https://acme.chat.example.test/rpc/items.list", "headers": []},
        "response": {"status": 200, "headers": [], "content": {"mimeType": "application/json", "text": json!({
            "ok": true,
            "messages": [{"attachments": [{
                "title": "Quarterly tracker rollup for the platform team",
                "title_link": "https://tracker.invalid.io/issues/42",
                "footer": "Tracker board for platform work",
                "service_name": "Tracker Service"
            }]}]
        }).to_string()}}
    }]}});
    let har_path = tmp.path().join("f.har");
    std::fs::write(&har_path, har.to_string()).unwrap();
    let out = tmp.path().join("out");
    let bin = env!("CARGO_BIN_EXE_recon-fake");
    let red = Command::new(bin)
        .args([
            "redact",
            har_path.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--map",
            tmp.path().join("m").join("idmap.json").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        red.status.success(),
        "{}",
        String::from_utf8_lossy(&red.stderr)
    );
    let all = read_all(&out);
    assert!(
        !all.contains("tracker") && !all.contains("Tracker") && !all.contains("platform"),
        "{all}"
    );
    let chk = Command::new(bin)
        .args(["check", out.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        chk.status.success(),
        "{}",
        String::from_utf8_lossy(&chk.stdout)
    );
}
