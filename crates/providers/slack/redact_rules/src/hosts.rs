//! The `slack.com` host suffix and its infrastructure keep-list.

use redact::HostSuffix;

/// Subdomains of `slack.com` that are not workspace names. Everything else is
/// rewritten to `workspace`, `workspace2`, ... A bare `slack.com` is kept too.
const KEEP: [&str; 7] = [
    "app",
    "api",
    "edgeapi",
    "wss-primary",
    "wss-backup",
    "wss-mobile-primary",
    "enterprise",
];

/// `slack.com`, with `<workspace>.enterprise.slack.com` as the organisation form.
pub fn suffixes() -> Vec<HostSuffix> {
    vec![
        HostSuffix::new("slack.com")
            .with_keep(&KEEP)
            .with_marker("enterprise"),
    ]
}
