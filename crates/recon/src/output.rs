//! Turn a HAR into redacted per-exchange and per-frame files.
//!
//! Output is built from selected fields, never by copying the raw entry, and
//! every file is scanned with the leak checker before anything is written.

use std::io;
use std::path::Path;

use redact::{Leak, Redactor, RuleSet, check_text, doc};
use serde_json::{Value, json};
use url::Url;

use crate::har::{Entry, Har, NameValue, PostData, WsMessage};

pub struct OutFile {
    pub name: String,
    pub text: String,
}

#[derive(Default)]
pub struct Built {
    pub files: Vec<OutFile>,
    pub http: usize,
    pub ws: usize,
    /// Leaks found in the redacted output (file name, finding). Must be empty.
    pub leaks: Vec<(String, Leak)>,
}

fn headers(r: &mut Redactor, list: &[NameValue]) -> Value {
    let rules = std::sync::Arc::clone(r.rules_arc());
    let rules: &dyn RuleSet = &*rules;
    let kept: Vec<Value> = list
        .iter()
        .filter(|h| doc::keep_header(&h.name, rules))
        .map(|h| {
            let value = h.value.as_deref().unwrap_or("");
            json!([h.name.to_ascii_lowercase(), r.redact_url(value)])
        })
        .collect();
    Value::Array(kept)
}

fn pairs(r: &mut Redactor, list: &[NameValue]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = list
        .iter()
        .map(|p| (p.name.clone(), p.value.clone().unwrap_or_default()))
        .collect();
    r.redact_form(&mut out);
    out
}

fn pairs_json(list: Vec<(String, String)>) -> Value {
    Value::Array(list.into_iter().map(|(k, v)| json!([k, v])).collect())
}

/// Parse a multipart body into fields; file parts become a placeholder.
fn multipart_fields(mime: &str, text: &str) -> Vec<(String, String)> {
    let Some(boundary) = mime
        .split(';')
        .filter_map(|p| p.trim().strip_prefix("boundary="))
        .next()
        .map(|b| b.trim_matches('"').to_string())
    else {
        return Vec::new();
    };
    let delimiter = format!("--{boundary}");
    let mut out = Vec::new();
    for part in text.split(&delimiter) {
        let part = part.trim_start_matches("\r\n").trim_start_matches('\n');
        let Some((head, body)) = part
            .split_once("\r\n\r\n")
            .or_else(|| part.split_once("\n\n"))
        else {
            continue;
        };
        let disposition = head
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("content-disposition"))
            .unwrap_or("");
        let Some(name) = field_attr(disposition, "name") else {
            continue;
        };
        let value = if field_attr(disposition, "filename").is_some() {
            "(file omitted)".to_string()
        } else {
            body.trim_end_matches("\r\n")
                .trim_end_matches('\n')
                .to_string()
        };
        out.push((name, value));
    }
    out
}

fn field_attr(disposition: &str, attr: &str) -> Option<String> {
    let needle = format!("; {attr}=\"");
    let start = disposition.find(&needle)? + needle.len();
    let end = disposition[start..].find('"')?;
    Some(disposition[start..start + end].to_string())
}

