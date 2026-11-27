//! `recon summary` and `recon check` over an output directory.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use redact::{Leak, RuleSet, check_text};
use serde_json::Value;

/// All regular files under `paths` (files are taken as given, directories walked).
pub fn walk(paths: &[PathBuf]) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path)? {
                stack.push(entry?.path());
            }
        } else {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Scan files for leaks. Returns the number of files scanned and the findings.
pub fn check_paths(
    paths: &[PathBuf],
    rules: &dyn RuleSet,
) -> io::Result<(usize, Vec<(PathBuf, Leak)>)> {
    let files = walk(paths)?;
    let mut leaks = Vec::new();
    for file in &files {
        let bytes = std::fs::read(file)?;
        let text = String::from_utf8_lossy(&bytes);
        for leak in check_text(&text, rules) {
            leaks.push((file.clone(), leak));
        }
    }
    Ok((files.len(), leaks))
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[derive(Default)]
struct MethodStats {
    count: usize,
    statuses: BTreeMap<i64, usize>,
    keys: BTreeMap<String, usize>,
}

/// Methods, events and response keys found in a redacted directory.
pub fn summarize(dir: &Path) -> io::Result<String> {
    let mut http: BTreeMap<String, MethodStats> = BTreeMap::new();
    let mut ws: BTreeMap<String, usize> = BTreeMap::new();
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json") && n != "index.json")
        .collect();
    names.sort();
    for name in names {
        let Some(doc) = read_json(&dir.join(&name)) else {
            continue;
        };
        let label = crate::output::label_of(&name);
        if name.starts_with("ws-") {
            *ws.entry(label.to_string()).or_default() += 1;
            continue;
        }
        let stats = http.entry(label.to_string()).or_default();
        stats.count += 1;
        let status = doc["response"]["status"].as_i64().unwrap_or(0);
        *stats.statuses.entry(status).or_default() += 1;
        if let Some(obj) = doc["response"]["body"]["json"].as_object() {
            for key in obj.keys() {
                *stats.keys.entry(key.clone()).or_default() += 1;
            }
        }
    }
    let mut out = String::new();
    out.push_str(&format!("http methods: {}\n", http.len()));
    for (label, s) in &http {
        let statuses: Vec<String> = s.statuses.iter().map(|(c, n)| format!("{c}x{n}")).collect();
        let keys: Vec<&str> = s.keys.keys().map(String::as_str).collect();
        out.push_str(&format!(
            "  {label}  calls={}  status={}  response keys: {}\n",
            s.count,
            statuses.join(","),
            if keys.is_empty() {
                "-".to_string()
            } else {
                keys.join(", ")
            }
        ));
    }
    out.push_str(&format!("websocket event types: {}\n", ws.len()));
    for (kind, n) in &ws {
        out.push_str(&format!("  {kind}  frames={n}\n"));
    }
    Ok(out)
}
