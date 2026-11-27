//! the owner-only IPC listener (spec 8a Transports, section 12).

mod common;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use domain::test_support::FakeProvider;
use mcp::{
    audit::Auditor,
    ipc::{Accepted, IpcError, IpcHandle, IpcServer, MAX_CONNECTIONS, connect},
    limits::Limits,
    server::{DenyAllConfirmer, ParlaServer},
    test_support::FakeBackend,
};
use rmcp::{ServiceExt, model::CallToolRequestParams};
use store::{AppStore, MemoryAppStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A short temp dir (macOS `sun_path` is 104 bytes).
fn short_dir() -> tempfile::TempDir {
    #[cfg(unix)]
    {
        tempfile::Builder::new()
            .prefix("t")
            .tempdir_in("/tmp")
            .unwrap()
    }
    #[cfg(not(unix))]
    {
        tempfile::tempdir().unwrap()
    }
}

/// An endpoint path inside `dir`: a socket file on Unix, a unique pipe name on Windows.
fn endpoint(dir: &tempfile::TempDir) -> std::path::PathBuf {
    #[cfg(unix)]
    {
        let run = dir.path().join("run");
        run.join("mcp.sock")
    }
    #[cfg(not(unix))]
    {
        let n = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        std::path::PathBuf::from(format!(r"\\.\pipe\parla-mcp-test-{n}"))
    }
}

/// A server whose accept callback parks every MCP stream in a task that reads
/// until EOF, counting what it saw.
struct Rig {
    handle: IpcHandle,
    mcp: Arc<AtomicUsize>,
    show: Arc<AtomicUsize>,
}

fn serve(server: IpcServer) -> Rig {
    let mcp = Arc::new(AtomicUsize::new(0));
    let show = Arc::new(AtomicUsize::new(0));
    let (m, s) = (mcp.clone(), show.clone());
    let handle = server.run(move |a| match a {
        Accepted::Mcp { mut stream, .. } => {
            m.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = [0u8; 64];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                }
            });
        }
        Accepted::Show => {
            s.fetch_add(1, Ordering::SeqCst);
        }
    });
    Rig { handle, mcp, show }
}

#[cfg(unix)]
mod unix_only {
    use super::*;

    /// A forked child (the `lsof` probe) briefly shares every open socket, which
    /// would make a just-dropped stale listener look live. Both tests take this.
    static FORK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[tokio::test]
    async fn socket_is_0600_dir_0700() {
        let dir = short_dir();
        let path = endpoint(&dir);
        let server = IpcServer::bind(&path).unwrap();
        let sock = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let rundir = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(sock, 0o600);
        assert_eq!(rundir, 0o700);
        drop(server);
    }

