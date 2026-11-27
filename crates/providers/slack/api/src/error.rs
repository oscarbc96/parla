//! API errors. Messages never contain request URLs, tokens or cookies.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// The session is gone (`invalid_auth` or a redirect to the sign-in page).
    #[error("session is no longer valid")]
    InvalidAuth,
    #[error("{method} failed: {error}")]
    Slack {
        method: &'static str,
        error: String,
        needed: Option<String>,
    },
    #[error("rate limited, retry after {retry_after:?}")]
    RateLimited { retry_after: Duration },
    #[error("http status {status}")]
    Http { status: u16 },
    #[error("transport error: {0}")]
    Transport(String),
    /// A client configuration problem (for example an unusable base URL); retrying cannot help.
    #[error("configuration error: {0}")]
    Config(String),
    /// A file URL outside the allowed file hosts; nothing was sent.
    #[error("refusing to send credentials to a host that is not a file host")]
    ForeignHost,
    #[error("file is larger than {max} bytes")]
    TooLarge { max: u64 },
    #[error("local file error: {0}")]
    Io(String),
    /// An email sign-in code was wrong; `attempts_left` when the service says.
    #[error("the code is wrong")]
    WrongCode { attempts_left: Option<u8> },
    /// An email sign-in code expired; a new one has to be sent.
    #[error("the code has expired")]
    CodeExpired,
    /// Too many wrong codes; a new one has to be sent later.
    #[error("too many wrong codes")]
    TooManyAttempts,
    #[error("{method}: cannot decode response at {pointer}: {message}")]
    Decode {
        method: &'static str,
        pointer: String,
        message: String,
    },
}

/// Error codes that mean the session is gone.
pub const AUTH_ERRORS: &[&str] = &[
    "invalid_auth",
    "not_authed",
    "account_inactive",
    "token_revoked",
    "token_expired",
];

impl ApiError {
    /// True when the credentials are no longer usable.
    pub fn is_auth(&self) -> bool {
        match self {
            ApiError::InvalidAuth => true,
            ApiError::Slack { error, .. } => AUTH_ERRORS.contains(&error.as_str()),
            _ => false,
        }
    }
}
