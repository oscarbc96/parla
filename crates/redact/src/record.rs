//! `PARLA_RECORD` recorder. Debug builds only.
//!
//! Every exchange and websocket frame goes through the [`Redactor`] before it
//! touches disk, in the same layout `recon redact` writes, so `recon extract`
//! and `recon:diff` read a recording like any other capture. A file whose
//! redacted text still trips the leak checker is dropped, never written.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::doc;
use crate::{IdMap, Redactor, RuleSet, check_text, create_private_dir, write_private};

/// Environment variable naming the output directory.
pub const ENV_VAR: &str = "PARLA_RECORD";

/// Which way a websocket frame travelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Send,
    Receive,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Send => "send",
            Direction::Receive => "receive",
        }
    }
}

/// Recording exists only in debug builds.
pub const fn enabled() -> bool {
    cfg!(debug_assertions)
}

struct State {
    redactor: Redactor,
    http: usize,
    ws: usize,
    index: Vec<Value>,
}

pub struct Recorder {
    dir: PathBuf,
    state: Mutex<State>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder").field("dir", &self.dir).finish()
    }
}

impl Recorder {
    /// Record into `dir` (created owner-only if missing). `None` in release builds.
    pub fn new(dir: &Path, rules: Arc<dyn RuleSet>) -> Option<Recorder> {
        if !enabled() {
            return None;
        }
        create_private_dir(dir).ok()?;
        Some(Recorder {
            dir: dir.to_path_buf(),
            state: Mutex::new(State {
                redactor: Redactor::new(IdMap::new(), rules),
                http: 0,
                ws: 0,
                index: Vec::new(),
            }),
        })
    }

    /// `PARLA_RECORD=<dir>` turns recording on. `None` when unset, empty or in release builds.
    pub fn from_env(rules: Arc<dyn RuleSet>) -> Option<Recorder> {
        Self::from_value(std::env::var(ENV_VAR).ok().as_deref(), rules)
    }

    /// [`Recorder::from_env`] with the variable's value passed in.
    pub fn from_value(value: Option<&str>, rules: Arc<dyn RuleSet>) -> Option<Recorder> {
        let dir = value.filter(|v| !v.is_empty())?;
        Self::new(Path::new(dir), rules)
    }

    /// Writes `doc` as `name` unless its redacted text still trips the leak
    /// checker; returns whether the file is on disk.
    fn write(&self, st: &mut State, name: &str, doc: &Value) -> bool {
        let text = serde_json::to_string_pretty(doc).expect("json serialises") + "\n";
        if !check_text(&text, st.redactor.rules()).is_empty() {
            return false;
        }
        write_private(&self.dir.join(name), text.as_bytes()).is_ok()
    }

    fn write_index(&self, st: &mut State) {
        let doc = Value::Array(st.index.clone());
        self.write(st, "index.json", &doc);
    }

    /// Record one HTTP exchange. `form` is the decoded request form (the `token` field
    /// included; it is redacted here).
    pub fn record_http(
        &self,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        form: &[(String, String)],
        status: u16,
        body: &str,
    ) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        let st = &mut *st;
        let r = &mut st.redactor;
        let url = r.redact_url(url);
        let rules = Arc::clone(r.rules_arc());
        let label = url::Url::parse(&url)
            .ok()
            .and_then(|u| rules.method_name(&u))
            .map(|m| doc::sanitize(&m))
            .unwrap_or_else(|| doc::sanitize(&method.to_ascii_lowercase()));
        let kept: Vec<Value> = headers
            .iter()
            .filter_map(|(k, v)| {
                let k = k.to_ascii_lowercase();
                doc::keep_header(&k, &*rules).then(|| json!([k, r.redact_url(v)]))
            })
            .collect();
        let mut fields = form.to_vec();
        r.redact_form(&mut fields);
        let fields: Vec<Value> = fields.into_iter().map(|(k, v)| json!([k, v])).collect();
        let mut request = json!({ "headers": kept, "query": [] });
        if !fields.is_empty() {
            request["body"] = json!({ "form": fields });
        }
        let mut response = json!({ "status": status, "mime": "", "headers": [] });
        if !body.is_empty() {
            response["body"] = doc::body_value(r, "", body);
        }
        st.http += 1;
        let name = doc::http_file(st.http, &label);
        let doc = json!({
            "method": method.to_ascii_uppercase(),
            "url": url,
            "request": request,
            "response": response,
        });
        let entry = doc::http_index(&name, &label, status.into());
        if self.write(st, &name, &doc) {
            st.index.push(entry);
            self.write_index(st);
        }
    }

    /// Record one websocket text frame.
    pub fn record_ws(&self, direction: Direction, frame: &str) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        let st = &mut *st;
        let dir = direction.as_str();
        let (kind, data) = match serde_json::from_str::<Value>(frame) {
            Ok(mut v) => {
                st.redactor.redact_json(&mut v);
                let kind = v
                    .get("type")
                    .and_then(Value::as_str)
                    .map(doc::sanitize)
                    .unwrap_or_else(|| dir.to_string());
                (kind, json!({ "json": v }))
            }
            Err(_) => (
                dir.to_string(),
                json!({ "text": st.redactor.redact_prose(frame) }),
            ),
        };
        st.ws += 1;
        let name = doc::ws_file(st.ws, &kind);
        let doc = json!({
            "direction": dir,
            "opcode": 1,
            "url": "wss://ws.example.test/",
            "data": data,
        });
        let entry = doc::ws_index(&name, &kind);
        if self.write(st, &name, &doc) {
            st.index.push(entry);
            self.write_index(st);
        }
    }
}
