//! `recon extract`: group a redacted directory by method or event, infer shapes,
//! draft the docs section and copy fixtures.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use redact::{Leak, RuleSet, check_text};
use serde_json::Value;

use crate::output::label_of;
use crate::shape::{ExchangeKind, Shape, infer, render_markdown};

/// Request parameters that are credentials, never documented.
const EXCLUDED_PARAMS: [&str; 1] = ["token"];

/// All exchanges of one method (HTTP) or event type (websocket).
#[derive(Debug)]
pub struct Group {
    pub name: String,
    pub kind: ExchangeKind,
    pub params: BTreeSet<String>,
    pub shape: Shape,
    pub files: Vec<PathBuf>,
}

#[derive(Default)]
struct Acc {
    params: BTreeSet<String>,
    samples: Vec<Value>,
    files: Vec<PathBuf>,
}

fn add_pairs(params: &mut BTreeSet<String>, pairs: Option<&Value>) {
    for pair in pairs.and_then(Value::as_array).into_iter().flatten() {
        if let Some(name) = pair.get(0).and_then(Value::as_str)
            && !EXCLUDED_PARAMS.contains(&name)
        {
            params.insert(name.to_string());
        }
    }
}

/// Group the exchanges and frames in `dir`. HTTP groups come first, then websocket, by name.
pub fn extract(dir: &Path) -> io::Result<Vec<Group>> {
    let mut http: BTreeMap<String, Acc> = BTreeMap::new();
    let mut ws: BTreeMap<String, Acc> = BTreeMap::new();
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json") && n != "index.json")
        .collect();
    names.sort();
    for name in names {
        let path = dir.join(&name);
        let Ok(doc) = serde_json::from_slice::<Value>(&std::fs::read(&path)?) else {
            continue;
        };
        if name.starts_with("ws-") {
            let acc = ws.entry(label_of(&name).to_string()).or_default();
            if let Some(data) = doc["data"].get("json") {
                acc.samples.push(data.clone());
            }
            acc.files.push(path);
            continue;
        }
        let acc = http.entry(label_of(&name).to_string()).or_default();
        add_pairs(&mut acc.params, doc["request"].get("query"));
        add_pairs(&mut acc.params, doc["request"]["body"].get("form"));
        if let Some(body) = doc["request"]["body"]
            .get("json")
            .and_then(Value::as_object)
        {
            acc.params.extend(
                body.keys()
                    .filter(|k| !EXCLUDED_PARAMS.contains(&k.as_str()))
                    .cloned(),
            );
        }
        if let Some(body) = doc["response"]["body"].get("json") {
            acc.samples.push(body.clone());
        }
        acc.files.push(path);
    }
    let build = |m: BTreeMap<String, Acc>, kind| {
        m.into_iter().map(move |(name, a)| Group {
            name,
            kind,
            params: a.params,
            shape: infer(&a.samples),
            files: a.files,
        })
    };
    Ok(build(http, ExchangeKind::Http)
        .chain(build(ws, ExchangeKind::Ws))
        .collect())
}

/// Draft section for the area page, one subsection per group.
pub fn render(provider: &str, area: &str, groups: &[Group]) -> String {
    let mut out = format!("<!-- draft for docs/providers/{provider}/web-client/{area}.md -->\n\n");
    for g in groups {
        out.push_str(&render_markdown(&g.name, g.kind, &g.params, &g.shape));
        out.push('\n');
    }
    out
}

/// Leaks (file, finding) in the group source files. Findings carry positions only.
pub fn leaks_in(groups: &[Group], rules: &dyn RuleSet) -> io::Result<Vec<(PathBuf, Leak)>> {
    let mut out = Vec::new();
    for file in groups.iter().flat_map(|g| &g.files) {
        let text = String::from_utf8_lossy(&std::fs::read(file)?).into_owned();
        out.extend(
            check_text(&text, rules)
                .into_iter()
                .map(|l| (file.clone(), l)),
        );
    }
    Ok(out)
}

/// Copy every group file into `dest`, checking each first. A leak copies nothing:
/// nothing already in `dest` is overwritten or removed.
pub fn copy_fixtures(
    groups: &[Group],
    dest: &Path,
    rules: &dyn RuleSet,
) -> io::Result<Vec<(PathBuf, Leak)>> {
    let mut pending: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    let mut leaks = Vec::new();
    for file in groups.iter().flat_map(|g| &g.files) {
        let target = dest.join(file.file_name().expect("group files have names"));
        let bytes = std::fs::read(file)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        leaks.extend(
            check_text(&text, rules)
                .into_iter()
                .map(|l| (target.clone(), l)),
        );
        pending.push((target, bytes));
    }
    if !leaks.is_empty() {
        return Ok(leaks);
    }
    redact::create_private_dir(dest)?;
    for (target, bytes) in pending {
        redact::write_private(&target, &bytes)?;
    }
    Ok(leaks)
}
