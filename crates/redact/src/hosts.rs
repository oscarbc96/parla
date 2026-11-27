//! Workspace-subdomain rewriting, driven by a provider's [`HostSuffix`].

use regex::Regex;

use crate::ids::IdMap;
use crate::{HostSuffix, RuleSet};

#[derive(Debug, PartialEq, Eq)]
pub enum HostClass {
    /// Infrastructure host, safe to keep.
    Keep,
    /// Already rewritten (`workspace`, `workspace2`, ...).
    Synthetic,
    /// A real workspace name.
    Real,
}

/// Split the labels before the suffix into (workspace labels, has marker label).
fn split_marker<'a>(labels: &'a [&'a str], suffix: &HostSuffix) -> (&'a [&'a str], bool) {
    match (labels.split_last(), &suffix.marker) {
        (Some((last, rest)), Some(marker))
            if last.eq_ignore_ascii_case(marker) && !rest.is_empty() =>
        {
            (rest, true)
        }
        _ => (labels, false),
    }
}

pub(crate) fn is_synthetic_label(label: &str) -> bool {
    label
        .strip_prefix("workspace")
        .is_some_and(|rest| rest.bytes().all(|b| b.is_ascii_digit()))
}

/// Classify the labels that precede the suffix (`["acme", "<marker>"]`).
pub fn classify(labels: &[&str], suffix: &HostSuffix) -> HostClass {
    let (core, _) = split_marker(labels, suffix);
    match core {
        [] => HostClass::Keep,
        [one] if suffix.keep.iter().any(|k| k.eq_ignore_ascii_case(one)) => HostClass::Keep,
        [one] if is_synthetic_label(one) => HostClass::Synthetic,
        _ => HostClass::Real,
    }
}

/// Hex pairs that decode to a character a host label cannot contain (control, space,
/// punctuation such as `/ : = @ & ? " ' , ; | # .`, and `%` for nested escapes), never
/// letters, digits or `-`. Only these count as an escape before a host.
const ESCAPES: &str = "[01][0-9a-f]|2[0-9a-ce-f]|3[a-f]|40|5[b-f]|60|7[b-f]";

/// Build the regex that finds hosts under `suffix`.
///
/// The host may directly follow a percent escape (`%2F`, or `%252F` when encoded twice),
/// where there is no word boundary: `pfx` captures that escape so it is never read as
/// part of the first label. Hex that could be label text (`%de`, `%41`) is not an escape.
/// `labels` captures the labels before the suffix.
pub fn host_regex(suffix: &str) -> Regex {
    let pattern = format!(
        r"(?i)(?:(?P<pfx>%(?:25)*(?:{ESCAPES}))|\b)(?P<labels>(?:[a-z0-9][a-z0-9-]*\.)*){}\b",
        regex::escape(suffix)
    );
    Regex::new(&pattern).expect("host pattern compiles")
}

/// Rewrite every real workspace host under `suffix` in `text`.
pub fn rewrite_hosts(text: &str, re: &Regex, suffix: &HostSuffix, map: &mut IdMap) -> String {
    re.replace_all(text, |caps: &regex::Captures<'_>| {
        let pfx = caps.name("pfx").map_or("", |m| m.as_str());
        let prefix = caps["labels"].trim_end_matches('.');
        let labels: Vec<&str> = prefix.split('.').filter(|l| !l.is_empty()).collect();
        if classify(&labels, suffix) != HostClass::Real {
            return caps[0].to_string();
        }
        let (core, marked) = split_marker(&labels, suffix);
        let label = map.host_label(&core.join("."));
        match (&suffix.marker, marked) {
            (Some(marker), true) => format!("{pfx}{label}.{marker}.{}", suffix.suffix),
            _ => format!("{pfx}{label}.{}", suffix.suffix),
        }
    })
    .into_owned()
}

/// Host of a URL authority: no userinfo, no port, lower-case.
pub fn authority_host(authority: &str) -> String {
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None => host.split(':').next().unwrap_or(host),
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// True when a URL authority carries credentials (`user:pass@host`).
pub fn has_userinfo(authority: &str) -> bool {
    authority.contains('@')
}

/// Loopback, private, link-local and unspecified addresses: they name no real machine.
/// A public IP literal can point at the owner's infrastructure, so it is not allowed.
fn is_local_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // unique local fc00::/7 and link-local fe80::/10
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Hosts that may stay in a capture: the explicit placeholder hosts, loopback, and the
/// provider's own suffixes and file hosts (those have their own rules).
pub fn is_allowed_host(host: &str, rules: &dyn RuleSet) -> bool {
    let under = |base: &str| host == base || host.ends_with(&format!(".{base}"));
    if host == "localhost" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return is_local_ip(ip);
    }
    if ["example.com", "example.org", "example.net", "example.test"]
        .iter()
        .any(|b| under(b))
    {
        return true;
    }
    rules.host_suffixes().iter().any(|s| under(&s.suffix)) || rules.file_hosts().is_match(host)
}
