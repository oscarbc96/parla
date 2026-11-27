//! Navigation and media-permission policy for the call window.
//!
//! The window loads only `https` pages on an allowed host; every other web
//! link goes to the default browser; every other scheme is blocked.

use std::sync::Arc;

use url::Url;

/// What to do with a navigation request from the call window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavDecision {
    /// Load it in the call window.
    Allow,
    /// Hand it to the default browser (or mail client).
    OpenExternally,
    /// Drop it.
    Block,
}

/// True when `host` equals a suffix or ends with `.<suffix>` (ASCII case
/// insensitive). A trailing dot, an empty label or an empty suffix never match.
pub fn host_allowed(host: &str, suffixes: &[Arc<str>]) -> bool {
    if host.is_empty() || host.ends_with('.') || host.starts_with('.') || host.contains("..") {
        return false;
    }
    suffixes.iter().any(|s| {
        let s: &str = s;
        if s.is_empty() || s.starts_with('.') || s.ends_with('.') {
            return false;
        }
        if host.eq_ignore_ascii_case(s) {
            return true;
        }
        host.len() > s.len() + 1
            && host.is_char_boundary(host.len() - s.len())
            && host.as_bytes()[host.len() - s.len() - 1] == b'.'
            && host[host.len() - s.len()..].eq_ignore_ascii_case(s)
    })
}

fn on_allowed_https(url: &Url, suffixes: &[Arc<str>]) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|h| host_allowed(h, suffixes))
}

/// Decides one navigation. `about:blank` loads; `https` on an allowed host
/// loads; other `https`, `http` and `mailto` open externally; the rest is
/// blocked (`javascript:`, `data:`, `file:`, `blob:`, ...).
pub fn decide(url: &Url, suffixes: &[Arc<str>]) -> NavDecision {
    match url.scheme() {
        "about" if url.as_str() == "about:blank" => NavDecision::Allow,
        "https" if on_allowed_https(url, suffixes) => NavDecision::Allow,
        "https" | "http" | "mailto" => NavDecision::OpenExternally,
        _ => NavDecision::Block,
    }
}

/// True when `url` is on an allowed host and its path is, or is under, one
/// of the signed-out paths (a redirect to sign in means the session failed).
pub fn is_signed_out(url: &Url, suffixes: &[Arc<str>], paths: &[Arc<str>]) -> bool {
    if !on_allowed_https(url, suffixes) {
        return false;
    }
    let path = url.path();
    paths.iter().any(|p| {
        let p = p.trim_end_matches('/');
        !p.is_empty()
            && path
                .strip_prefix(p)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// Microphone and camera are granted only to an allowed `https` origin.
pub fn permit_media(origin: &Url, suffixes: &[Arc<str>]) -> bool {
    on_allowed_https(origin, suffixes)
}
