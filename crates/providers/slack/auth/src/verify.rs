//! Session verification through `auth.test`.

use slack_api::{ClientConfig, SessionCookie, WebClient, XoxcToken};

use crate::{DiscoveredWorkspace, WorkspaceCredentials};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Valid {
        user_id: String,
        team_name: String,
    },
    /// The server said the credentials are no longer usable.
    Expired,
    /// Could not tell (network, server error, rate limit). Never treated as expired.
    Unreachable(String),
}

async fn check(cfg: ClientConfig, token: XoxcToken, cookie: SessionCookie) -> SessionStatus {
    let client = match WebClient::new(cfg, token, cookie) {
        Ok(c) => c,
        Err(e) => return SessionStatus::Unreachable(e.to_string()),
    };
    match client.auth_test().await {
        Ok(a) => SessionStatus::Valid {
            user_id: a.user_id,
            team_name: a.team,
        },
        Err(e) if e.is_auth() => SessionStatus::Expired,
        Err(e) => SessionStatus::Unreachable(e.to_string()),
    }
}

pub async fn verify(c: &WorkspaceCredentials, cfg: ClientConfig) -> SessionStatus {
    check(cfg, c.token.duplicate(), c.cookie.duplicate()).await
}

/// One `auth.test` per team, sequential, same cookie.
pub async fn verify_discovered(
    cookie: &SessionCookie,
    found: Vec<DiscoveredWorkspace>,
) -> Vec<(DiscoveredWorkspace, SessionStatus)> {
    let mut out = Vec::with_capacity(found.len());
    for ws in found {
        let status = check(
            ClientConfig::for_workspace(&ws.url),
            ws.token.duplicate(),
            cookie.duplicate(),
        )
        .await;
        out.push((ws, status));
    }
    out
}
