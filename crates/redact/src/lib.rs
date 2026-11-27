//! Provider-neutral redaction engine shared by recon, auth and logs.
//!
//! This crate is the one place that knows how to remove secrets. What a secret, an
//! ID or a workspace host looks like for a given provider comes from that provider's
//! [`RuleSet`]. It never prints or logs the values it removes.

mod check;
pub mod doc;
mod fake;
mod hosts;
mod ids;
mod patterns;
mod pii;
pub mod record;
mod rules;

use std::sync::Arc;

use regex::Regex;

pub use check::{Leak, LeakKind, check_text};
pub use fake::FakeRules;
pub use ids::{IdMap, create_private_dir, write_private};
pub use rules::{HostSuffix, LOGIN_CODE, PATH_SLASH, RuleSet};

/// Redacts secrets, IDs, workspace hosts and PII, keeping stable synthetic values
/// through its [`IdMap`].
pub struct Redactor {
    map: IdMap,
    rules: Arc<dyn RuleSet>,
    /// `(regex, suffix)` for each host suffix whose workspace subdomains are rewritten.
    host_suffixes: Vec<(Regex, HostSuffix)>,
}

impl Redactor {
    pub fn new(map: IdMap, rules: Arc<dyn RuleSet>) -> Self {
        let host_suffixes = rules
            .host_suffixes()
            .iter()
            .map(|s| (hosts::host_regex(&s.suffix), s.clone()))
            .collect();
        Self {
            map,
            rules,
            host_suffixes,
        }
    }

    /// Also rewrite workspace subdomains of `suffix` (for example a custom domain).
    /// It keeps the same infrastructure labels and marker as the provider's first suffix.
    pub fn with_host_suffix(mut self, suffix: &str) -> Self {
        let mut extra = HostSuffix::new(suffix);
        if extra.suffix.is_empty()
            || self
                .host_suffixes
                .iter()
                .any(|(_, s)| s.suffix == extra.suffix)
        {
            return self;
        }
        if let Some((_, first)) = self.host_suffixes.first() {
            extra.keep = first.keep.clone();
            extra.marker = first.marker.clone();
        }
        self.host_suffixes
            .push((hosts::host_regex(&extra.suffix), extra));
        self
    }

    pub fn map(&self) -> &IdMap {
        &self.map
    }

    pub fn into_map(self) -> IdMap {
        self.map
    }

    pub fn rules(&self) -> &dyn RuleSet {
        &*self.rules
    }

    /// The shared rule set, for callers that keep using it while mutating the redactor.
    pub fn rules_arc(&self) -> &Arc<dyn RuleSet> {
        &self.rules
    }

    /// Redact free text: tokens, file URLs, emails, workspace hosts, other URLs, then IDs.
    pub fn redact_str(&mut self, text: &str) -> String {
        let mut out = text.to_string();
        for token in self.rules.tokens() {
            out = token
                .replace_all(&out, patterns::SECRET_PLACEHOLDER)
                .into_owned();
        }
        out = patterns::BEARER
            .replace_all(&out, format!("Bearer {}", patterns::SECRET_PLACEHOLDER))
            .into_owned();
        let rules = Arc::clone(&self.rules);
        out = rules
            .file_hosts()
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                let n = self.map.fake_number("fileurl", &caps[0]);
                format!("https://files.example.test/file/{n}")
            })
            .into_owned();
        out = patterns::EMAIL
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                if patterns::is_fake_email(&caps[0]) {
                    return caps[0].to_string();
                }
                let n = self.map.fake_number("email", &caps[0]);
                format!("user{n}@example.test")
            })
            .into_owned();
        for (re, suffix) in &self.host_suffixes {
            out = hosts::rewrite_hosts(&out, re, suffix, &mut self.map);
        }
        if let Some(re) = rules.path_slugs() {
            out = re
                .replace_all(&out, |caps: &regex::Captures<'_>| {
                    let part = |n: &str| caps.name(n).map_or("", |m| m.as_str());
                    let slug = if hosts::is_synthetic_label(part("slug")) {
                        part("slug").to_string()
                    } else {
                        self.map.host_label(part("slug"))
                    };
                    let secret = match part("secret") {
                        "" => "",
                        _ => patterns::SECRET_PLACEHOLDER,
                    };
                    format!("{}{slug}{}{secret}", part("pre"), part("mid"))
                })
                .into_owned();
        }
        out = patterns::URL
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                if !hosts::has_userinfo(&caps[1])
                    && hosts::is_allowed_host(&hosts::authority_host(&caps[1]), &*rules)
                {
                    return self.redact_allowed_url_query(&caps[0]);
                }
                let n = self.map.fake_number("url", &caps[0]);
                format!("https://example.com/{n}")
            })
            .into_owned();
        rules
            .ids()
            .replace_all(&out, |caps: &regex::Captures<'_>| {
                if patterns::is_real_id(&caps[0]) {
                    self.map.id(&caps[0])
                } else {
                    caps[0].to_string()
                }
            })
            .into_owned()
    }
}
