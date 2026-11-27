//! Local MCP server for Parla: the neutral, GPUI-free half.
//! It holds the `Backend` seam the app implements, the tool DTOs, the policy,
//! audit, limits and confirmation layers, the tools and the local IPC. It
//! never depends on a provider crate or on GPUI.

pub mod audit;
pub mod backend;
pub mod confirm;
pub mod dto;
pub mod errors;
pub mod idempotency;
pub mod ids;
pub mod ipc;
pub mod limits;
pub mod policy;
pub mod render;
pub mod resources;
pub mod server;
pub mod shim;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod tools;

/// A failure a tool reports to the model as text (`isError: true`). The text
/// is written for an agent and carries no provider internals.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolError {
    #[error("invalid workspace {0:?}: expected \"<provider>/<id>\" as listed by list_workspaces")]
    InvalidWorkspace(String),
    #[error("invalid cursor: pass the next_cursor of the previous page unchanged")]
    InvalidCursor,
    #[error("invalid timestamp {0:?}: expected the ts of a message, such as \"1700000000.000100\"")]
    InvalidTimestamp(String),
}
