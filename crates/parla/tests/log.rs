//! Logging must never take the app down. When Parla is started from a script
//! or terminal that later goes away, stderr is a pipe nobody reads; a log
//! line written then used to panic ("failed printing to stderr"), which
//! aborted the app and closed its window.

use std::process::Command;

const CHILD: &str = "PARLA_LOG_TEST_CHILD_DIR";

/// Runs only as the child of [`log_survives_a_closed_stderr`].
#[test]
fn child_logs_with_closed_stderr() {
    let Some(dir) = std::env::var_os(CHILD) else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    parla::log::init(&dir);
    parla::log::info("first line");
    parla::log::error("second line");
    std::fs::write(dir.join("done"), "ok").unwrap();
}

#[test]
fn log_survives_a_closed_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let (reader, writer) = std::io::pipe().unwrap();
    // Nobody reads the other end: every write to stderr fails with EPIPE.
    drop(reader);
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_logs_with_closed_stderr", "--nocapture"])
        .env(CHILD, dir.path())
        .stderr(writer)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "logging with a closed stderr failed: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(dir.path().join("done").exists());
    let log = std::fs::read_to_string(parla::log::path_in(dir.path())).unwrap();
    assert!(log.contains("INFO first line"), "{log}");
    assert!(log.contains("ERROR second line"), "{log}");
}
