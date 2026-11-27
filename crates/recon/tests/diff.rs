//! Synthetic neutral captures only (see tests/fixtures/diff).

use std::path::{Path, PathBuf};
use std::process::Command;

use recon::diff::{Capture, DiffItem, diff};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diff")
}

fn load(side: &str) -> Capture {
    Capture::load(&fixtures().join(side)).unwrap()
}

/// Copy a capture dir into a temp dir so a test can edit it.
fn copy(side: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(fixtures().join(side)).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), dir.path().join(e.file_name())).unwrap();
    }
    dir
}

#[test]
fn identical_captures_have_no_diff() {
    assert!(diff(&load("browser"), &load("browser")).is_empty());
}

#[test]
fn missing_param_reported() {
    let items = diff(&load("browser"), &load("parla"));
    assert!(items.contains(&DiffItem::ParamMissing {
        method: "items.list".into(),
        param: "include_pin_count".into()
    }));
    assert!(items.contains(&DiffItem::ParamExtra {
        method: "items.list".into(),
        param: "debug_flag".into()
    }));
}

#[test]
fn missing_header_reported_and_noise_ignored() {
    let items = diff(&load("browser"), &load("parla"));
    assert!(items.contains(&DiffItem::HeaderMissing {
        method: "items.boot".into(),
        header: "origin".into()
    }));
    assert!(!items.iter().any(|i| matches!(i,
        DiffItem::HeaderMissing { header, .. } if header.starts_with("sec-ch-") || header == "accept-language")));
}

#[test]
fn call_order_reported() {
    let dir = copy("parla");
    let p = dir.path();
    std::fs::rename(
        p.join("0001-items.boot.json"),
        p.join("0009-items.boot.json"),
    )
    .unwrap();
    let items = diff(&load("browser"), &Capture::load(p).unwrap());
    assert!(items.contains(&DiffItem::OrderChanged {
        browser: vec![
            "items.boot".into(),
            "items.list".into(),
            "items.post".into()
        ],
        parla: vec![
            "items.list".into(),
            "items.post".into(),
            "items.boot".into()
        ],
    }));
}

#[test]
fn missing_call_and_ws_sent_reported() {
    let dir = copy("parla");
    std::fs::remove_file(dir.path().join("0003-items.post.json")).unwrap();
    let items = diff(&load("browser"), &Capture::load(dir.path()).unwrap());
    assert!(items.contains(&DiffItem::MissingCall {
        method: "items.post".into()
    }));
    assert!(items.contains(&DiffItem::WsSentMissing {
        kind: "ping".into()
    }));
}

#[test]
fn fixtures_are_clean_under_fake_rules() {
    let out = Command::new(env!("CARGO_BIN_EXE_recon-fake"))
        .args(["check"])
        .arg(fixtures())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn values_never_printed() {
    let out = Command::new(env!("CARGO_BIN_EXE_recon-fake"))
        .args(["diff"])
        .arg(fixtures().join("browser"))
        .arg(fixtures().join("parla"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("items.list"));
    for v in [
        "SECRETBROWSERVALUE",
        "SECRETPARLAVALUE",
        "B0000001",
        "app.example.test",
    ] {
        assert!(!text.contains(v), "leaked {v}");
    }
}
