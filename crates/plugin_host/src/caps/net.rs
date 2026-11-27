//! `network` hosts: https only, exact names.

use url::{Host, Url};

use crate::manifest::NetworkHost;

impl NetworkHost {
    /// Whether `host` is this host: exact, ignoring ASCII case and one
    /// trailing dot. No suffix or wildcard match.
    pub fn matches(&self, host: &str) -> bool {
        let host = host.strip_suffix('.').unwrap_or(host);
        host.eq_ignore_ascii_case(self.as_str())
    }
}

/// Why a URL is not one `http` may fetch.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum BadUrl {
    NotHttps,
    /// A port other than 443, user info, or an IP address.
    Invalid,
}

/// The host of `url` when it is an https link to a named host on the
/// default port with no user info. `Url` drops `:443` itself.
pub(super) fn https_host(url: &Url) -> Result<&str, BadUrl> {
    if url.scheme() != "https" {
        return Err(BadUrl::NotHttps);
    }
    if url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err(BadUrl::Invalid);
    }
    match url.host() {
        Some(Host::Domain(h)) => Ok(h),
        _ => Err(BadUrl::Invalid),
    }
}
