//! Synthetic fixtures and a wiremock stand-in for the provider. No network.

use std::process::{Command, Stdio};

use recon::shape::infer;
use recon::verify::compare;
use redact::IdMap;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "fake-111111-222222-333333-aabbccddee";
const COOKIE: &str = "d=fake-abc%2Fdef";
const REAL_CHANNEL: &str = "B024BE7LH";

fn fixture_doc() -> Value {
    json!({
        "method": "POST",
        "request": {"body": {"form": [
            ["token", "REDACTED"], ["channel", "B0000001"], ["limit", "28"]
        ]}},
        "response": {"status": 200, "body": {"json": {
            "ok": true, "has_more": false,
            "items": [{"ts": "1.5", "text": "hi"}]
        }}}
    })
}

struct Env {
    dir: tempfile::TempDir,
    server: MockServer,
}

async fn env(catalogue_method: &str, read_only: &str, live: Value) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let fixtures = dir.path().join("fixtures");
    std::fs::create_dir_all(&fixtures).unwrap();
    std::fs::write(
        fixtures.join(format!("{catalogue_method}.json")),
        fixture_doc().to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("catalogue.md"),
        format!(
            "# Catalogue\n\n| Feature | Method or event | Kind (http/ws) | Read-only | Fixture | First seen | Last verified |\n|---|---|---|---|---|---|---|\n| Items | {catalogue_method} | http | {read_only} | `fixtures/<method>.json` | 2026-11-13 | 2026-11-13 |\n| Socket | (connect URL) | ws | yes | none | 2026-11-13 | 2026-11-13 |\n"
        ),
    )
    .unwrap();
    let mut map = IdMap::new();
    assert_eq!(map.id(REAL_CHANNEL), "B0000001");
    map.save(&dir.path().join("idmap.json")).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/api/{catalogue_method}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(live))
        .mount(&server)
        .await;
    Env { dir, server }
}

fn run(env: &Env, stdin: &str, extra_env: &[(&str, &str)]) -> (i32, String) {
    use std::io::Write;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_recon-fake"));
    cmd.args(["verify", "--catalogue"])
        .arg(env.dir.path().join("catalogue.md"))
        .arg("--root")
        .arg(env.dir.path())
        .arg("--map")
        .arg(env.dir.path().join("idmap.json"))
        .args(["--base-url", &env.server.uri(), "--rate", "6000"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap(), text)
}

fn creds() -> String {
    format!("{TOKEN}\n{COOKIE}\n")
}

fn good_live() -> Value {
    json!({"ok": true, "has_more": true, "items": [{"ts": "9.1", "text": "yo"}]})
}

#[tokio::test(flavor = "multi_thread")]
async fn verify_refuses_write_methods() {
    let e = env("chat.postMessage", "yes", good_live()).await;
    let (code, out) = run(&e, &creds(), &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("refused chat.postMessage"), "{out}");
    assert!(e.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn catalogue_no_blocks_even_allowlisted_method() {
    let e = env("items.list", "no", good_live()).await;
    let (code, out) = run(&e, &creds(), &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("refused items.list"), "{out}");
    assert!(e.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn matching_shape_passes() {
    let e = env("items.list", "yes", good_live()).await;
    let (code, out) = run(&e, &creds(), &[]);
    assert_eq!(code, 0, "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn added_field_is_note() {
    let mut live = good_live();
    live["foo"] = json!(1);
    let e = env("items.list", "yes", live).await;
    let (code, out) = run(&e, &creds(), &[]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("added /foo"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_field_fails() {
    let e = env("items.list", "yes", json!({"ok": true, "items": []})).await;
    let (code, out) = run(&e, &creds(), &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("removed /has_more"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn retyped_field_fails() {
    let live = json!({"ok": true, "has_more": false, "items": [{"ts": 1.5, "text": "hi"}]});
    let e = env("items.list", "yes", live).await;
    let (code, out) = run(&e, &creds(), &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("retyped /items/[]/ts"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ids_mapped_back_for_request() {
    let e = env("items.list", "yes", good_live()).await;
    let (code, out) = run(&e, &creds(), &[]);
    assert_eq!(code, 0, "{out}");
    let reqs = e.server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    let body = String::from_utf8_lossy(&reqs[0].body).to_string();
    assert!(body.contains(&format!("channel={REAL_CHANNEL}")), "{body}");
    assert!(!body.contains("B0000001"), "{body}");
    assert!(body.contains(&format!("token={TOKEN}")), "{body}");
    assert_eq!(
        reqs[0].headers.get("cookie").unwrap().to_str().unwrap(),
        COOKIE
    );
    // Names and counts only: no secret or real id on stdout.
    assert!(!out.contains(TOKEN) && !out.contains(REAL_CHANNEL), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_only_from_stdin() {
    let e = env("items.list", "yes", good_live()).await;
    let (code, out) = run(
        &e,
        "",
        &[("PARLA_TOKEN", TOKEN), ("TOKEN", TOKEN), ("COOKIE", COOKIE)],
    );
    assert_eq!(code, 2, "{out}");
    assert!(e.server.received_requests().await.unwrap().is_empty());
}

#[test]
fn compare_reports_pointers() {
    let a = infer(&[json!({"a": 1, "b": {"c": "x"}})]);
    let b = infer(&[json!({"b": {"c": 2, "d": true}, "e": null})]);
    let d = compare(&a, &b);
    assert_eq!(d.removed, vec!["/a".to_string()]);
    assert_eq!(d.added, vec!["/b/d".to_string(), "/e".to_string()]);
    assert_eq!(
        d.retyped,
        vec![(
            "/b/c".to_string(),
            "string".to_string(),
            "number".to_string()
        )]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn base_url_must_be_https_unless_local() {
    use std::io::Write;
    let e = env("items.list", "yes", good_live()).await;
    let mut child = Command::new(env!("CARGO_BIN_EXE_recon-fake"))
        .args(["verify", "--catalogue"])
        .arg(e.dir.path().join("catalogue.md"))
        .arg("--root")
        .arg(e.dir.path())
        .arg("--map")
        .arg(e.dir.path().join("idmap.json"))
        .args(["--base-url", "http://203.0.113.9", "--rate", "6000"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(creds().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("must be https"), "{text}");
}
