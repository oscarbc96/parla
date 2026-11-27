//! Local IPC between `parla mcp` shims (and a second Parla instance) and the
//! running app (spec 8a Transports, section 3 Platform layer "Local IPC").
//!
//! One owner-only endpoint: a Unix socket in a `0700` run dir, or a per-user
//! named pipe on Windows. Every connection starts with one JSON line each way
//! (the handshake) before any MCP byte. All `cfg` lives in `platform/`.

use std::{
    io,
    path::Path,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use serde::Deserialize;
use store::AuditOutcome;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use crate::{
    audit::{Auditor, ClientInfo},
    limits::ClientId,
};

pub mod platform;

/// Live connections served at once, handshake included. The next one gets `busy`.
pub const MAX_CONNECTIONS: usize = 8;
/// The hello line, newline included, never exceeds this.
const MAX_HELLO: usize = 256;
/// A peer has this long to send its hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
/// A client waits this long for the server's answer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// The run dir is a symlink, not ours, group/other accessible, or the path
    /// holds something that is not our socket.
    #[error("the socket directory is not private to this user")]
    UnsafeDir,
    /// Another instance is already listening.
    #[error("another Parla instance is already listening")]
    InUse,
    #[error("the socket path is too long for this platform")]
    PathTooLong,
    /// Nothing listens at the path.
    #[error("no Parla instance is listening")]
    NotFound,
    /// The endpoint exists but refused the connection.
    #[error("the connection was refused")]
    Refused,
    /// The app is running with MCP turned off.
    #[error("MCP is off")]
    McpOff,
    /// The connection cap was reached.
    #[error("too many connections")]
    Busy,
    /// The app speaks another protocol version.
    #[error("unsupported IPC version")]
    Version,
    /// The peer sent something that is not the handshake.
    #[error("invalid IPC handshake")]
    Protocol,
    #[error("IPC error: {0}")]
    Io(io::Error),
}

impl From<io::Error> for IpcError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => IpcError::NotFound,
            io::ErrorKind::ConnectionRefused => IpcError::Refused,
            _ => IpcError::Io(e),
        }
    }
}

/// What the app is handed for a connection that passed the handshake.
pub enum Accepted {
    /// An MCP session: serve it with `ParlaServer::serve`.
    Mcp {
        stream: IpcStream,
        /// The peer process (the shim, not the MCP client behind it).
        peer_pid: Option<u32>,
    },
    /// A second instance asks the running one to show its window.
    Show,
}

/// A connected, authenticated byte stream. Server-side streams end (reads see
/// EOF, writes fail) when the server shuts down.
pub struct IpcStream {
    inner: platform::Stream,
    cancel: Option<Pin<Box<WaitForCancellationFutureOwned>>>,
    _slot: Option<Slot>,
}

impl IpcStream {
    fn client(inner: platform::Stream) -> IpcStream {
        IpcStream {
            inner,
            cancel: None,
            _slot: None,
        }
    }

    fn cancelled(&mut self, cx: &mut Context<'_>) -> bool {
        self.cancel
            .as_mut()
            .is_some_and(|c| c.as_mut().poll(cx).is_ready())
    }
}

impl AsyncRead for IpcStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.cancelled(cx) {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for IpcStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.cancelled(cx) {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// One of the `MAX_CONNECTIONS` slots; frees itself on drop.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Deserialize)]
struct Hello {
    parla_ipc: u32,
    op: String,
}

/// Reads one `\n`-terminated line of at most `max` bytes, byte by byte so
/// nothing after the newline is consumed (MCP bytes follow the handshake).
async fn read_line<S: AsyncRead + Unpin>(s: &mut S, max: usize) -> Result<Vec<u8>, IpcError> {
    let mut line = Vec::with_capacity(64);
    loop {
        let mut b = [0u8; 1];
        if s.read(&mut b).await? == 0 {
            return Err(IpcError::Protocol);
        }
        if b[0] == b'\n' {
            return Ok(line);
        }
        line.push(b[0]);
        if line.len() >= max {
            return Err(IpcError::Protocol);
        }
    }
}

fn answer(error: Option<&str>) -> Vec<u8> {
    match error {
        None => b"{\"ok\":true}\n".to_vec(),
        Some(code) => format!("{{\"ok\":false,\"error\":\"{code}\"}}\n").into_bytes(),
    }
}

/// Records a peer that is not the owner. Content-free: no peer name, only the
/// pid the platform reported.
pub fn audit_denied_peer(auditor: &Auditor, peer_pid: Option<u32>) {
    let client = ClientInfo {
        id: ClientId(0),
        name: Arc::from("unknown"),
        peer_pid,
    };
    auditor
        .start(&client, "connect")
        .finish(AuditOutcome::Denied, Some("peer_uid"));
}

type Gate = Arc<dyn Fn() -> bool + Send + Sync>;

/// The listener, bound but not yet accepting.
pub struct IpcServer {
    listener: platform::Listener,
    gate: Gate,
    auditor: Option<Auditor>,
}

impl IpcServer {
    /// Creates the endpoint with owner-only access. See `platform` for the
    /// per-OS checks; fails with `UnsafeDir`, `InUse` or `PathTooLong`.
    pub fn bind(path: &Path) -> Result<IpcServer, IpcError> {
        Ok(IpcServer {
            listener: platform::Listener::bind(path)?,
            gate: Arc::new(|| true),
            auditor: None,
        })
    }