    #[tokio::test]
    async fn socket_dir_with_loose_mode_refuses_to_listen() {
        let dir = short_dir();
        let run = dir.path().join("run");
        std::fs::create_dir(&run).unwrap();
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = IpcServer::bind(&run.join("mcp.sock")).err().unwrap();
        assert!(matches!(err, IpcError::UnsafeDir), "{err:?}");
        // 0710 (group execute) is loose too.
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o710)).unwrap();
        let err = IpcServer::bind(&run.join("mcp.sock")).err().unwrap();
        assert!(matches!(err, IpcError::UnsafeDir), "{err:?}");
    }

    #[tokio::test]
    async fn symlinked_dir_refuses_to_listen() {
        let dir = short_dir();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = dir.path().join("run");
        symlink(&real, &link).unwrap();
        let err = IpcServer::bind(&link.join("mcp.sock")).err().unwrap();
        assert!(matches!(err, IpcError::UnsafeDir), "{err:?}");
    }

    #[tokio::test]
    async fn path_too_long_for_sun_path_is_refused() {
        let dir = short_dir();
        let long = dir.path().join("r".repeat(120)).join("mcp.sock");
        let err = IpcServer::bind(&long).err().unwrap();
        assert!(matches!(err, IpcError::PathTooLong), "{err:?}");
    }

    #[tokio::test]
    async fn non_socket_file_at_the_path_is_refused() {
        let dir = short_dir();
        let path = endpoint(&dir);
        std::fs::create_dir(path.parent().unwrap()).unwrap();
        std::fs::set_permissions(
            path.parent().unwrap(),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        std::fs::write(&path, b"x").unwrap();
        let err = IpcServer::bind(&path).err().unwrap();
        assert!(matches!(err, IpcError::UnsafeDir), "{err:?}");
        assert!(path.exists(), "a regular file must not be deleted");
    }

    #[tokio::test]
    async fn stale_socket_is_replaced_live_one_is_in_use() {
        let _fork = FORK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = short_dir();
        let path = endpoint(&dir);
        let run = path.parent().unwrap();
        std::fs::create_dir(run).unwrap();
        std::fs::set_permissions(run, std::fs::Permissions::from_mode(0o700)).unwrap();
        // A stale socket: bound and dropped, the file stays.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let first = IpcServer::bind(&path).expect("stale socket is replaced");
        let _rig = serve(first);
        let err = IpcServer::bind(&path).err().unwrap();
        assert!(matches!(err, IpcError::InUse), "{err:?}");
    }

    #[test]
    fn peer_with_other_uid_is_rejected() {
        use mcp::ipc::platform::check_peer;
        assert!(!check_peer(501, 502));
        assert!(check_peer(501, 501));
    }

    #[tokio::test]
    async fn live_same_uid_connection_passes() {
        let dir = short_dir();
        let path = endpoint(&dir);
        let rig = serve(IpcServer::bind(&path).unwrap());
        let _c = connect(&path, "mcp").await.unwrap();
        wait_for(|| rig.mcp.load(Ordering::SeqCst) == 1).await;
    }

    #[tokio::test]
    async fn no_tcp_or_udp_socket_is_opened() {
        let dir = short_dir();
        let path = endpoint(&dir);
        let rig = serve(IpcServer::bind(&path).unwrap());
        let _c = connect(&path, "mcp").await.unwrap();
        wait_for(|| rig.mcp.load(Ordering::SeqCst) == 1).await;
        let pid = std::process::id().to_string();
        if cfg!(target_os = "linux") {
            for f in ["tcp", "tcp6", "udp", "udp6"] {
                let inodes = own_socket_inodes();
                let table = std::fs::read_to_string(format!("/proc/self/net/{f}")).unwrap();
                for line in table.lines().skip(1) {
                    let inode = line.split_whitespace().nth(9).unwrap_or("0");
                    assert!(!inodes.contains(&inode.to_string()), "{f} socket: {line}");
                }
            }
        } else {
            let _fork = FORK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            match std::process::Command::new("lsof")
                .args(["-a", "-p", &pid, "-i"])
                .output()
            {
                Ok(out) => assert!(
                    String::from_utf8_lossy(&out.stdout).trim().is_empty(),
                    "network sockets: {}",
                    String::from_utf8_lossy(&out.stdout)
                ),
                Err(_) => eprintln!("skipped: no lsof"),
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn own_socket_inodes() -> Vec<String> {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
            .filter_map(|l| {
                let s = l.to_string_lossy().to_string();
                s.strip_prefix("socket:[")?
                    .strip_suffix(']')
                    .map(String::from)
            })
            .collect()
    }

    #[cfg(not(target_os = "linux"))]
    fn own_socket_inodes() -> Vec<String> {
        Vec::new()
    }
}

async fn wait_for(cond: impl Fn() -> bool) {
    for _ in 0..200 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached in 2 s");
}

#[tokio::test]
async fn handshake_mcp_then_tool_call() {
    let dir = short_dir();
    let path = endpoint(&dir);

    let backend = Arc::new(FakeBackend::new(vec![Arc::new(FakeProvider::new(
        "W1",
        domain::test_support::uid("u1"),
    ))]));
    let settings = Arc::new(common::Settings(
        Mutex::new(settings::McpSettings {
            enabled: true,
            ..Default::default()
        }),
        AtomicBool::new(false),
    ));
    let limits = Arc::new(Limits::system());
    let auditor = Auditor::new(Arc::new(MemoryAppStore::default()), settings.clone(), || 1);
    let server = IpcServer::bind(&path).unwrap();
    let _handle = server.run(move |a| {
        if let Accepted::Mcp { stream, peer_pid } = a {
            let s = ParlaServer::new(
                backend.clone(),
                settings.clone(),
                limits.clone(),
                auditor.clone(),
                Arc::new(DenyAllConfirmer),
                peer_pid,
            );
            tokio::spawn(s.serve(stream));
        }
    });

    let stream = connect(&path, "mcp").await.unwrap();
    let client = ().serve(stream).await.expect("initialize over ipc");
    let r = client
        .call_tool(CallToolRequestParams::new("list_workspaces"))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true));
    assert!(!serde_json::to_value(&r).unwrap()["content"].is_null());
}

/// Sends raw bytes as the hello and returns whatever the server answers.
async fn raw_hello(path: &std::path::Path, hello: &[u8]) -> Vec<u8> {
    // Reuse the client connect for the transport, but skip its own hello by
    // going through a throwaway op, so use the lower level entry point.
    let mut s = mcp::ipc::connect_raw(path).await.unwrap();
    let _ = s.write_all(hello).await;
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(4), s.read_to_end(&mut out)).await;
    out
}

