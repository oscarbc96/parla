//! `parla mcp`: a thin stdio shim (spec 8a Transports). It connects to the
//! running app's IPC endpoint, starting Parla headless if nobody listens, and
//! copies bytes between stdin/stdout and the endpoint. It parses no MCP, reads
//! no settings and no keychain, and writes nothing to stdout (the MCP channel)
//! before the handshake succeeds.

use std::{
    io,
    pin::Pin,
    process::{Command, ExitCode, Stdio},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};

use crate::ipc::{IpcError, IpcStream, connect, platform::detach};

/// How long to wait for a freshly started app to listen.
const START_TIMEOUT: Duration = Duration::from_secs(15);
const RETRY_EVERY: Duration = Duration::from_millis(100);

const EXIT_MCP_OFF: u8 = 2;
const EXIT_NO_START: u8 = 3;
const EXIT_BUSY: u8 = 4;
const EXIT_OTHER: u8 = 5;

/// Copies bytes both ways until either side closes. No parsing and no
/// buffering beyond `tokio::io::copy`'s own.
pub async fn relay<A, B>(client: A, app: B) -> io::Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (mut client_r, mut client_w) = tokio::io::split(client);
    let (mut app_r, mut app_w) = tokio::io::split(app);
    let up = async {
        tokio::io::copy(&mut client_r, &mut app_w).await?;
        // The client closed its output: tell the app, which then ends the session.
        app_w.shutdown().await
    };
    let down = async {
        tokio::io::copy(&mut app_r, &mut client_w).await?;
        client_w.flush().await
    };
    tokio::select! {
        r = up => r,
        r = down => r,
    }
}

/// stdin and stdout as one duplex stream.
struct Stdio2 {
    stdin: tokio::io::Stdin,
    stdout: tokio::io::Stdout,
}

impl AsyncRead for Stdio2 {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stdin).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stdio2 {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stdout).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stdout).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stdout).poll_shutdown(cx)
    }
}

/// The program that starts the app. Tests may override it in debug builds.
fn app_command() -> Command {
    #[cfg(debug_assertions)]
    if let Some(cmd) = std::env::var_os("PARLA_SHIM_APP_CMD").filter(|v| !v.is_empty()) {
        return Command::new(cmd);
    }
    Command::new(std::env::current_exe().unwrap_or_else(|_| "parla".into()))
}

fn start_timeout() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("PARLA_SHIM_START_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }
    START_TIMEOUT
}

fn start_app() -> io::Result<()> {
    let mut cmd = app_command();
    cmd.arg("--headless")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut cmd);
    // The child is not waited for: it outlives the shim.
    cmd.spawn().map(drop)
}

fn not_listening(e: &IpcError) -> bool {
    matches!(e, IpcError::NotFound | IpcError::Refused)
}

/// Connects, starting the app once if nothing listens.
async fn connect_or_start(path: &std::path::Path) -> Result<IpcStream, IpcError> {
    match connect(path, "mcp").await {
        Err(e) if not_listening(&e) => {}
        other => return other,
    }
    let deadline = Instant::now() + start_timeout();
    if let Err(e) = start_app() {
        return Err(IpcError::Io(e));
    }
    loop {
        match connect(path, "mcp").await {
            Err(e) if not_listening(&e) => {
                if Instant::now() >= deadline {
                    return Err(e);
                }
                tokio::time::sleep(RETRY_EVERY).await;
            }
            other => return other,
        }
    }
}

fn describe(path: &std::path::Path) -> String {
    path.file_name()
        .map_or_else(|| "mcp socket".into(), |n| n.to_string_lossy().into_owned())
}

async fn run() -> u8 {
    let path = settings::paths::mcp_socket_path();
    let stream = match connect_or_start(&path).await {
        Ok(s) => s,
        Err(IpcError::McpOff) => {
            eprintln!("Parla's MCP server is off. Turn on \"mcp.enabled\" in Parla's settings.");
            return EXIT_MCP_OFF;
        }
        Err(IpcError::Busy) => {
            eprintln!("Parla is serving too many MCP clients. Close one and try again.");
            return EXIT_BUSY;
        }
        Err(e) if not_listening(&e) => {
            eprintln!(
                "Parla did not start within {} s.",
                start_timeout().as_secs().max(1)
            );
            return EXIT_NO_START;
        }
        Err(IpcError::Io(_) | IpcError::UnsafeDir | IpcError::PathTooLong)
        | Err(IpcError::InUse) => {
            eprintln!("Could not reach Parla over {}.", describe(&path));
            return EXIT_OTHER;
        }
        Err(IpcError::Version) => {
            eprintln!("This Parla speaks another MCP relay version; update `parla`.");
            return EXIT_OTHER;
        }
        Err(IpcError::Protocol) => {
            eprintln!("Unexpected answer from {}.", describe(&path));
            return EXIT_OTHER;
        }
        Err(_) => return EXIT_OTHER,
    };
    let stdio = Stdio2 {
        stdin: tokio::io::stdin(),
        stdout: tokio::io::stdout(),
    };
    match relay(stdio, stream).await {
        Ok(()) => 0,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
            ) =>
        {
            0
        }
        Err(_) => {
            eprintln!("The connection to Parla ended unexpectedly.");
            EXIT_OTHER
        }
    }
}

/// Entry point of `parla mcp`: the process exit code.
pub fn main_code() -> u8 {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return EXIT_OTHER,
    };
    let code = rt.block_on(run());
    // A blocked stdin read must not keep the process alive after the relay ended.
    rt.shutdown_background();
    code
}

/// [`main_code`] as an `ExitCode`, for a `fn main` that returns one.
pub fn main() -> ExitCode {
    ExitCode::from(main_code())
}
