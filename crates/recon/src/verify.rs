//! `recon verify`: read-only drift check of catalogue methods against committed fixtures.
//! Responses are redacted in memory and reduced to shapes; nothing is written to disk and
//! only method names, JSON pointers and counts are printed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use redact::{IdMap, Redactor};
use serde_json::Value;

use crate::catalogue::{self, Entry};
use crate::shape::{Shape, infer, label};

/// Added, removed and retyped JSON pointers between a fixture shape and a live shape.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShapeDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// (pointer, old, new)
    pub retyped: Vec<(String, String, String)>,
}

/// True only when `method` is in the provider's explicit allowlist and the catalogue says yes.
pub fn is_read_only(allowlist: &[&str], entry: &Entry) -> bool {
    entry.read_only && allowlist.contains(&entry.method.as_str())
}

pub fn compare(fixture: &Shape, live: &Shape) -> ShapeDiff {
    let mut d = ShapeDiff::default();
    walk("", fixture, live, &mut d);
    d
}

fn is_empty_mixed(s: &Shape) -> bool {
    matches!(s, Shape::Mixed(v) if v.is_empty())
}

fn walk(path: &str, old: &Shape, new: &Shape, d: &mut ShapeDiff) {
    match (old, new) {
        // Nullable fields flip often; null on either side is not drift.
        (Shape::Null, _) | (_, Shape::Null) => {}
        (a, b) if is_empty_mixed(a) || is_empty_mixed(b) => {}
        (Shape::Object(a), Shape::Object(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for k in keys {
                let p = format!("{path}/{k}");
                match (a.get(k), b.get(k)) {
                    (Some(x), Some(y)) => walk(&p, &x.shape, &y.shape, d),
                    (None, Some(_)) => d.added.push(p),
                    (Some(x), None) if !x.optional => d.removed.push(p),
                    _ => {}
                }
            }
        }
        (Shape::Array(a), Shape::Array(b)) => walk(&format!("{path}/[]"), a, b, d),
        (a, b) => {
            let (la, lb) = (label(a), label(b));
            if la != lb {
                d.retyped.push((path.to_string(), la, lb));
            }
        }
    }
}

/// Request form for a fixture: the token placeholder is replaced and synthetic IDs are mapped
/// back to the owner's real ones. Values are `Zeroizing` so the token and real IDs are wiped
/// after the request; the caller borrows them for `reqwest::RequestBuilder::form`.
pub fn request_form(
    fixture: &Value,
    token: &str,
    map: &IdMap,
) -> Vec<(String, zeroize::Zeroizing<String>)> {
    let pairs = fixture
        .pointer("/request/body/form")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    pairs
        .iter()
        .filter_map(|p| {
            let k = p.get(0)?.as_str()?;
            let v = p.get(1)?.as_str()?;
            let v = if k == "token" {
                token.to_owned()
            } else {
                map.real_for(v).unwrap_or(v).to_string()
            };
            Some((k.to_string(), zeroize::Zeroizing::new(v)))
        })
        .collect()
}

pub struct Options {
    pub catalogue: PathBuf,
    /// Repository root the catalogue's fixture paths are relative to.
    pub root: PathBuf,
    pub base_url: String,
    pub per_minute: u32,
    pub allowlist: Vec<String>,
    pub token: zeroize::Zeroizing<String>,
    pub cookie: zeroize::Zeroizing<String>,
    pub map: IdMap,
    pub rules: std::sync::Arc<dyn redact::RuleSet>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub lines: Vec<String>,
    pub verified: usize,
    pub refused: usize,
    pub skipped: usize,
    pub notes: usize,
    pub failures: usize,
}

impl Report {
    pub fn failed(&self) -> bool {
        self.failures > 0 || self.refused > 0
    }
}

fn fixture_shape(path: &Path) -> Result<(Value, Shape), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read fixture: {e}"))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| format!("parse fixture: {e}"))?;
    let body = doc
        .pointer("/response/body/json")
        .ok_or("fixture has no json response")?
        .clone();
    Ok((doc, infer(&[body])))
}

/// The owner's token and cookie are sent to `base_url`: it must be `https`
/// (plain `http` only to this machine, for tests).
fn check_base_url(base_url: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(base_url).map_err(|_| "--base-url is not a URL".to_string())?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    match url.scheme() {
        "https" => Ok(()),
        "http" if local => Ok(()),
        _ => Err("--base-url must be https (http only for localhost)".to_string()),
    }
}

pub async fn run(opts: Options) -> Result<Report, String> {
    check_base_url(&opts.base_url)?;
    let text =
        std::fs::read_to_string(&opts.catalogue).map_err(|e| format!("read catalogue: {e}"))?;
    let entries = catalogue::parse(&text);
    let allow: Vec<&str> = opts.allowlist.iter().map(String::as_str).collect();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "cannot build http client".to_string())?;
    let origin = opts.base_url.trim_end_matches('/').to_string();
    let gap = Duration::from_millis(60_000 / u64::from(opts.per_minute.max(1)));
    let mut report = Report::default();
    let mut first = true;
    for entry in &entries {
        if !is_read_only(&allow, entry) {
            report.refused += 1;
            report
                .lines
                .push(format!("refused {} (not marked read-only)", entry.method));
            continue;
        }
        let Some(rel) = &entry.fixture else {
            report.skipped += 1;
            report
                .lines
                .push(format!("skipped {} (no fixture)", entry.method));
            continue;
        };
        let (doc, old) = match fixture_shape(&opts.root.join(rel)) {
            Ok(v) => v,
            Err(e) => {
                report.failures += 1;
                report.lines.push(format!("FAIL {}: {e}", entry.method));
                continue;
            }
        };
        if !first {
            tokio::time::sleep(gap).await;
        }
        first = false;
        let form = request_form(&doc, &opts.token, &opts.map);
        let fields: Vec<(&str, &str)> =
            form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let resp = client
            .post(format!("{origin}/api/{}", entry.method))
            .header("cookie", opts.cookie.as_str())
            .header("origin", &origin)
            .form(&fields)
            .send()
            .await;
        let body = match resp {
            Ok(r) if r.status().is_success() => r.text().await.ok(),
            Ok(r) => {
                report.failures += 1;
                report.lines.push(format!(
                    "FAIL {} http status {}",
                    entry.method,
                    r.status().as_u16()
                ));
                continue;
            }
            Err(_) => {
                report.failures += 1;
                report
                    .lines
                    .push(format!("FAIL {} request failed", entry.method));
                continue;
            }
        };
        // Redact in memory before anything else looks at the response.
        let mut redactor = Redactor::new(opts.map.clone(), opts.rules.clone());
        let live = body
            .map(|t| redactor.redact_str(&t))
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let Some(live) = live else {
            report.failures += 1;
            report
                .lines
                .push(format!("FAIL {} response is not json", entry.method));
            continue;
        };
        if live.get("ok") == Some(&Value::Bool(false)) {
            report.failures += 1;
            report.lines.push(format!("FAIL {} ok=false", entry.method));
            continue;
        }
        let diff = compare(&old, &infer(&[live]));
        report.verified += 1;
        for p in &diff.added {
            report.notes += 1;
            report
                .lines
                .push(format!("note {} added {p}", entry.method));
        }
        for p in &diff.removed {
            report.failures += 1;
            report
                .lines
                .push(format!("FAIL {} removed {p}", entry.method));
        }
        for (p, a, b) in &diff.retyped {
            report.failures += 1;
            report
                .lines
                .push(format!("FAIL {} retyped {p} {a} -> {b}", entry.method));
        }
    }
    Ok(report)
}