#[tokio::test]
async fn handshake_rejects_garbage_and_oversized_hello() {
    let dir = short_dir();
    let path = endpoint(&dir);
    let rig = serve(IpcServer::bind(&path).unwrap());

    assert!(raw_hello(&path, b"not json\n").await.is_empty());
    let mut big = vec![b'a'; 1024];
    big.push(b'\n');
    assert!(raw_hello(&path, &big).await.is_empty());
    let version = raw_hello(&path, b"{\"parla_ipc\":9,\"op\":\"mcp\"}\n").await;
    assert_eq!(version, b"{\"ok\":false,\"error\":\"version\"}\n");
    // Silence: closed after the 2 s hello deadline.
    let t = std::time::Instant::now();
    assert!(raw_hello(&path, b"").await.is_empty());
    assert!(t.elapsed() < Duration::from_secs(4));
    assert_eq!(rig.mcp.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn show_op_is_accepted_and_closed() {
    let dir = short_dir();
    let path = endpoint(&dir);
    let rig = serve(IpcServer::bind(&path).unwrap());
    let _ = connect(&path, "show").await.unwrap();
    wait_for(|| rig.show.load(Ordering::SeqCst) == 1).await;
    assert_eq!(rig.mcp.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn mcp_off_answers_mcp_off() {
    let dir = short_dir();
    let path = endpoint(&dir);
    let on = Arc::new(AtomicBool::new(false));
    let g = on.clone();
    let rig = serve(
        IpcServer::bind(&path)
            .unwrap()
            .with_mcp_gate(Arc::new(move || g.load(Ordering::SeqCst))),
    );
    let err = connect(&path, "mcp").await.err().unwrap();
    assert!(matches!(err, IpcError::McpOff), "{err:?}");
    // `show` still works while MCP is off, and turning MCP on opens the gate.
    let _ = connect(&path, "show").await.unwrap();
    on.store(true, Ordering::SeqCst);
    let _c = connect(&path, "mcp").await.unwrap();
    wait_for(|| rig.mcp.load(Ordering::SeqCst) == 1).await;
}

#[tokio::test]
async fn ninth_connection_is_busy() {
    let dir = short_dir();
    let path = endpoint(&dir);
    let rig = serve(IpcServer::bind(&path).unwrap());
    let mut held = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        held.push(connect(&path, "mcp").await.unwrap());
    }
    wait_for(|| rig.mcp.load(Ordering::SeqCst) == MAX_CONNECTIONS).await;
    let err = connect(&path, "mcp").await.err().unwrap();
    assert!(matches!(err, IpcError::Busy), "{err:?}");
    // Closing one frees a slot.
    held.pop();
    for _ in 0..200 {
        if connect(&path, "mcp").await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("slot not freed");
}

#[tokio::test]
async fn shutdown_drops_live_connections_within_one_second() {
    let dir = short_dir();
    let path = endpoint(&dir);
    let rig = serve(IpcServer::bind(&path).unwrap());
    let mut c = connect(&path, "mcp").await.unwrap();
    wait_for(|| rig.mcp.load(Ordering::SeqCst) == 1).await;
    rig.handle.shutdown();
    let mut buf = [0u8; 8];
    let r = tokio::time::timeout(Duration::from_secs(1), c.read(&mut buf))
        .await
        .expect("connection closed within 1 s");
    assert!(matches!(r, Ok(0) | Err(_)));
    for _ in 0..100 {
        if connect(&path, "mcp").await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("listener still accepts after shutdown");
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_removes_the_socket_file() {
    let dir = short_dir();
    let path = endpoint(&dir);
    let rig = serve(IpcServer::bind(&path).unwrap());
    rig.handle.shutdown();
    wait_for(|| !path.exists()).await;
}

#[tokio::test]
async fn denied_peer_is_audited_without_content() {
    // The audit row for a uid mismatch is produced by `deny_row`; the live
    // path cannot fake a foreign uid, so test the row builder directly.
    let store = Arc::new(MemoryAppStore::default());
    let settings = Arc::new(common::Settings(
        Mutex::new(Default::default()),
        AtomicBool::new(false),
    ));
    let auditor = Auditor::new(store.clone(), settings, || 5);
    mcp::ipc::audit_denied_peer(&auditor, Some(42));
    auditor.flush().await;
    let rows = store
        .audit_page(store::AuditQuery {
            limit: 10,
            ..Default::default()
        })
        .await
        .expect("query audit");
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(&*r.tool, "connect");
    assert_eq!(r.outcome, store::AuditOutcome::Denied);
    assert_eq!(r.detail.as_deref(), Some("peer_uid"));
    assert_eq!(r.peer_pid, Some(42));
}
