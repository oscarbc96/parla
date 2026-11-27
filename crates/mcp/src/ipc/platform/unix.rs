//! Shared by macOS and Linux: a Unix socket in a private run dir.

use std::{
    fs::{self, DirBuilder},
    io,
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixStream as StdUnixStream,
    },
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use settings::paths::socket_path_fits;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{UnixListener, UnixStream},
};

use super::Peer;
use crate::ipc::IpcError;

/// The peer is the owner when its uid is ours.
pub fn check_peer(peer_uid: u32, own_uid: u32) -> bool {
    peer_uid == own_uid
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

pub struct Stream(UnixStream);

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

pub struct Listener {
    inner: UnixListener,
    own_uid: u32,
    path: std::path::PathBuf,
}

/// The run dir must be a real directory we own with no group/other access.
fn ensure_private_dir(dir: &Path) -> Result<(), IpcError> {
    match fs::symlink_metadata(dir) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        Err(e) => return Err(e.into()),
    }
    let md = fs::symlink_metadata(dir)?;
    if md.file_type().is_symlink() || !md.is_dir() || md.uid() != euid() || md.mode() & 0o077 != 0 {
        return Err(IpcError::UnsafeDir);
    }
    Ok(())
}

/// Clears a stale socket at `path`; refuses a live one or anything else.
fn clear_path(path: &Path) -> Result<(), IpcError> {
    let md = match fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if !md.file_type().is_socket() || md.uid() != euid() {
        return Err(IpcError::UnsafeDir);
    }
    if StdUnixStream::connect(path).is_ok() {
        return Err(IpcError::InUse);
    }
    fs::remove_file(path)?;
    Ok(())
}

impl Listener {
    pub fn bind(path: &Path) -> Result<Listener, IpcError> {
        if !socket_path_fits(path) {
            return Err(IpcError::PathTooLong);
        }
        let dir = path.parent().ok_or(IpcError::UnsafeDir)?;
        ensure_private_dir(dir)?;
        clear_path(path)?;
        // The 0700 run dir keeps others out until the socket is chmod-ed 0600.
        // (A process-wide umask swap would race other threads creating files.)
        let inner = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(Listener {
            inner,
            own_uid: euid(),
            path: path.to_path_buf(),
        })
    }

    pub async fn accept(&mut self) -> io::Result<(Stream, Peer)> {
        let (s, _) = self.inner.accept().await?;
        let peer = match s.peer_cred() {
            Ok(c) => Peer {
                allowed: check_peer(c.uid(), self.own_uid),
                pid: c.pid().and_then(|p| u32::try_from(p).ok()),
            },
            Err(_) => Peer {
                allowed: false,
                pid: None,
            },
        };
        Ok((Stream(s), peer))
    }

    /// Stops listening and removes the socket file.
    pub fn close(self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub async fn connect(path: &Path) -> io::Result<Stream> {
    UnixStream::connect(path).await.map(Stream)
}

/// Starts `cmd` detached from this process: its own session, no controlling
/// terminal, so it outlives the shim and the MCP client that started it.
pub fn detach(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: `setsid` is async-signal-safe and the closure touches nothing else.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}
