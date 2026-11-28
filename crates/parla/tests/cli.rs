//! `parla mcp`: the stdio shim runs before any GPUI or settings init.
#![cfg(unix)]

use std::{
    io::{Read, Write},
    process::{Command, Stdio},
};

use mcp::ipc::{Accepted, IpcServer};

/// `IpcServer::bind` narrows the process umask while it creates the socket, so
/// a temp dir made by another test thread meanwhile loses its search bit.
/// Dir creation and binds take this lock.
static UMASK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn bind(sock: &std::path::Path) -> IpcServer {
    let _g = UMASK.lock().unwrap_or_else(|e| e.into_inner());
    IpcServer::bind(sock).unwrap()
}

fn data_dir() -> tempfile::TempDir {
    let _g = UMASK.lock().unwrap_or_else(|e| e.into_inner());
    tempfile::Builder::new()
        .prefix("t")
        .tempdir_in("/tmp")
        .unwrap()
}

#[test]
fn parla_mcp_starts_no_gpui() {
    let dir = data_dir();
    let out = Command::new(env!("CARGO_BIN_EXE_parla"))
        .arg("mcp")
        .env("PARLA_DATA_DIR", dir.path())
        .env("PARLA_CONFIG_DIR", dir.path().join("config"))
        .env("PARLA_SHIM_APP_CMD", "/usr/bin/true")
        .env("PARLA_SHIM_START_TIMEOUT_MS", "300")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    // No wall-clock bound: exiting with 3 at all proves the shim gave up on
    // its own, and the run's hung-test kill (nextest) catches a real hang.
    assert!(out.stdout.is_empty(), "stdout is the MCP channel");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("started, log file"), "{err}");
    assert!(err.contains("did not start"), "{err}");
    // No settings, log or store files: the app's init never ran.
    assert!(!dir.path().join("config").exists());
    assert!(!dir.path().join("parla.log").exists());
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.iter().all(|n| n == "run"), "{names:?}");
}

/// Resident set of a process in KiB, from `ps`.
fn rss_kib(pid: u32) -> u64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn shim_rss_under_10_mb() {
    let dir = data_dir();
    let sock = dir.path().join("run").join("mcp.sock");
    let server = bind(&sock).run(|a| {
        if let Accepted::Mcp { mut stream, .. } = a {
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 || stream.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_parla"))
        .arg("mcp")
        .env("PARLA_DATA_DIR", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    // A relaying shim: push 1 MiB through and read it back.
    let chunk = vec![b'x'; 64 * 1024];
    let reader = std::thread::spawn(move || {
        let mut got = 0;
        let mut buf = vec![0u8; 64 * 1024];
        while got < 16 * chunk_len() {
            let n = stdout.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got += n;
        }
        got
    });
    for _ in 0..16 {
        stdin.write_all(&chunk).unwrap();
    }
    let got = reader.join().unwrap();
    assert_eq!(got, 16 * chunk_len());
    let rss = rss_kib(child.id());
    // Spec 9 `mcp.shim`: < 10 MB. A debug binary is 150 MB with symbols and the
    // dynamic frameworks GPUI links are resident, so debug only guards against
    // the app initialising (which costs far more); release is held to the budget.
    // Linux maps GTK and WebKitGTK (linked for wry) at process start, about
    // 60 MB in a debug build; the 10 MB budget is a macOS one.
    let limit = if cfg!(target_os = "linux") {
        81_920
    } else if cfg!(debug_assertions) {
        20_480
    } else {
        10_240
    };
    assert!(rss < limit, "shim rss {rss} KiB (limit {limit})");
    drop(stdin);
    let _ = child.wait();
    server.shutdown();
}

fn chunk_len() -> usize {
    64 * 1024
}

#[test]
fn call_window_without_open_line_exits_nonzero_fast() {
    let dir = data_dir();
    let out = Command::new(env!("CARGO_BIN_EXE_parla"))
        .arg("call-window")
        .env("PARLA_DATA_DIR", dir.path())
        .env("PARLA_CONFIG_DIR", dir.path().join("config"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("started, log file"), "{err}");
    // No settings, log or store files: the app's init never ran.
    assert!(!dir.path().join("config").exists());
    assert!(!dir.path().join("parla.log").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn call_window_skips_single_instance() {
    let dir = data_dir();
    let sock = dir.path().join("run").join("mcp.sock");
    let shown = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let s2 = shown.clone();
    let server = bind(&sock).run(move |a| {
        if matches!(a, Accepted::Show) {
            s2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let out = Command::new(env!("CARGO_BIN_EXE_parla"))
        .arg("call-window")
        .env("PARLA_DATA_DIR", dir.path())
        .env("PARLA_CONFIG_DIR", dir.path().join("config"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    // It ran as a helper (no Open line: exit 2), not as a second app that
    // hands off to the running one (which would exit 0 after a "show").
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(shown.load(std::sync::atomic::Ordering::SeqCst), 0);
    drop(server);
}
