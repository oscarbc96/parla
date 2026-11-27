//! Workspace keys and page cursors as they appear in tool arguments.

use std::sync::Arc;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use domain::MessageTs;
use settings::WorkspaceKey;

use crate::ToolError;

/// Parses `<provider>/<id>`.
pub fn parse_workspace(s: &str) -> Result<WorkspaceKey, ToolError> {
    s.parse()
        .map_err(|_| ToolError::InvalidWorkspace(s.chars().take(MAX_ECHO).collect()))
}

/// How much of a bad argument an error message repeats.
const MAX_ECHO: usize = 64;

/// Where the next page of an older-than listing starts: strictly before this
/// message. Opaque to clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub before: MessageTs,
}

impl Cursor {
    /// URL-safe base64 (no padding) of the timestamp text.
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.before.0.as_bytes())
    }

    pub fn decode(s: &str) -> Result<Cursor, ToolError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| ToolError::InvalidCursor)?;
        let text = String::from_utf8(bytes).map_err(|_| ToolError::InvalidCursor)?;
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        let (secs, frac) = text.split_once('.').unwrap_or((&text, "0"));
        if text.len() > 32 || !digits(secs) || !digits(frac) {
            return Err(ToolError::InvalidCursor);
        }
        Ok(Cursor {
            before: MessageTs(Arc::from(text)),
        })
    }
}

/// Parses a message timestamp argument (`1700000000.000100`).
pub fn parse_timestamp(s: &str) -> Result<MessageTs, ToolError> {
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    let (secs, frac) = s.split_once('.').unwrap_or((s, "0"));
    if s.len() > 32 || !digits(secs) || !digits(frac) {
        return Err(ToolError::InvalidTimestamp(s.chars().take(40).collect()));
    }
    Ok(MessageTs(Arc::from(s)))
}
