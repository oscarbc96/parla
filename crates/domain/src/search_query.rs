//! Search query parsing, name resolution and local/remote hit merging.
//!
//! Operators: `in:#name`, `in:name`, `in:@person` (a direct message),
//! `from:@name`, `from:me`, `before:`, `after:`, `on:` (`YYYY-MM-DD`),
//! `during:` (`YYYY-MM`) and `has:file|link|reaction|pin`. Date bounds are
//! exclusive, like `before:` and `after:`. Warnings carry the offending token
//! as typed; the UI words them through the catalog.

use std::collections::HashSet;

use jiff::civil::Date;

use crate::ids::{ConversationId, MessageId, UserId};
use crate::model::{ConversationKind, Member};
use crate::search::{Has, MessageHit, SearchQuery};
use crate::state::WorkspaceState;

/// A query split into free text and operators. Never an error.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedQuery {
    pub text: String,
    /// Names after `in:`, without a leading `#`; a leading `@` is kept (DM).
    pub in_names: Vec<String>,
    /// Names after `from:`, without a leading `@`; may be `me`.
    pub from_names: Vec<String>,
    pub before: Option<Date>,
    pub after: Option<Date>,
    pub has: Vec<Has>,
    /// Operator tokens that stayed in `text` because they were unknown or
    /// malformed.
    pub warnings: Vec<String>,
}

