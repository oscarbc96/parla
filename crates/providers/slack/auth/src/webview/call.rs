//! Call window helpers: huddle URLs and the call window's host policy constants.

use slack_api::validate::{is_conversation_id, is_team_id};
use url::Url;

const APP_HOST: &str = "app.slack.com";

/// Host suffix allowed in the call window (`slack.com` and its subdomains).
pub const CALL_HOST_SUFFIX: &str = "slack.com";

/// Paths on an allowed host that mean the session is signed out.
pub const CALL_SIGNED_OUT_PATHS: &[&str] = &["/signin", "/workspace-signin"];

/// The huddle surface for a conversation: `https://app.slack.com/huddle/<team>/<channel>`,
/// the `huddle_link` seen in captured huddle payloads. `None` when an id is malformed.
pub fn huddle_url(team_id: &str, channel_id: &str) -> Option<Url> {
    if !is_team_id(team_id) || !is_conversation_id(channel_id) {
        return None;
    }
    Url::parse(&format!("https://{APP_HOST}/huddle/{team_id}/{channel_id}")).ok()
}
