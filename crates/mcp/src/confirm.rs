//! The confirmation seam: a write that needs the
//! user's yes is described to a [`Confirmer`], which the app implements with
//! its confirmation window. The server applies the timeout around it.
//!
//! A thin adapter over `write_confirm`, which MCP shares with plugins: the
//! actions and the timeout are re-exported, and an MCP [`WriteRequest`] and
//! [`Confirmation`] convert to and from the shared ones.

use std::sync::Arc;

use futures::future::{BoxFuture, ready};

pub use write_confirm::{CONFIRM_TIMEOUT, WriteAction};

use crate::{audit::ClientInfo, backend::WorkspaceInfo};

#[derive(Debug, Clone)]
pub struct WriteRequest {
    pub client: ClientInfo,
    pub workspace: WorkspaceInfo,
    /// The display name of the user the write is made as, when known.
    pub as_user: Option<Arc<str>>,
    pub action: WriteAction,
}

impl WriteRequest {
    /// The write as the agent asked for it, as pretty JSON (the prompt's
    /// Details disclosure). Names stand in for ids, like the rest of the
    /// prompt; message text is carried exactly.
    pub fn details(&self) -> String {
        write_confirm::WriteRequest::from(self.clone()).details()
    }
}

impl From<WriteRequest> for write_confirm::WriteRequest {
    fn from(r: WriteRequest) -> Self {
        write_confirm::WriteRequest::new(
            write_confirm::Requester::Mcp(r.client),
            write_confirm::Workspace {
                key: r.workspace.key,
                name: r.workspace.name,
            },
            r.as_user,
            r.action,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// Allow once: this write only.
    Approved,
    /// Allow this write and, for the rest of the session, later writes of
    /// the same tool to the same destination without asking.
    ApprovedForSession,
    Declined,
    TimedOut,
}

impl From<write_confirm::Confirmation> for Confirmation {
    fn from(c: write_confirm::Confirmation) -> Self {
        match c {
            write_confirm::Confirmation::Approved { remember: false } => Confirmation::Approved,
            write_confirm::Confirmation::Approved { remember: true } => {
                Confirmation::ApprovedForSession
            }
            write_confirm::Confirmation::Declined => Confirmation::Declined,
            write_confirm::Confirmation::TimedOut => Confirmation::TimedOut,
        }
    }
}

/// Asks the user to confirm a write. The future may stay pending for a long
/// time; the server drops it at [`CONFIRM_TIMEOUT`].
pub trait Confirmer: Send + Sync + 'static {
    fn confirm(&self, req: WriteRequest) -> BoxFuture<'static, Confirmation>;
}

/// Declines every write; for tests and for when no UI exists.
pub struct DenyAllConfirmer;

impl Confirmer for DenyAllConfirmer {
    fn confirm(&self, _req: WriteRequest) -> BoxFuture<'static, Confirmation> {
        Box::pin(ready(Confirmation::Declined))
    }
}