    /// `op mcp` answers `mcp_off` while the gate returns false (the app's
    /// `mcp.enabled` setting). Defaults to always on.
    pub fn with_mcp_gate(mut self, gate: Gate) -> IpcServer {
        self.gate = gate;
        self
    }

    /// Where rejected peers are recorded.
    pub fn with_auditor(mut self, auditor: Auditor) -> IpcServer {
        self.auditor = Some(auditor);
        self
    }

    /// Accepts until [`IpcHandle::shutdown`]. Needs a Tokio runtime.
    pub fn run(self, accept: impl Fn(Accepted) + Send + Sync + 'static) -> IpcHandle {
        let token = CancellationToken::new();
        let accept: Arc<dyn Fn(Accepted) + Send + Sync> = Arc::new(accept);
        let live = Arc::new(AtomicUsize::new(0));
        let IpcServer {
            mut listener,
            gate,
            auditor,
        } = self;
        let t = token.clone();
        tokio::spawn(async move {
            loop {
                let next = tokio::select! {
                    () = t.cancelled() => break,
                    r = listener.accept() => r,
                };
                let (stream, peer) = match next {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::warn!("mcp ipc accept failed: {e}");
                        // Back off so a persistent error (no fds) cannot spin.
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                if !peer.allowed {
                    tracing::warn!("peer uid mismatch");
                    if let Some(a) = &auditor {
                        audit_denied_peer(a, peer.pid);
                    }
                    continue;
                }
                let slot = Slot(live.clone());
                let over = live.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS;
                let conn = Conn {
                    stream,
                    pid: peer.pid,
                    slot,
                    over,
                    gate: gate.clone(),
                    accept: accept.clone(),
                    cancel: t.child_token(),
                };
                tokio::spawn(conn.run());
            }
            listener.close();
        });
        IpcHandle { token }
    }
}

struct Conn {
    stream: platform::Stream,
    pid: Option<u32>,
    slot: Slot,
    over: bool,
    gate: Gate,
    accept: Arc<dyn Fn(Accepted) + Send + Sync>,
    cancel: CancellationToken,
}

impl Conn {
    async fn run(self) {
        let cancel = self.cancel.clone();
        tokio::select! {
            () = cancel.cancelled() => {}
            () = self.handle() => {}
        }
    }

    async fn handle(mut self) {
        let hello =
            tokio::time::timeout(HELLO_TIMEOUT, read_line(&mut self.stream, MAX_HELLO)).await;
        let Ok(Ok(line)) = hello else {
            return;
        };
        let Ok(hello) = serde_json::from_slice::<Hello>(&line) else {
            return;
        };
        let refusal = if hello.parla_ipc != VERSION || !matches!(hello.op.as_str(), "mcp" | "show")
        {
            Some("version")
        } else if self.over {
            Some("busy")
        } else if hello.op == "mcp" && !(self.gate)() {
            Some("mcp_off")
        } else {
            None
        };
        if self.stream.write_all(&answer(refusal)).await.is_err()
            || self.stream.flush().await.is_err()
            || refusal.is_some()
        {
            return;
        }
        if hello.op == "show" {
            (self.accept)(Accepted::Show);
            return;
        }
        let stream = IpcStream {
            inner: self.stream,
            cancel: Some(Box::pin(self.cancel.cancelled_owned())),
            _slot: Some(self.slot),
        };
        (self.accept)(Accepted::Mcp {
            stream,
            peer_pid: self.pid,
        });
    }
}

/// Stops the listener and every live connection.
#[derive(Clone)]
pub struct IpcHandle {
    token: CancellationToken,
}

impl IpcHandle {
    /// Closes the listener and ends every connection; idempotent.
    pub fn shutdown(&self) {
        self.token.cancel();
    }
}

/// Connects without the handshake. For tests of the server's input handling.
#[doc(hidden)]
pub async fn connect_raw(path: &Path) -> Result<IpcStream, IpcError> {
    Ok(IpcStream::client(platform::connect(path).await?))
}

/// Connects and performs the handshake for `op` (`"mcp"` or `"show"`).
pub async fn connect(path: &Path, op: &str) -> Result<IpcStream, IpcError> {
    let mut stream = platform::connect(path).await?;
    let hello = format!("{{\"parla_ipc\":{VERSION},\"op\":\"{op}\"}}\n");
    stream.write_all(hello.as_bytes()).await?;
    stream.flush().await?;
    let line = tokio::time::timeout(REPLY_TIMEOUT, read_line(&mut stream, MAX_HELLO))
        .await
        .map_err(|_| IpcError::Protocol)??;
    #[derive(Deserialize)]
    struct Reply {
        ok: bool,
        error: Option<String>,
    }
    let reply: Reply = serde_json::from_slice(&line).map_err(|_| IpcError::Protocol)?;
    if reply.ok {
        return Ok(IpcStream::client(stream));
    }
    Err(match reply.error.as_deref() {
        Some("mcp_off") => IpcError::McpOff,
        Some("busy") => IpcError::Busy,
        Some("version") => IpcError::Version,
        _ => IpcError::Protocol,
    })
}
