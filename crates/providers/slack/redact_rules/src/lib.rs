//! Redaction rules for the Slack provider: token, ID and host
//! shapes, mentions, the `x-slack-` header keep rule and the `/api/<method>` name.
//! Plug [`Rules`] into `redact::Redactor::new` and `redact::check_text`.

mod hosts;
mod patterns;
mod pii;

use redact::{HostSuffix, RuleSet};
use regex::Regex;
use url::Url;

/// Slack's [`RuleSet`].
pub struct Rules {
    tokens: Vec<Regex>,
    ids: Regex,
    suffixes: Vec<HostSuffix>,
    files: Regex,
    mention: Regex,
    slugs: Regex,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            tokens: patterns::tokens(),
            ids: patterns::ids(),
            suffixes: hosts::suffixes(),
            files: patterns::file_hosts(),
            mention: pii::mention(),
            slugs: patterns::path_slugs(),
        }
    }
}

impl RuleSet for Rules {
    fn id(&self) -> &str {
        "slack"
    }

    fn tokens(&self) -> &[Regex] {
        &self.tokens
    }

    fn ids(&self) -> &Regex {
        &self.ids
    }

    fn host_suffixes(&self) -> &[HostSuffix] {
        &self.suffixes
    }

    fn file_hosts(&self) -> &Regex {
        &self.files
    }

    fn mention(&self) -> Option<&Regex> {
        Some(&self.mention)
    }

    fn path_slugs(&self) -> Option<&Regex> {
        Some(&self.slugs)
    }

    fn keep_header(&self, name: &str) -> bool {
        name.to_ascii_lowercase().starts_with("x-slack-")
    }

    fn method_name(&self, url: &Url) -> Option<String> {
        let (_, method) = url.path().rsplit_once("/api/")?;
        (!method.is_empty()).then(|| method.to_string())
    }
}
