use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/actions.json")
}

fn print_actions_output() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_parla"))
        .arg("--print-actions")
        .output()
        .expect("run parla --print-actions");
    assert!(
        out.status.success(),
        "parla --print-actions failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8")
}

#[test]
fn print_actions_matches_snapshot() {
    let expected = fs::read_to_string(snapshot_path()).expect("read snapshot");
    assert_eq!(print_actions_output(), expected);
}

#[test]
fn every_action_in_snapshot_is_documented_on_keymap_page() {
    let snapshot = fs::read_to_string(snapshot_path()).expect("read snapshot");
    let entries: Vec<serde_json::Value> = serde_json::from_str(&snapshot).expect("parse snapshot");
    let root = repo_root();
    for locale in ["keymap.mdx", "keymap.es.mdx", "keymap.ca.mdx"] {
        let page = fs::read_to_string(root.join("site/content/docs").join(locale))
            .unwrap_or_else(|e| panic!("read {locale}: {e}"));
        for entry in &entries {
            let action = entry["action"].as_str().expect("action");
            assert!(page.contains(action), "{locale} is missing action {action}");
        }
    }
}
