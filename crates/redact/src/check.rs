//! Leak scanner: finds secrets, real IDs, emails, file URLs and real workspace hosts.
//! Findings carry a kind and a position only, never the matched value.

use std::fmt;

use crate::RuleSet;
use crate::hosts::{self, HostClass};
use crate::patterns::{self, BEARER, EMAIL};
use crate::pii;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeakKind {
    Token,
    Email,
    RealId,
    FileUrl,
    WorkspaceHost,
    /// A URL whose host is not a placeholder or provider host.
    ForeignUrl,
    /// A long free-text field that is not placeholder text.
    FreeText,
}

impl fmt::Display for LeakKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LeakKind::Token => "token",
            LeakKind::Email => "email",
            LeakKind::RealId => "real-id",
            LeakKind::FileUrl => "file-url",
            LeakKind::WorkspaceHost => "workspace-host",
            LeakKind::ForeignUrl => "foreign-url",
            LeakKind::FreeText => "free-text",
        })
    }
}

/// One finding. `line` and `col` are 1-based; `col` counts bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leak {
    pub kind: LeakKind,
    pub line: usize,
    pub col: usize,
}

impl fmt::Display for Leak {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{} {}", self.line, self.col, self.kind)
    }
}

/// Scan `text` against `rules` and return every leak found, in line order.
pub fn check_text(text: &str, rules: &dyn RuleSet) -> Vec<Leak> {
    let host_res: Vec<_> = rules
        .host_suffixes()
        .iter()
        .map(|s| hosts::host_regex(&s.suffix))
        .collect();
    let mut leaks = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let mut push = |kind, start: usize| {
            leaks.push(Leak {
                kind,
                line: line_no,
                col: start + 1,
            })
        };
        for token in rules.tokens() {
            for m in token.find_iter(line) {
                push(LeakKind::Token, m.start());
            }
        }
        for m in BEARER.find_iter(line) {
            push(LeakKind::Token, m.start());
        }
        for m in EMAIL.find_iter(line) {
            if !patterns::is_fake_email(m.as_str()) {
                push(LeakKind::Email, m.start());
            }
        }
        for m in rules.file_hosts().find_iter(line) {
            push(LeakKind::FileUrl, m.start());
        }
        for m in rules.ids().find_iter(line) {
            if patterns::is_real_id(m.as_str()) {
                push(LeakKind::RealId, m.start());
            }
        }
        for caps in patterns::URL.captures_iter(line) {
            let host = hosts::authority_host(&caps[1]);
            let start = caps.get(0).map_or(0, |m| m.start());
            // File hosts and workspace hosts are reported above.
            if (hosts::has_userinfo(&caps[1]) || !hosts::is_allowed_host(&host, rules))
                && !rules.file_hosts().is_match(&caps[0])
            {
                push(LeakKind::ForeignUrl, start);
            } else if let Some((_, rest)) = caps[0].split_once('?') {
                let query = rest.split('#').next().unwrap_or(rest);
                let secret = url::form_urlencoded::parse(query.as_bytes()).any(|(k, v)| {
                    pii::is_secret_key(&k) && !v.is_empty() && v != patterns::SECRET_PLACEHOLDER
                });
                if secret {
                    push(LeakKind::Token, start);
                }
            }
        }
        for caps in patterns::TEXT_FIELD.captures_iter(line) {
            let (key, value) = (&caps[1], &caps[2]);
            if pii::is_magic_login_key(key) && value != patterns::SECRET_PLACEHOLDER {
                push(LeakKind::Token, caps.get(2).map_or(0, |m| m.start()));
            }
            let min_len = if pii::is_strict_text_key(key) { 1 } else { 20 };
            let text = pii::is_text_key(key) && !pii::is_placeholder(value, rules, min_len);
            if text || pii::is_encoded_blob(key, value) {
                push(LeakKind::FreeText, caps.get(2).map_or(0, |m| m.start()));
            }
        }
        if let Some(re) = rules.path_slugs() {
            for caps in re.captures_iter(line) {
                let slug = caps.name("slug").map_or("", |m| m.as_str());
                if !hosts::is_synthetic_label(slug) {
                    push(
                        LeakKind::WorkspaceHost,
                        caps.name("slug").map_or(0, |m| m.start()),
                    );
                }
                if let Some(m) = caps.name("secret")
                    && m.as_str() != patterns::SECRET_PLACEHOLDER
                {
                    push(LeakKind::Token, m.start());
                }
            }
        }
        for (re, suffix) in host_res.iter().zip(rules.host_suffixes()) {
            for caps in re.captures_iter(line) {
                let prefix = caps["labels"].trim_end_matches('.');
                let labels: Vec<&str> = prefix.split('.').filter(|l| !l.is_empty()).collect();
                // A bare file host is already reported as a file URL.
                if labels.len() == 1 && rules.file_hosts().is_match(&caps[0]) {
                    continue;
                }
                if hosts::classify(&labels, suffix) == HostClass::Real {
                    push(
                        LeakKind::WorkspaceHost,
                        caps.get(0).map_or(0, |m| m.start()),
                    );
                }
            }
        }
    }
    leaks
}
