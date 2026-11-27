//! The connect URL and the sources that produce it.

use std::fmt;

use ::url::Url;
use futures::future::BoxFuture;
use slack_api::{ApiError, WebClient, XoxcToken};

use crate::conn::UrlSource;

/// Host the web client connects to (spike 2026-11-13).
pub const WS_BASE: &str = "wss://wss-primary.slack.com/";

/// The URL `app.slack.com` opens, with the parameters in the captured order, on
/// `base` (the web client's host is [`WS_BASE`]; tests point it at a loopback server).
/// The token appears only in the returned `Url`, which must never be logged.
pub fn ws_url_on(base: &Url, team_id: &str, token: &XoxcToken, agent_version: &str) -> Url {
    let start_args = format!(
        "?agent=client&org_wide_aware=true&agent_version={agent_version}&eac_cache_ts=true\
&cache_ts=0&name_tagging=true&only_self_subteams=true&connect_only=true&ms_latest=true"
    );
    let mut url = base.clone();
    url.set_query(None);
    url.query_pairs_mut()
        .append_pair("token", token.expose())
        .append_pair("sync_desync", "1")
        .append_pair("slack_client", "desktop")
        .append_pair("start_args", &start_args)
        .append_pair("no_query_on_subscribe", "1")
        .append_pair("flannel", "3")
        .append_pair("lazy_channels", "1")
        .append_pair("gateway_server", &format!("{team_id}-1"))
        .append_pair("batch_presence_aware", "1");
    url
}

/// Builds the URL the way the web client does, after confirming the session with
/// `auth.test` so an expired session surfaces as an auth error before any socket
/// attempt.
pub struct ClientBuilt {
    pub team_id: String,
    pub token: XoxcToken,
    pub client: WebClient,
    /// The web client's build timestamp (`_x_version_ts`).
    pub agent_version: String,
    /// Defaults to [`WS_BASE`].
    pub base: Url,
}

impl ClientBuilt {
    pub fn new(
        team_id: String,
        token: XoxcToken,
        client: WebClient,
        agent_version: String,
    ) -> ClientBuilt {
        ClientBuilt {
            team_id,
            token,
            client,
            agent_version,
            base: Url::parse(WS_BASE).expect("static url"),
        }
    }
}

impl fmt::Debug for ClientBuilt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientBuilt")
            .field("team_id", &self.team_id)
            .field("credentials", &"REDACTED")
            .finish()
    }
}

impl UrlSource for ClientBuilt {
    fn fetch(&self) -> BoxFuture<'_, Result<Url, ApiError>> {
        Box::pin(async move {
            self.client.auth_test().await?;
            Ok(ws_url_on(
                &self.base,
                &self.team_id,
                &self.token,
                &self.agent_version,
            ))
        })
    }
}
