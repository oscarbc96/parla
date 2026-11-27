//! `recon diff`: compare a browser capture with a Parla recording by names and
//! counts only. Values are never read into the report.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

use serde_json::Value;

/// Credentials, never compared.
const EXCLUDED_PARAMS: [&str; 1] = ["token"];
/// Headers whose presence depends on the client, not on the protocol.
const NOISE_HEADERS: [&str; 10] = [
    "accept",
    "accept-encoding",
    "accept-language",
    "connection",
    "content-length",
    "host",
    "priority",
    "referer",
    "user-agent",
    "cookie",
];

/// One HTTP exchange reduced to names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    pub method: String,
    pub params: BTreeSet<String>,
    pub headers: BTreeSet<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Capture {
    /// HTTP exchanges in recorded order.
    pub exchanges: Vec<Exchange>,
    /// Types of websocket frames the client sent, in recorded order.
    pub ws_sent: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffItem {
    MissingCall {
        method: String,
    },
    ExtraCall {
        method: String,
    },
    ParamMissing {
        method: String,
        param: String,
    },
    ParamExtra {
        method: String,
        param: String,
    },
    HeaderMissing {
        method: String,
        header: String,
    },
    HeaderExtra {
        method: String,
        header: String,
    },
    OrderChanged {
        browser: Vec<String>,
        parla: Vec<String>,
    },
    WsSentMissing {
        kind: String,
    },
    WsSentExtra {
        kind: String,
    },
}

impl DiffItem {
    /// Items that make `recon diff` exit 1.
    pub fn is_missing(&self) -> bool {
        matches!(
            self,
            Self::MissingCall { .. }
                | Self::ParamMissing { .. }
                | Self::HeaderMissing { .. }
                | Self::WsSentMissing { .. }
        )
    }
}

impl std::fmt::Display for DiffItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingCall { method } => write!(f, "missing call: {method}"),
            Self::ExtraCall { method } => write!(f, "extra call: {method}"),
            Self::ParamMissing { method, param } => write!(f, "missing param: {method} {param}"),
            Self::ParamExtra { method, param } => write!(f, "extra param: {method} {param}"),
            Self::HeaderMissing { method, header } => {
                write!(f, "missing header: {method} {header}")
            }
            Self::HeaderExtra { method, header } => write!(f, "extra header: {method} {header}"),
            Self::OrderChanged { browser, parla } => write!(
                f,
                "call order differs: browser [{}] parla [{}]",
                browser.join(", "),
                parla.join(", ")
            ),
            Self::WsSentMissing { kind } => write!(f, "missing ws frame sent: {kind}"),
            Self::WsSentExtra { kind } => write!(f, "extra ws frame sent: {kind}"),
        }
    }
}

fn pair_names(v: Option<&Value>) -> impl Iterator<Item = &str> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| p.get(0).and_then(Value::as_str))
}

fn is_noise(name: &str) -> bool {
    NOISE_HEADERS.contains(&name) || name.starts_with("sec-")
}

fn label_of(name: &str) -> &str {
    name.split_once('-')
        .map_or(name, |(_, l)| l)
        .trim_end_matches(".json")
}

impl Capture {
    /// Load a directory in `recon redact` layout (also written by `PARLA_RECORD`).
    pub fn load(dir: &Path) -> io::Result<Capture> {
        let mut names: Vec<String> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".json") && n != "index.json")
            .collect();
        names.sort();
        let mut cap = Capture::default();
        for name in names {
            let Ok(doc) = serde_json::from_slice::<Value>(&std::fs::read(dir.join(&name))?) else {
                continue;
            };
            if name.starts_with("ws-") {
                if doc["direction"].as_str() == Some("send") {
                    cap.ws_sent
                        .push(label_of(name.strip_prefix("ws-").unwrap_or(&name)).to_string());
                }
                continue;
            }
            let req = &doc["request"];
            let mut params = BTreeSet::new();
            for n in pair_names(req.get("query")).chain(pair_names(req["body"].get("form"))) {
                if !EXCLUDED_PARAMS.contains(&n) {
                    params.insert(n.to_string());
                }
            }
            if let Some(obj) = req["body"]["json"].as_object() {
                params.extend(
                    obj.keys()
                        .filter(|k| !EXCLUDED_PARAMS.contains(&k.as_str()))
                        .cloned(),
                );
            }
            let headers = pair_names(req.get("headers"))
                .map(str::to_ascii_lowercase)
                .filter(|h| !is_noise(h))
                .collect();
            cap.exchanges.push(Exchange {
                method: label_of(&name).to_string(),
                params,
                headers,
            });
        }
        Ok(cap)
    }

    fn by_method(&self) -> BTreeMap<&str, (BTreeSet<&str>, BTreeSet<&str>)> {
        let mut m: BTreeMap<&str, (BTreeSet<&str>, BTreeSet<&str>)> = BTreeMap::new();
        for e in &self.exchanges {
            let slot = m.entry(&e.method).or_default();
            slot.0.extend(e.params.iter().map(String::as_str));
            slot.1.extend(e.headers.iter().map(String::as_str));
        }
        m
    }

    /// Methods in order of first call.
    fn order(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        self.exchanges
            .iter()
            .filter(|e| seen.insert(e.method.as_str()))
            .map(|e| e.method.clone())
            .collect()
    }
}

/// Compare names only. Params and headers are the union over a method's calls.
pub fn diff(browser: &Capture, parla: &Capture) -> Vec<DiffItem> {
    let (b, t) = (browser.by_method(), parla.by_method());
    let mut out = Vec::new();
    for (method, (bp, bh)) in &b {
        let Some((tp, th)) = t.get(method) else {
            out.push(DiffItem::MissingCall {
                method: method.to_string(),
            });
            continue;
        };
        for p in bp.difference(tp) {
            out.push(DiffItem::ParamMissing {
                method: method.to_string(),
                param: p.to_string(),
            });
        }
        for p in tp.difference(bp) {
            out.push(DiffItem::ParamExtra {
                method: method.to_string(),
                param: p.to_string(),
            });
        }
        for h in bh.difference(th) {
            out.push(DiffItem::HeaderMissing {
                method: method.to_string(),
                header: h.to_string(),
            });
        }
        for h in th.difference(bh) {
            out.push(DiffItem::HeaderExtra {
                method: method.to_string(),
                header: h.to_string(),
            });
        }
    }
    for method in t.keys().filter(|m| !b.contains_key(*m)) {
        out.push(DiffItem::ExtraCall {
            method: method.to_string(),
        });
    }
    let common = |c: &Capture, other: &BTreeMap<&str, _>| -> Vec<String> {
        c.order()
            .into_iter()
            .filter(|m| other.contains_key(m.as_str()))
            .collect()
    };
    let (bo, to) = (common(browser, &t), common(parla, &b));
    if bo != to {
        out.push(DiffItem::OrderChanged {
            browser: bo,
            parla: to,
        });
    }
    let (bw, tw): (BTreeSet<_>, BTreeSet<_>) = (
        browser.ws_sent.iter().collect(),
        parla.ws_sent.iter().collect(),
    );
    for k in bw.difference(&tw) {
        out.push(DiffItem::WsSentMissing { kind: (*k).clone() });
    }
    for k in tw.difference(&bw) {
        out.push(DiffItem::WsSentExtra { kind: (*k).clone() });
    }
    out
}