/// Splits on whitespace; a double-quoted run stays inside one token.
fn tokens(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    let mut quoted = false;
    for (i, c) in input.char_indices() {
        match c {
            '"' => {
                quoted = !quoted;
                start.get_or_insert(i);
            }
            c if c.is_whitespace() && !quoted => {
                if let Some(s) = start.take() {
                    out.push(&input[s..i]);
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        out.push(&input[s..]);
    }
    out
}

fn unquote(v: &str) -> &str {
    let v = v.strip_prefix('"').unwrap_or(v);
    v.strip_suffix('"').unwrap_or(v)
}

fn parse_has(v: &str) -> Option<Has> {
    Some(match v.to_ascii_lowercase().as_str() {
        "file" => Has::File,
        "link" => Has::Link,
        "reaction" => Has::Reaction,
        "pin" => Has::Pin,
        _ => return None,
    })
}

fn parse_month(v: &str) -> Option<(Date, Date)> {
    let (y, m) = v.split_once('-')?;
    if y.len() != 4 || m.len() != 2 {
        return None;
    }
    let first = Date::new(y.parse().ok()?, m.parse().ok()?, 1).ok()?;
    let next = first.checked_add(jiff::Span::new().months(1)).ok()?;
    Some((first, next))
}

fn parse_date(v: &str) -> Option<Date> {
    if v.len() != 10 {
        return None;
    }
    v.parse().ok()
}

/// Applies one `key:value` token; `false` when malformed.
fn apply(p: &mut ParsedQuery, key: &str, value: &str) -> bool {
    let value = unquote(value);
    if value.is_empty() {
        return false;
    }
    match key {
        "in" => {
            let v = value.strip_prefix('#').unwrap_or(value);
            if v.is_empty() || v == "@" {
                return false;
            }
            p.in_names.push(v.to_string());
        }
        "from" => {
            let v = value.strip_prefix('@').unwrap_or(value);
            if v.is_empty() {
                return false;
            }
            p.from_names.push(v.to_string());
        }
        "before" | "after" | "on" => {
            let Some(d) = parse_date(value) else {
                return false;
            };
            match key {
                "before" => p.before = Some(d),
                "after" => p.after = Some(d),
                _ => {
                    let (Ok(prev), Ok(next)) = (d.yesterday(), d.tomorrow()) else {
                        return false;
                    };
                    p.after = Some(prev);
                    p.before = Some(next);
                }
            }
        }
        "during" => {
            let Some((first, next)) = parse_month(value) else {
                return false;
            };
            let Ok(prev) = first.yesterday() else {
                return false;
            };
            p.after = Some(prev);
            p.before = Some(next);
        }
        "has" => {
            let Some(h) = parse_has(value) else {
                return false;
            };
            p.has.push(h);
        }
        _ => return false,
    }
    true
}

/// Parses a search box string. Never fails; see the module docs.
pub fn parse_query(input: &str) -> ParsedQuery {
    let mut p = ParsedQuery::default();
    let mut text: Vec<&str> = Vec::new();
    for tok in tokens(input) {
        let Some((key, value)) = tok.split_once(':') else {
            text.push(tok);
            continue;
        };
        let known = matches!(
            key.to_ascii_lowercase().as_str(),
            "in" | "from" | "before" | "after" | "on" | "during" | "has"
        );
        if known {
            if !apply(&mut p, &key.to_ascii_lowercase(), value) {
                p.warnings.push(tok.to_string());
                text.push(tok);
            }
            continue;
        }
        // Looks like an operator (a word, then a value) but is not one;
        // URLs, times and emoji codes are plain text without a warning.
        let wordy = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic());
        if wordy && !value.is_empty() && !value.starts_with("//") {
            p.warnings.push(tok.to_string());
        }
        text.push(tok);
    }
    p.text = text.join(" ");
    p
}

fn member_matches(m: &Member, name: &str) -> bool {
    m.display_name.eq_ignore_ascii_case(name)
        || m.real_name.eq_ignore_ascii_case(name)
        || (!m.handle.is_empty() && m.handle.eq_ignore_ascii_case(name))
}

fn users_named(state: &WorkspaceState, name: &str) -> Vec<UserId> {
    if name.eq_ignore_ascii_case("me") {
        return state.self_user.iter().cloned().collect();
    }
    let mut ids: Vec<UserId> = state
        .members
        .values()
        .filter(|m| member_matches(m, name))
        .map(|m| m.id.clone())
        .collect();
    ids.sort();
    ids
}

fn push_unique<T: PartialEq>(v: &mut Vec<T>, items: impl IntoIterator<Item = T>) {
    for i in items {
        if !v.contains(&i) {
            v.push(i);
        }
    }
}

/// Resolves names against the workspace. Names that match nothing are
/// returned as warnings (`in:#name`, `from:@name`) and dropped. Parse
/// warnings stay on `p`.
pub fn resolve_query(p: &ParsedQuery, state: &WorkspaceState) -> (SearchQuery, Vec<String>) {
    let mut q = SearchQuery {
        text: p.text.as_str().into(),
        before: p.before,
        after: p.after,
        has: p.has.clone(),
        ..Default::default()
    };
    let mut warnings = Vec::new();
    for name in &p.in_names {
        let found: Vec<ConversationId> = if let Some(person) = name.strip_prefix('@') {
            let users = users_named(state, person);
            state
                .conversations
                .values()
                .filter(|c| {
                    c.kind == ConversationKind::Direct
                        && c.peer.as_ref().is_some_and(|u| users.contains(u))
                })
                .map(|c| c.id.clone())
                .collect()
        } else {
            state
                .conversations
                .values()
                .filter(|c| c.name.eq_ignore_ascii_case(name))
                .map(|c| c.id.clone())
                .collect()
        };
        if found.is_empty() {
            let shown = if name.starts_with('@') {
                name.clone()
            } else {
                format!("#{name}")
            };
            warnings.push(format!("in:{shown}"));
        }
        push_unique(&mut q.conversations, found);
    }
    for name in &p.from_names {
        let found = users_named(state, name);
        if found.is_empty() {
            let shown = if name.eq_ignore_ascii_case("me") {
                name.clone()
            } else {
                format!("@{name}")
            };
            warnings.push(format!("from:{shown}"));
        }
        push_unique(&mut q.authors, found);
    }
    (q, warnings)
}

/// Local hits first in rank order, then remote hits not already present,
/// newest first.
pub fn merge_hits(local: Vec<MessageHit>, remote: Vec<MessageHit>) -> Vec<MessageHit> {
    let mut seen: HashSet<MessageId> = local.iter().map(|h| h.message.id.clone()).collect();
    let mut extra: Vec<MessageHit> = remote
        .into_iter()
        .filter(|h| seen.insert(h.message.id.clone()))
        .collect();
    extra.sort_by(|a, b| b.message.id.ts.cmp(&a.message.id.ts));
    let mut out = local;
    out.extend(extra);
    out
}

/// Whether the local index can answer the query (`link` and `pin` filters
/// are server-only).
pub fn local_supported(q: &SearchQuery) -> bool {
    !q.has.iter().any(|h| matches!(h, Has::Link | Has::Pin))
}
