use slack_api::{ApiError, CredentialError};

/// Auth failures. `Display` never includes a secret.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("no workspaces with a session token were found")]
    NoTeams,
    #[error("the local configuration could not be read")]
    BadConfig,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error("keychain error: {0}")]
    Keychain(String),
    #[error("the session has expired")]
    Expired,
    #[error("webview error: {0}")]
    Webview(String),
}
