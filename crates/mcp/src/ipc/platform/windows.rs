//! Windows: a per-user named pipe, owner-only descriptor, first-instance flag.

use std::{
    io,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use interprocess::os::windows::{
    named_pipe::{PipeListenerOptions, pipe_mode, tokio::DuplexPipeStream},
    security_descriptor::SecurityDescriptor,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use widestring::U16CString;

use super::Peer;
use crate::ipc::IpcError;

type Pipe = DuplexPipeStream<pipe_mode::Bytes>;
type PipeListener =
    interprocess::os::windows::named_pipe::tokio::PipeListener<pipe_mode::Bytes, pipe_mode::Bytes>;

/// SDDL for a protected DACL with a single allow entry (generic all) for `sid`.
pub fn owner_only_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

pub struct Stream(Pin<Box<Pipe>>);

impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.0.as_mut().poll_read(cx, buf)
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.0.as_mut().poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.as_mut().poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.as_mut().poll_shutdown(cx)
    }
}

pub struct Listener {
    inner: PipeListener,
    own_sid: String,
}

/// `ERROR_ACCESS_DENIED` and `ERROR_PIPE_BUSY`: what creating a first
/// instance reports when the pipe name is already taken.
const NAME_TAKEN: [i32; 2] = [5, 231];

impl Listener {
    pub fn bind(path: &Path) -> Result<Listener, IpcError> {
        let own_sid = settings::win_sid::current_user_sid()?;
        let sddl =
            U16CString::from_str(owner_only_sddl(&own_sid)).map_err(|_| IpcError::UnsafeDir)?;
        let sd = SecurityDescriptor::deserialize(&sddl)?;
        // `create_tokio` makes the first instance with FILE_FLAG_FIRST_PIPE_INSTANCE
        // and, with `accept_remote` false, PIPE_REJECT_REMOTE_CLIENTS.
        let inner = PipeListenerOptions::new()
            .path(path.as_os_str())
            .accept_remote(false)
            .security_descriptor(Some(sd))
            .create_tokio::<pipe_mode::Bytes, pipe_mode::Bytes>()
            .map_err(|e| match e.raw_os_error() {
                Some(code) if NAME_TAKEN.contains(&code) => IpcError::InUse,
                _ => IpcError::Io(e),
            })?;
        Ok(Listener { inner, own_sid })
    }

    pub async fn accept(&mut self) -> io::Result<(Stream, Peer)> {
        let pipe = self.inner.accept().await?;
        let pid = pipe.client_process_id().ok();
        // Pipe pid -> process token -> user SID, compared with ours. A process
        // this user cannot inspect is treated as not the owner.
        let allowed = pid
            .and_then(|p| settings::win_sid::process_user_sid(p).ok())
            .is_some_and(|sid| sid == self.own_sid);
        Ok((Stream(Box::pin(pipe)), Peer { allowed, pid }))
    }

    /// Stops listening; the pipe name is released when the instance drops.
    pub fn close(self) {}
}

pub async fn connect(path: &Path) -> io::Result<Stream> {
    Pipe::connect_by_path(path.as_os_str())
        .await
        .map(|p| Stream(Box::pin(p)))
}

/// Starts `cmd` detached from this console (`DETACHED_PROCESS`).
pub fn detach(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    cmd.creation_flags(DETACHED_PROCESS);
}
