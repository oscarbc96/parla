//! The `PARLA_RECORD` recorder: an owner-only directory and an index that only
//! names files that were actually written.

use std::sync::Arc;

use redact::FakeRules;
use redact::record::{Direction, Recorder};
use serde_json::Value;

#[test]
fn recorder_dir_is_owner_only() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("record");
    if !redact::record::enabled() {
        return;
    }
    assert!(Recorder::new(&dir, Arc::new(FakeRules::default())).is_some());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{dir:?}");
    }
}

#[test]
fn index_lists_only_files_that_exist() {
    if !redact::record::enabled() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::new(dir.path(), Arc::new(FakeRules::default())).unwrap();
    rec.record_http(
        "POST",
        "https://app.chat.example.test/rpc/items.list",
        &[("content-type".into(), "application/json".into())],
        &[],
        200,
        r#"{"ok":true}"#,
    );
    rec.record_ws(Direction::Receive, r#"{"type":"item_added"}"#);

    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("index.json")).unwrap())
            .unwrap();
    let listed: Vec<&str> = index
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["file"].as_str().unwrap())
        .collect();
    assert_eq!(listed.len(), 2, "{listed:?}");
    for name in &listed {
        assert!(
            dir.path().join(name).exists(),
            "{name} is listed but missing"
        );
    }
    let on_disk = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json") && n != "index.json")
        .count();
    assert_eq!(listed.len(), on_disk);
}
