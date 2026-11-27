//! Provider rule sets: what a secret, an ID or a workspace host looks like for
//! one chat provider. The engine in this crate is neutral; each provider crate
//! supplies a [`RuleSet`].

use regex::Regex;
use url::Url;

/// A domain suffix whose workspace subdomains are rewritten to
/// `workspace`, `workspace2`, ...
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSuffix {
    /// Lower-case suffix without a leading dot (for example `chat.example.test`).
    pub suffix: String,
    /// Single labels before the suffix that are infrastructure, not workspace names.
    /// A bare suffix is always kept.
    pub keep: Vec<String>,
    /// Trailing label that marks an organisation-level host (`<ws>.<marker>.<suffix>`).
    pub marker: Option<String>,
}

impl HostSuffix {
    pub fn new(suffix: &str) -> Self {
        Self {
            suffix: suffix.trim_start_matches('.').to_ascii_lowercase(),
            keep: Vec::new(),
            marker: None,
        }
    }

    pub fn with_keep(mut self, keep: &[&str]) -> Self {
        self.keep = keep.iter().map(|k| k.to_ascii_lowercase()).collect();
        self
    }

    pub fn with_marker(mut self, marker: &str) -> Self {
        self.marker = Some(marker.to_ascii_lowercase());
        self
    }
}

/// Everything provider-specific the redactor and the leak scanner need.
pub trait RuleSet: Send + Sync {
    /// Short provider id (`fake`), used in output paths and messages.
    fn id(&self) -> &str;
    /// Credential shapes, replaced by a placeholder and reported as token leaks.
    fn tokens(&self) -> &[Regex];
    /// Identifier shape (matched with word boundaries by the pattern itself).
    /// A match must also contain a digit after its first character to count as real.
    fn ids(&self) -> &Regex;
    /// Host suffixes whose workspace subdomains are rewritten.
    fn host_suffixes(&self) -> &[HostSuffix];
    /// Hosts that serve user files and avatars, optionally with scheme and path.
    fn file_hosts(&self) -> &Regex;
    /// A whole-word mention in message text that must keep its (mapped) shape.
    /// Named groups `keep` (the part that is redacted and kept) and `tail`
    /// (the closing text appended as is).
    fn mention(&self) -> Option<&Regex>;
    /// URLs that carry a workspace slug in the path (`<host>/t/<slug>/login/<code>`),
    /// if the provider has them. Named groups: `pre` (kept), `slug` (rewritten to a
    /// workspace label), optional `mid` (kept) and `secret` (replaced by a placeholder).
    fn path_slugs(&self) -> Option<&Regex> {
        None
    }
    /// Whether a protocol-specific header is kept in redacted captures.
    fn keep_header(&self, name: &str) -> bool;
    /// API method name of a (redacted) request URL, if it is one.
    fn method_name(&self, url: &Url) -> Option<String>;
}

/// Regex alternation for one path slash as it appears in captures: plain, JSON-escaped
/// (`\/`, `\u002F`), HTML entities (`&#47;`, `&#x2F;`, `&sol;`) or percent-encoded any number of times
/// (`%2F`, `%252F`, ...). Use inside a `(?i)` pattern for provider `path_slugs` rules.
pub const PATH_SLASH: &str = r"(?:/|\\/|\\u002f|&#0*47;|&#x0*2f;|&sol;|%(?:25)*2f)";

/// Characters of a one-time login code (`.`, `~` and `%2B` included so no tail survives).
pub const LOGIN_CODE: &str = r"(?:[A-Za-z0-9_.~-]|%(?:25)*2b)+";
