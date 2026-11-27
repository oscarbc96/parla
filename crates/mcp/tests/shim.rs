//! the `parla mcp` stdio shim.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use mcp::{
    ipc::{Accepted, IpcServer},
    shim::relay,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

/// A deterministic non-UTF-8 byte stream with partial lines.
fn noise(n: usize, seed: u8) -> Vec<u8> {
    let mut x = u32::from(seed) + 1;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect()
}

#[tokio::test]
async fn relay_is_byte_exact() {
    let (client, client_far) = tokio::io::duplex(64 * 1024);
    let (app, app_far) = tokio::io::duplex(64 * 1024);
    let relay = tokio::spawn(relay(client, app));
    let up = noise(1 << 20, 1);
    let down = noise(1 << 20, 2);
    // Each end writes and reads at the same time; sequential would deadlock
    // on the bounded pipes.
    async fn exchange(
        io: tokio::io::DuplexStream,
        send: Vec<u8>,
        expect: Vec<u8>,
    ) -> tokio::io::DuplexStream {
        let (mut r, mut w) = tokio::io::split(io);
        let writer = tokio::spawn(async move {
            w.write_all(&send).await.unwrap();
            w
        });
        let mut got = vec![0u8; expect.len()];
        r.read_exact(&mut got).await.unwrap();
        assert!(got == expect, "bytes differ");
        r.unsplit(writer.await.unwrap())
    }
    let a = tokio::spawn(exchange(client_far, up.clone(), down.clone()));
    let b = tokio::spawn(exchange(app_far, down, up));
    drop((a.await.unwrap(), b.await.unwrap()));
    relay.await.unwrap().ok();
}

#[tokio::test]
async fn relay_ends_when_either_side_closes() {
    for close_client in [true, false] {
        let (client, client_far) = tokio::io::duplex(1024);
        let (app, app_far) = tokio::io::duplex(1024);
        let task = tokio::spawn(relay(client, app));
        if close_client {
            drop(client_far);
            let _keep = app_far;
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .expect("relay ends when the client closes")
                .unwrap()
                .ok();
        } else {
            drop(app_far);
            let _keep = client_far;
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .expect("relay ends when the app closes")
                .unwrap()
                .ok();
        }
    }
}

/// The shim binary, built by cargo before this test binary runs.
fn shim_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_parla_shim"))
}

/// `PARLA_DATA_DIR` for a short temp dir; the socket is `<dir>/run/mcp.sock`.
fn data_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("t")
        .tempdir_in("/tmp")
        .unwrap()
}

fn sock(dir: &Path) -> PathBuf {
    dir.join("run").join("mcp.sock")
}

fn shim(dir: &Path) -> Command {
    let mut c = Command::new(shim_bin());
    c.env("PARLA_DATA_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    c
}

#[tokio::test]
async fn shim_reports_mcp_off_with_exit_2() {
    let dir = data_dir();
    let server = IpcServer::bind(&sock(dir.path()))
        .unwrap()
        .with_mcp_gate(Arc::new(|| false))
        .run(|_| {});
    let out = shim(dir.path())
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "stdout is the MCP channel");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Parla's MCP server is off"), "{err}");
    assert!(err.contains("mcp.enabled"), "{err}");
    server.shutdown();
}

#[tokio::test]
async fn shim_reports_busy_with_exit_4() {
    let dir = data_dir();
    let server = IpcServer::bind(&sock(dir.path())).unwrap().run(|a| {
        // Hold every MCP stream so the cap fills up.
        if let Accepted::Mcp { stream, .. } = a {
            std::mem::forget(stream);
        }
    });
    let mut held = Vec::new();
    for _ in 0..mcp::ipc::MAX_CONNECTIONS {
        held.push(mcp::ipc::connect(&sock(dir.path()), "mcp").await.unwrap());
    }
    let out = shim(dir.path())
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
    assert!(out.stdout.is_empty());
    server.shutdown();
}

#[cfg(unix)]
#[tokio::test]
async fn shim_spawns_headless_when_no_app() {
    use std::os::unix::fs::PermissionsExt;
    let dir = data_dir();
    // The fake app writes its arguments into a FIFO, so the test blocks until
    // the detached child has really run instead of guessing how long it takes.
    let rec = dir.path().join("args.txt");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&rec)
            .status()
            .unwrap()
            .success()
    );
    let fake = dir.path().join("fake-app.sh");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n", rec.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = shim(dir.path())
        .env("PARLA_SHIM_APP_CMD", &fake)
        .env("PARLA_SHIM_START_TIMEOUT_MS", "800")
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(3), "the fake app never listens");
    let args = tokio::task::spawn_blocking(move || std::fs::read_to_string(rec))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(args, "--headless\n");
}

