//! Pieces of the redacted capture layout shared by the `PARLA_RECORD` recorder
//! and `recon redact`, so a recording reads like any other capture.

use serde_json::{Value, json};

use crate::{Redactor, RuleSet};

/// A file-name-safe version of `label`: ASCII alphanumerics, `.`, `-` and `_`
/// survive, everything else becomes `_`, and it is at most 60 characters.
pub fn sanitize(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    if cleaned.is_empty() {
        "x".to_string()
    } else {
        cleaned
    }
}

/// Header names worth keeping: the common ones here, protocol-specific ones from
/// the rule set. Everything else (cookies, authorization, user agent) is dropped.
pub fn keep_header(name: &str, rules: &dyn RuleSet) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "content-type" | "accept" | "origin" | "referer" | "retry-after"
    ) || rules.keep_header(&name.to_ascii_lowercase())
}

fn looks_json(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with('{') || t.starts_with('[')
}

/// A body as a redacted document: JSON when `mime` says so or it parses as
/// JSON, else prose.
pub fn body_value(r: &mut Redactor, mime: &str, text: &str) -> Value {
    if (mime.contains("json") || looks_json(text))
        && let Ok(mut v) = serde_json::from_str::<Value>(text)
    {
        r.redact_json(&mut v);
        return json!({ "json": v });
    }
    json!({ "text": r.redact_prose(text) })
}

/// The file an HTTP exchange is written to (`0001-items.list.json`).
pub fn http_file(n: usize, label: &str) -> String {
    format!("{n:04}-{label}.json")
}

/// The file a websocket frame is written to (`ws-0002-item_added.json`).
pub fn ws_file(n: usize, kind: &str) -> String {
    format!("ws-{n:04}-{kind}.json")
}

/// The `index.json` entry of an HTTP exchange.
pub fn http_index(file: &str, method: &str, status: i64) -> Value {
    json!({ "file": file, "kind": "http", "method": method, "status": status })
}

/// The `index.json` entry of a websocket frame.
pub fn ws_index(file: &str, kind: &str) -> Value {
    json!({ "file": file, "kind": "ws", "type": kind })
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn sanitize_keeps_safe_characters_only() {
        assert_eq!(sanitize("items.list"), "items.list");
        assert_eq!(sanitize("a/b c"), "a_b_c");
        assert_eq!(sanitize(""), "x");
        assert_eq!(sanitize(&"z".repeat(80)).len(), 60);
    }
}
