use std::path::Path;

use components::sign_in::{Candidate, SignInError};
use slack_auth::{AuthError, parse_local_config, parse_manual};

use super::finish::complete_capture;
use crate::factory::CredentialBackend;

fn failed(e: AuthError) -> SignInError {
    SignInError::Failed(super::auth_error_text(&e))
}

/// Verifies and saves what the user pasted: the `d` cookie and the console
/// output (the local config, which holds the token and URL of each workspace).
pub async fn complete_manual(
    backend: &dyn CredentialBackend,
    accounts_path: &Path,
    cookie: &str,
    config: &str,
) -> Result<Vec<Candidate>, SignInError> {
    let found = parse_local_config(config.trim()).map_err(failed)?;
    let tokens: Vec<&str> = found.iter().map(|w| w.token.expose()).collect();
    let input = parse_manual(cookie, &tokens.join(" ")).map_err(failed)?;
    complete_capture(backend, accounts_path, input.cookie, found).await
}
