use std::collections::HashSet;

use serde::Deserialize;
use settings::WorkspaceKey;
use slack_api::XoxcToken;
use url::Url;

use crate::AuthError;

/// A workspace URL the session token and cookie may be sent to: `https`, no credentials or
/// port, on `slack.com` or `slack-gov.com`. Anything else could take them to another host.
pub fn is_workspace_url(url: &Url) -> bool {
    // Tests point the client at a mock server on the loopback interface.
    #[cfg(feature = "test-support")]
    if matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")) {
        return true;
    }
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    ["slack.com", "slack-gov.com"]
        .iter()
        .any(|d| host == *d || host.strip_suffix(d).is_some_and(|h| h.ends_with('.')))
}

/// Read-only script run in the sign-in webview: returns the raw local config.
pub const READ_TOKENS_SCRIPT: &str = r#"localStorage.getItem("localConfig_v2")"#;

/// A workspace found in the local config.
#[derive(Debug)]
pub struct DiscoveredWorkspace {
    pub workspace: WorkspaceKey,
    pub name: String,
    pub url: Url,
    pub token: XoxcToken,
    pub enterprise_id: Option<String>,
}

#[derive(Deserialize)]
struct Config {
    teams: std::collections::BTreeMap<String, Team>,
}

#[derive(Deserialize)]
struct Team {
    #[serde(default)]
    name: String,
    url: Option<String>,
    token: Option<String>,
    enterprise_id: Option<String>,
}

/// Parses the `teams` object of `localConfig_v2`. Entries without an `xoxc-` token are
/// skipped, as are enterprise org entries (`E...` ids or ids named as another entry's
/// `enterprise_id`). Sorted by name.
pub fn parse_local_config(json: &str) -> Result<Vec<DiscoveredWorkspace>, AuthError> {
    let cfg: Config = serde_json::from_str(json).map_err(|_| AuthError::BadConfig)?;
    let orgs: HashSet<&str> = cfg
        .teams
        .values()
        .filter_map(|t| t.enterprise_id.as_deref())
        .collect();
    let mut out = Vec::new();
    for (id, t) in &cfg.teams {
        if id.starts_with('E') || orgs.contains(id.as_str()) {
            continue;
        }
        let Some(token) = t.token.as_deref().and_then(|s| XoxcToken::parse(s).ok()) else {
            continue;
        };
        let Some(url) = t
            .url
            .as_deref()
            .and_then(|u| Url::parse(u).ok())
            .filter(is_workspace_url)
        else {
            continue;
        };
        out.push(DiscoveredWorkspace {
            workspace: WorkspaceKey::new("slack", id.clone()),
            name: t.name.clone(),
            url,
            token,
            enterprise_id: t.enterprise_id.clone().filter(|e| !e.is_empty()),
        });
    }
    if out.is_empty() {
        return Err(AuthError::NoTeams);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
