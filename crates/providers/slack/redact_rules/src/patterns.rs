//! Secret, identifier and file-host patterns of the Slack web client.

use regex::Regex;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern compiles")
}

/// Bot, user, cookie and app-level tokens (`xoxc-`, `xoxd-`, `xoxp-`, `xapp-`, ...).
/// A raw cookie may carry `/`, `+`, `=` and JSON escapes (`\/`), so those are part of the token.
/// The `[A-Za-z0-9]` after the dash keeps prose such as `xoxc-...` from matching.
pub fn tokens() -> Vec<Regex> {
    vec![
        re(r"(?:xox[a-zA-Z]|xapp)-[A-Za-z0-9][A-Za-z0-9%._/+=\\-]*"),
        // `frt` is the websocket fast-reconnect token carried in the socket URL and in
        // `reconnect_url` frames. At least 20 characters, so `frt=REDACTED` is left alone.
        re(r"\bfrt=[A-Za-z0-9%._=/-]{20,}"),
    ]
}

/// IDs: a type letter plus 8-10 upper-case alphanumerics. Synthetic IDs
/// (`T0000001`) have a 7-digit body and never match.
pub fn ids() -> Regex {
    re(r"\b[TUWCGDFBEARLS][A-Z0-9]{8,10}\b")
}

/// Hosts that serve user files and avatars (and gravatar), optionally with a scheme and path.
pub fn file_hosts() -> Regex {
    re(
        r#"(?i)(?:https?://|//)?[a-z0-9.-]*(?:slack-edge\.com|slack-files\.com|files\.slack\.com|gravatar\.com)(?:[/?#][^\s"'<>\\)\]]*)?"#,
    )
}

/// `app.slack.com/t/<slug>[/login/<code>]`, also percent-encoded or JSON-escaped.
/// The slug is the workspace name and the code is a one-time login code.
pub fn path_slugs() -> Regex {
    re(&format!(
        r"(?i)(?P<pre>app\.slack\.com{S}t{S})(?P<slug>[a-z0-9][a-z0-9-]*)(?:(?P<mid>{S}login{S})(?P<secret>{C}))?",
        S = redact::PATH_SLASH,
        C = redact::LOGIN_CODE,
    ))
}
