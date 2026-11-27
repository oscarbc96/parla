use slack_api::{SessionCookie, XoxcToken};

use crate::AuthError;

/// One-liner for the browser console: prints the local config.
pub const MANUAL_CONSOLE_SNIPPET: &str = r#"copy(localStorage.getItem("localConfig_v2"))"#;

/// Cookie plus one or more tokens pasted by the user.
#[derive(Debug)]
pub struct ManualInput {
    pub cookie: SessionCookie,
    pub tokens: Vec<XoxcToken>,
}

/// Tokens are separated by whitespace (lines or spaces); duplicates are removed.
pub fn parse_manual(cookie: &str, tokens: &str) -> Result<ManualInput, AuthError> {
    let cookie = SessionCookie::parse(cookie)?;
    let mut out: Vec<XoxcToken> = Vec::new();
    for raw in tokens.split_whitespace() {
        let t = XoxcToken::parse(raw)?;
        if !out.iter().any(|o| o.expose() == t.expose()) {
            out.push(t);
        }
    }
    if out.is_empty() {
        return Err(slack_api::CredentialError::Empty.into());
    }
    Ok(ManualInput {
        cookie,
        tokens: out,
    })
}