#[cfg(unix)]
#[tokio::test]
async fn shim_gives_up_after_the_start_timeout_with_exit_3() {
    let dir = data_dir();
    let out = shim(dir.path())
        .env("PARLA_SHIM_APP_CMD", "/usr/bin/true")
        .env("PARLA_SHIM_START_TIMEOUT_MS", "500")
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn shim_starts_the_app_then_relays_once_it_listens() {
    use std::{
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicBool, Ordering},
    };
    let dir = data_dir();
    // The fake app announces itself through a FIFO; the test's listener only
    // appears after that, so the shim has to start the app and then retry.
    let fifo = dir.path().join("started");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let fake = dir.path().join("fake-app.sh");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\necho started > '{}'\n", fifo.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = shim(dir.path())
        .env("PARLA_SHIM_APP_CMD", &fake)
        // The shim waits for the listener as long as the fake app takes to
        // run (a fresh script can take seconds to exec on a loaded machine);
        // the FIFO below orders the events, not a clock.
        .env("PARLA_SHIM_START_TIMEOUT_MS", "600000")
        .spawn()
        .unwrap();
    // Blocks until the shim has launched the app; no guessed delay.
    tokio::task::spawn_blocking(move || std::fs::read_to_string(fifo))
        .await
        .unwrap()
        .unwrap();
    let echoed = Arc::new(AtomicBool::new(false));
    let e = echoed.clone();
    let server = IpcServer::bind(&sock(dir.path())).unwrap().run(move |a| {
        if let Accepted::Mcp { mut stream, .. } = a {
            let e = e.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 5];
                stream.read_exact(&mut buf).await.unwrap();
                stream.write_all(&buf).await.unwrap();
                e.store(true, Ordering::SeqCst);
            });
        }
    });
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    if let Err(e) = stdin.write_all(b"he\xffll").await {
        let out = child.wait_with_output().await.unwrap();
        panic!(
            "write failed: {e}; shim {:?} stderr {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let mut got = [0u8; 5];
    tokio::time::timeout(Duration::from_secs(30), stdout.read_exact(&mut got))
        .await
        .expect("echo through the shim")
        .unwrap();
    assert_eq!(&got, b"he\xffll");
    assert!(echoed.load(Ordering::SeqCst));
    drop(stdin);
    server.shutdown();
}

#[cfg(unix)]
#[tokio::test]
async fn shim_stdout_silent_before_handshake() {
    let dir = data_dir();
    let std_listener = {
        std::fs::create_dir(dir.path().join("run")).unwrap();
        std::os::unix::net::UnixListener::bind(sock(dir.path())).unwrap()
    };
    let mut child = shim(dir.path()).spawn().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (mut conn, _) = tokio::task::spawn_blocking(move || std_listener.accept().unwrap())
        .await
        .unwrap();
    // The shim sent its hello and waits; the handshake is unanswered.
    let mut hello = [0u8; 64];
    let n = std::io::Read::read(&mut conn, &mut hello).unwrap();
    assert!(hello[..n].starts_with(b"{\"parla_ipc\":1"));
    let mut b = [0u8; 1];
    let waited = tokio::time::timeout(Duration::from_millis(400), stdout.read(&mut b)).await;
    assert!(waited.is_err(), "nothing on stdout before the handshake");
    std::io::Write::write_all(&mut conn, b"{\"ok\":false,\"error\":\"mcp_off\"}\n").unwrap();
    let mut rest = Vec::new();
    stdout.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());
    assert_eq!(child.wait().await.unwrap().code(), Some(2));
}

#[cfg(unix)]
#[tokio::test]
async fn relaying_shim_rss_under_10_mb() {
    let dir = data_dir();
    let server = IpcServer::bind(&sock(dir.path())).unwrap().run(|a| {
        if let Accepted::Mcp { mut stream, .. } = a {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 || stream.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let mut child = shim(dir.path()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    // Relay 1 MiB, then measure while the shim is still relaying.
    let writer = tokio::spawn(async move {
        for _ in 0..16 {
            stdin.write_all(&[b'x'; 64 * 1024]).await.unwrap();
        }
        stdin
    });
    let mut got = vec![0u8; 16 * 64 * 1024];
    stdout.read_exact(&mut got).await.unwrap();
    let stdin = writer.await.unwrap();
    let ps = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &child.id().unwrap().to_string()])
        .output()
        .unwrap();
    let rss: u64 = String::from_utf8_lossy(&ps.stdout).trim().parse().unwrap();
    assert!(rss < 10_240, "shim rss {rss} KiB");
    drop(stdin);
    server.shutdown();
}
