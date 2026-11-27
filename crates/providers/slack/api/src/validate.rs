//! Shared syntax checks for Slack ids and hosts. Ids and hosts reach Parla from the
//! network, so everything that builds a URL or a cookie scope from them validates here.

fn all_upper_or_digit(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// An id made of `prefix` followed by at least one upper-case letter or digit (`T0000001`).
pub fn is_id(s: &str, prefix: char) -> bool {
    s.starts_with(prefix) && s.len() > 1 && all_upper_or_digit(s)
}

/// A team id (`T...`).
pub fn is_team_id(s: &str) -> bool {
    is_id(s, 'T')
}

/// A conversation id of any kind (`C...`, `G...`, `D...`): non-empty upper-case letters
/// and digits only.
pub fn is_conversation_id(s: &str) -> bool {
    !s.is_empty() && all_upper_or_digit(s)
}

/// `slack.com` or one of its subdomains.
pub fn is_slack_host(host: &str) -> bool {
    host == "slack.com" || host.ends_with(".slack.com")
}
