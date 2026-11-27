//! Repo tooling for the UI test layers: the mise tasks.

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn mise_has_ui_tasks() {
    let doc: toml::Table = read("mise.toml").parse().expect("mise.toml parses");
    let tasks = doc["tasks"].as_table().expect("tasks table");
    for name in [
        "test:visual",
        "test:visual:update",
        "test:visual:shots",
        "test:e2e",
    ] {
        assert!(tasks.contains_key(name), "missing mise task {name}");
    }
    let run = |n: &str| tasks[n]["run"].as_str().unwrap_or_default().to_string();
    assert!(run("test:visual").contains("--features visual"));
    assert!(
        !run("test").contains("--features"),
        "`test` must stay feature-free"
    );
    let env = |n: &str, k: &str| tasks[n]["env"][k].as_str().map(str::to_string);
    assert_eq!(
        env("test:visual:update", "PARLA_VISUAL_UPDATE").as_deref(),
        Some("1")
    );
    assert_eq!(
        env("test:visual:shots", "PARLA_VISUAL_SHOTS").as_deref(),
        Some("1")
    );
}
