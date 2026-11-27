//! Per-OS endpoint code: the only place in `mcp` with `cfg` on the OS
//! (spec 3 Platform layer). Each platform provides `Listener`, `Stream`,
//! `Peer` and `connect`.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(windows)]
pub use windows::*;

/// Who is on the other end of an accepted connection.
pub struct Peer {
    /// False when the peer is not the user running this process.
    pub allowed: bool,
    /// The peer process, when the OS tells (the shim, not the MCP client).
    pub pid: Option<u32>,
}
