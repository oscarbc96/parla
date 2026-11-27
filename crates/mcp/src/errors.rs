//! Tool failures. They reach the agent as a tool result with `isError: true`
//! and a small JSON body, never as a JSON-RPC protocol error. Unknown and
//! hidden things share one code and one message so a client cannot probe
//! which conversations exist.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::json;
use store::AuditOutcome;

use crate::{ToolError, backend::BackendError, limits::Limited, policy::Deny};

/// The codes a client sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    NotAvailable,
    RateLimited,
    WritesOff,
    Declined,
    TimedOut,
    ProviderError,
    Offline,
    BadArgument,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::NotAvailable => "not_available",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::WritesOff => "writes_off",
            ErrorCode::Declined => "declined",
            ErrorCode::TimedOut => "timed_out",
            ErrorCode::ProviderError => "provider_error",
            ErrorCode::Offline => "offline",
            ErrorCode::BadArgument => "bad_argument",
        }
    }
}

/// The one message for an unknown or hidden workspace or conversation.
pub const NOT_AVAILABLE_MESSAGE: &str =
    "not available: it does not exist or is not shared with MCP clients";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFailure {
    pub code: ErrorCode,
    pub message: String,
    pub retry_after_ms: Option<u64>,
    /// What the audit log records: the outcome and a fixed, content-free reason.
    pub outcome: AuditOutcome,
    pub detail: &'static str,
}

impl ToolFailure {
    /// Unknown to the backend.
    pub fn unknown() -> Self {
        Self {
            code: ErrorCode::NotAvailable,
            message: NOT_AVAILABLE_MESSAGE.into(),
            retry_after_ms: None,
            outcome: AuditOutcome::NotAvailable,
            detail: "not_found",
        }
    }

    pub fn bad_argument(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::BadArgument,
            message: message.into(),
            retry_after_ms: None,
            outcome: AuditOutcome::Failed,
            detail: "bad_argument",
        }
    }

    /// The user said no. Written for the model: do not retry on your own.
    pub fn declined() -> Self {
        Self {
            code: ErrorCode::Declined,
            message: "the user declined this action; nothing was sent or changed. Do not retry unless the user asks you to".into(),
            retry_after_ms: None,
            outcome: AuditOutcome::Declined,
            detail: "declined",
        }
    }

    /// The user did not answer in time.
    pub fn timed_out() -> Self {
        Self {
            code: ErrorCode::TimedOut,
            message: "the user did not answer in time; nothing was sent or changed. Ask the user whether they still want it before trying again".into(),
            retry_after_ms: None,
            outcome: AuditOutcome::TimedOut,
            detail: "timed_out",
        }
    }

    pub fn into_result(self) -> CallToolResult {
        let mut error = json!({"code": self.code.as_str(), "message": self.message});
        if let Some(ms) = self.retry_after_ms {
            error["retry_after_ms"] = json!(ms);
        }
        let body = json!({ "error": error });
        let mut r = CallToolResult::error(vec![ContentBlock::text(body.to_string())]);
        r.structured_content = Some(body);
        r
    }
}

impl From<Deny> for ToolFailure {
    fn from(d: Deny) -> Self {
        let code = match d {
            Deny::WritesOff => ErrorCode::WritesOff,
            _ => ErrorCode::NotAvailable,
        };
        let message = match d {
            Deny::WritesOff => "writes are off for this workspace".to_string(),
            _ => NOT_AVAILABLE_MESSAGE.to_string(),
        };
        Self {
            code,
            message,
            retry_after_ms: None,
            outcome: AuditOutcome::Denied,
            detail: d.code(),
        }
    }
}

impl From<Limited> for ToolFailure {
    fn from(l: Limited) -> Self {
        Self {
            code: ErrorCode::RateLimited,
            message: format!("rate limited: try again in {} ms", l.retry_after_ms),
            retry_after_ms: Some(l.retry_after_ms),
            outcome: AuditOutcome::RateLimited,
            detail: "rate_limited",
        }
    }
}

impl From<ToolError> for ToolFailure {
    fn from(e: ToolError) -> Self {
        Self::bad_argument(e.to_string())
    }
}

impl From<BackendError> for ToolFailure {
    fn from(e: BackendError) -> Self {
        match e {
            BackendError::NotFound => Self::unknown(),
            BackendError::Offline => Self {
                code: ErrorCode::Offline,
                message: "the workspace is offline; a message you sent is queued and goes out when it reconnects, so do not send it again".into(),
                retry_after_ms: None,
                outcome: AuditOutcome::Failed,
                detail: "offline",
            },
            // Provider text can echo remote content; keep it out of the result.
            BackendError::Provider(_) | BackendError::Closed => Self {
                code: ErrorCode::ProviderError,
                message: "the provider could not complete the request".into(),
                retry_after_ms: None,
                outcome: AuditOutcome::Failed,
                detail: "provider_error",
            },
        }
    }
}
