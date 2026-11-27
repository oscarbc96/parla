//! The Slack wiring of the recon CLI: Slack rules are active, no flag selects them.

use std::process::Command;

fn run(dir: &std::path::Path) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_slack_recon"))
        .args(["check", dir.to_str().unwrap()])
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn check_uses_slack_rules_by_default() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ok.json"), r#"{"ok":true}"#).unwrap();
    assert_eq!(run(dir.path()).0, 0);
    std::fs::write(
        dir.path().join("leak.json"),
        r#"{"token":"xoxc-111111111111-222222222222-333333333333-aabbccddeeff00112233"}"#,
    )
    .unwrap();
    let (code, stdout) = run(dir.path());
    assert_eq!(code, 1, "{stdout}");
    assert!(stdout.contains("leak.json"), "{stdout}");
}
