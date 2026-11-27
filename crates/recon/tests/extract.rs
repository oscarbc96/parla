//! Synthetic fixtures only (tests/fixtures/extract).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use recon::extract::{self, Group};
use recon::shape::{ExchangeKind, Field, Shape, infer, render_markdown};
use serde_json::json;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extract")
}

fn obj(shape: &Shape) -> &std::collections::BTreeMap<String, Field> {
    match shape {
        Shape::Object(m) => m,
        other => panic!("not an object: {other:?}"),
    }
}

fn group<'a>(groups: &'a [Group], name: &str) -> &'a Group {
    groups
        .iter()
        .find(|g| g.name == name)
        .unwrap_or_else(|| panic!("no group {name}"))
}

#[test]
fn infer_marks_missing_keys_optional() {
    let shape = infer(&[json!({"a": 1, "b": "x"}), json!({"a": 2})]);
    let m = obj(&shape);
    assert_eq!(m["a"].shape, Shape::Number);
    assert!(!m["a"].optional);
    assert!(matches!(m["b"].shape, Shape::String { .. }));
    assert!(m["b"].optional);
}

#[test]
fn infer_collects_type_enums_only() {
    let shape = infer(&[
        json!({"type": "item_added", "text": "one"}),
        json!({"type": "item_removed", "text": "two"}),
    ]);
    let m = obj(&shape);
    let Shape::String { enums } = &m["type"].shape else {
        panic!("type is not a string")
    };
    let want: BTreeSet<String> = ["item_added", "item_removed"].map(String::from).into();
    assert_eq!(enums, &want);
    let Shape::String { enums } = &m["text"].shape else {
        panic!("text is not a string")
    };
    assert!(enums.is_empty());
}

#[test]
fn infer_caps_enums_at_eight() {
    let samples: Vec<_> = (0..9).map(|i| json!({"type": format!("t{i}")})).collect();
    let shape = infer(&samples);
    let Shape::String { enums } = &obj(&shape)["type"].shape else {
        panic!()
    };
    assert!(enums.is_empty());
}

#[test]
fn infer_merges_array_items() {
    let shape = infer(&[json!({"messages": [
        {"ts": "1", "thread_ts": "1"},
        {"ts": "2"}
    ]})]);
    let Shape::Array(item) = &obj(&shape)["messages"].shape else {
        panic!("messages is not an array")
    };
    let m = obj(item);
    assert!(!m["ts"].optional);
    assert!(m["thread_ts"].optional);
}

#[test]
fn infer_mixed_on_type_conflict() {
    let shape = infer(&[json!({"a": 1}), json!({"a": "x"})]);
    assert!(matches!(obj(&shape)["a"].shape, Shape::Mixed(_)));
}

#[test]
fn extract_groups_by_method_and_event() {
    let groups = extract::extract(&fixtures()).unwrap();
    let names: Vec<(&str, ExchangeKind)> =
        groups.iter().map(|g| (g.name.as_str(), g.kind)).collect();
    assert_eq!(
        names,
        [
            ("items.list", ExchangeKind::Http),
            ("item_added", ExchangeKind::Ws),
            ("item_removed", ExchangeKind::Ws),
        ]
    );
    let history = group(&groups, "items.list");
    assert_eq!(history.files.len(), 2);
    let m = obj(&history.shape);
    assert!(m["response_meta"].optional, "missing from one sample");
    assert!(!m["more"].optional);
}

#[test]
fn extract_lists_request_params() {
    let groups = extract::extract(&fixtures()).unwrap();
    let want: BTreeSet<String> = ["_x_id", "folder", "cursor", "limit"]
        .map(String::from)
        .into();
    assert_eq!(group(&groups, "items.list").params, want);
    assert!(group(&groups, "item_added").params.is_empty());
}

#[test]
fn extract_fixtures_copies_clean_files() {
    let dest = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_recon-fake"))
        .args(["extract", fixtures().to_str().unwrap(), "--area", "items"])
        .args(["--fixtures", dest.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("### items.list"), "{stdout}");
    assert!(
        stdout.contains("<!-- draft for docs/providers/fake/web-client/items.md -->"),
        "{stdout}"
    );
    assert!(dest.path().join("0001-items.list.json").exists());
    assert!(dest.path().join("ws-0002-item_removed.json").exists());
}

#[test]
fn extract_fixtures_refuses_leak() {
    let src = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(fixtures()).unwrap() {
        let p = e.unwrap().path();
        std::fs::copy(&p, src.path().join(p.file_name().unwrap())).unwrap();
    }
    let planted = src.path().join("0001-items.list.json");
    let text = std::fs::read_to_string(&planted)
        .unwrap()
        .replace("\"cursor-2\"", "\"fake-1-2\"");
    std::fs::write(&planted, text).unwrap();
    let dest = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_recon-fake"))
        .args(["extract", src.path().to_str().unwrap(), "--area", "items"])
        .args(["--fixtures", dest.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains("fake-1-2"), "{all}");
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 0);
}

#[test]
fn copy_fixtures_keeps_existing_files_on_a_leak() {
    let src = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(fixtures()).unwrap() {
        let p = e.unwrap().path();
        std::fs::copy(&p, src.path().join(p.file_name().unwrap())).unwrap();
    }
    let planted = src.path().join("0001-items.list.json");
    let text = std::fs::read_to_string(&planted)
        .unwrap()
        .replace("\"cursor-2\"", "\"fake-1-2\"");
    std::fs::write(&planted, text).unwrap();
    let groups = extract::extract(src.path()).unwrap();
    // `dest` holds committed fixtures already; a refused run must not touch them.
    let dest = tempfile::tempdir().unwrap();
    let kept = dest.path().join("0001-items.list.json");
    std::fs::write(&kept, b"committed fixture").unwrap();
    let leaks =
        extract::copy_fixtures(&groups, dest.path(), &redact::FakeRules::default()).unwrap();
    assert_eq!(leaks.len(), 1);
    assert_eq!(std::fs::read(&kept).unwrap(), b"committed fixture");
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 1);
}

#[test]
fn markdown_snapshot() {
    let groups = extract::extract(&fixtures()).unwrap();
    let g = group(&groups, "items.list");
    let md = render_markdown(&g.name, g.kind, &g.params, &g.shape);
    let snap = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/items.list.md");
    if std::env::var_os("UPDATE_SNAPSHOT").is_some() {
        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        std::fs::write(&snap, &md).unwrap();
    }
    let want =
        std::fs::read_to_string(&snap).expect("snapshot missing; run with UPDATE_SNAPSHOT=1");
    assert_eq!(md, want);
}