fn request_body(r: &mut Redactor, post: &PostData) -> Option<Value> {
    let text = post.text.as_deref().unwrap_or("");
    let mime = post.mime_type.to_ascii_lowercase();
    if mime.contains("multipart") {
        let fields = if text.is_empty() {
            post.params
                .iter()
                .map(|p| (p.name.clone(), p.value.clone().unwrap_or_default()))
                .collect()
        } else {
            multipart_fields(&post.mime_type, text)
        };
        let mut fields = fields;
        r.redact_form(&mut fields);
        return Some(json!({ "form": pairs_json(fields) }));
    }
    if mime.contains("x-www-form-urlencoded") {
        let mut fields: Vec<(String, String)> = if post.params.is_empty() {
            url::form_urlencoded::parse(text.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect()
        } else {
            post.params
                .iter()
                .map(|p| (p.name.clone(), p.value.clone().unwrap_or_default()))
                .collect()
        };
        r.redact_form(&mut fields);
        return Some(json!({ "form": pairs_json(fields) }));
    }
    if text.is_empty() {
        return None;
    }
    Some(doc::body_value(r, &mime, text))
}

fn response_body(r: &mut Redactor, entry: &Entry) -> Option<Value> {
    let content = &entry.response.content;
    let text = content.text.as_deref()?;
    let mime = content.mime_type.to_ascii_lowercase();
    let binary = content.encoding.as_deref() == Some("base64")
        || [
            "image/",
            "audio/",
            "video/",
            "font/",
            "application/octet-stream",
        ]
        .iter()
        .any(|p| mime.starts_with(p));
    if binary {
        return Some(json!({ "omitted": mime }));
    }
    Some(doc::body_value(r, &mime, text))
}

/// API method (for example `conversations.history`) from a redacted URL, as the
/// rule set names it, else the HTTP verb.
fn exchange_label(redacted_url: &str, verb: &str, rules: &dyn RuleSet) -> String {
    match Url::parse(redacted_url)
        .ok()
        .and_then(|url| rules.method_name(&url))
    {
        Some(method) => doc::sanitize(&method),
        None => doc::sanitize(&verb.to_ascii_lowercase()),
    }
}

/// The method or event a capture file is named after: `0001-items.list.json`
/// and `ws-0002-item_added.json` give `items.list` and `item_added`.
pub fn label_of(name: &str) -> &str {
    let rest = name.strip_prefix("ws-").unwrap_or(name);
    rest.split_once('-')
        .map_or(rest, |(_, label)| label)
        .trim_end_matches(".json")
}

fn is_websocket(entry: &Entry) -> bool {
    !entry.web_socket_messages.is_empty() || entry.resource_type.as_deref() == Some("websocket")
}

fn frame(r: &mut Redactor, url: &str, msg: &WsMessage) -> (String, Value) {
    let direction = doc::sanitize(&msg.direction.to_ascii_lowercase());
    // Opcode 2 is a binary frame; its payload is not text.
    let (kind, body) = if msg.opcode == 2 {
        (direction.clone(), json!({ "omitted": "binary" }))
    } else if let Ok(mut v) = serde_json::from_str::<Value>(&msg.data) {
        r.redact_json(&mut v);
        let kind = v
            .get("type")
            .and_then(Value::as_str)
            .map(doc::sanitize)
            .unwrap_or_else(|| direction.clone());
        (kind, json!({ "json": v }))
    } else {
        (
            direction.clone(),
            json!({ "text": r.redact_prose(&msg.data) }),
        )
    };
    let doc = json!({
        "direction": direction,
        "opcode": msg.opcode,
        "url": url,
        "data": body,
    });
    (kind, doc)
}

fn push(built: &mut Built, name: String, doc: &Value, rules: &dyn RuleSet) {
    let text = serde_json::to_string_pretty(doc).expect("json serialises") + "\n";
    for leak in check_text(&text, rules) {
        built.leaks.push((name.clone(), leak));
    }
    built.files.push(OutFile { name, text });
}

/// Redact every exchange and websocket frame of `har`.
pub fn build(har: &Har, r: &mut Redactor) -> Built {
    let rules = std::sync::Arc::clone(r.rules_arc());
    let rules: &dyn RuleSet = &*rules;
    let mut built = Built::default();
    let mut index = Vec::new();
    for entry in &har.log.entries {
        let url = r.redact_url(&entry.request.url);
        if is_websocket(entry) {
            for msg in &entry.web_socket_messages {
                built.ws += 1;
                let (kind, frame_doc) = frame(r, &url, msg);
                let name = doc::ws_file(built.ws, &kind);
                index.push(doc::ws_index(&name, &kind));
                push(&mut built, name, &frame_doc, rules);
            }
            continue;
        }
        built.http += 1;
        let label = exchange_label(&url, &entry.request.method, rules);
        let name = doc::http_file(built.http, &label);
        let mut request = json!({
            "headers": headers(r, &entry.request.headers),
            "query": pairs_json(pairs(r, &entry.request.query_string)),
        });
        if let Some(post) = &entry.request.post_data
            && let Some(body) = request_body(r, post)
        {
            request["body"] = body;
        }
        let mut response = json!({
            "status": entry.response.status,
            "mime": entry.response.content.mime_type.to_ascii_lowercase(),
            "headers": headers(r, &entry.response.headers),
        });
        if let Some(body) = response_body(r, entry) {
            response["body"] = body;
        }
        let doc = json!({
            "method": entry.request.method.to_ascii_uppercase(),
            "url": url,
            "request": request,
            "response": response,
        });
        index.push(doc::http_index(&name, &label, entry.response.status));
        push(&mut built, name, &doc, rules);
    }
    push(
        &mut built,
        "index.json".to_string(),
        &Value::Array(index),
        rules,
    );
    built
}

/// Write the built files into `dir` (created owner-only if missing).
pub fn write(dir: &Path, built: &Built) -> io::Result<()> {
    redact::create_private_dir(dir)?;
    for file in &built.files {
        redact::write_private(&dir.join(&file.name), file.text.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::label_of;

    #[test]
    fn label_of_strips_the_number_and_kind() {
        assert_eq!(label_of("0001-items.list.json"), "items.list");
        assert_eq!(label_of("0001-ping.json"), "ping");
        assert_eq!(label_of("ws-0002-item_added.json"), "item_added");
        assert_eq!(label_of("ws-weird"), "weird");
    }
}
