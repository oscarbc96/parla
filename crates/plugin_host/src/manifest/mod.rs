//! `plugin.toml`: parser and validator.
//!
//! [`parse`] reads the TOML, reports every unknown key (API 0.1 denies
//! them, so a typo never drops a capability silently), deserialises the
//! rest and checks every rule, collecting all errors with their line and
//! dotted key instead of stopping at the first.

mod areas;
mod model;
mod validate;

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

pub use areas::{ReadArea, WriteArea};
pub use model::*;

/// What is wrong; the UI and the log map it to their own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// Not valid TOML.
    Syntax,
    /// A value of the wrong type, or a required key missing.
    Type,
    UnknownKey,
    InvalidId,
    InvalidNamespace,
    InvalidVersion,
    /// `api` is not `major.minor`.
    InvalidApi,
    /// This host cannot load the plugin's API version.
    UnsupportedApi,
    InvalidActivation,
    StartupActivation,
    ContributionWithoutEvent,
    EventWithoutContribution,
    NetworkScheme,
    NetworkWildcard,
    NetworkIpLiteral,
    NetworkInvalidHost,
    MemoryOutOfRange,
    UnknownArea,
    TooManyPatterns,
    InvalidPattern,
    ExecProgramPath,
    InvalidArgShape,
    InvalidSlashName,
    InvalidContributionId,
    DuplicateContribution,
    InvalidPath,
    InvalidSettingKey,
    SettingType,
    SettingDefault,
    /// `[dev]` outside dev mode.
    DevOnly,
    /// The file is over `limits::MAX_SOURCE_BYTES`.
    TooLarge,
    TooLong,
    TooMany,
    /// `default_key` is not a single keystroke with a modifier.
    InvalidKey,
    InvalidIcon,
    /// `repository` is not an https URL.
    InvalidRepository,
    InvalidFluentKey,
    /// An exec shape that makes a shell run a command string.
    ExecShell,
}

/// One problem in a manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError {
    /// 1-based line, when the problem has a place in the file.
    pub line: Option<usize>,
    /// Dotted key, `[i]` for array items (`capabilities.network[2]`).
    pub key: String,
    pub kind: ErrorKind,
    /// English detail for the plugin log and dev mode.
    pub detail: String,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(l) = self.line {
            write!(f, "line {l}: ")?;
        }
        if !self.key.is_empty() {
            write!(f, "{}: ", self.key)?;
        }
        f.write_str(&self.detail)
    }
}

/// Every problem found in a manifest, in file order.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct ManifestErrors(Vec<ManifestError>);

impl ManifestErrors {
    pub fn errors(&self) -> &[ManifestError] {
        &self.0
    }
}

impl fmt::Display for ManifestErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

/// Most unknown keys reported before giving up; bounds the retry loop.
const MAX_UNKNOWN_KEYS: usize = 64;

/// Parses and validates a `plugin.toml`.
pub fn parse(src: &str, source: ManifestSource) -> Result<Manifest, ManifestErrors> {
    if src.len() > limits::MAX_SOURCE_BYTES {
        return Err(ManifestErrors(vec![ManifestError {
            line: None,
            key: String::new(),
            kind: ErrorKind::TooLarge,
            detail: format!("plugin.toml is over {} bytes", limits::MAX_SOURCE_BYTES),
        }]));
    }
    let lines = LineIndex::new(src);
    let mut doc = DeTable::parse(src).map_err(|e| {
        ManifestErrors(vec![ManifestError {
            line: e.span().map(|s| lines.line(s.start)),
            key: String::new(),
            kind: ErrorKind::Syntax,
            detail: e.message().to_owned(),
        }])
    })?;
    let mut spans = HashMap::new();
    index_table(doc.get_ref(), "", &mut spans);
    let mut sink = Sink {
        lines: &lines,
        spans: &spans,
        errors: Vec::new(),
    };

    // Unknown keys: remove each one serde rejects and try again, so all of
    // them are reported, then the rest of the file is still checked.
    let mut raw = None;
    for _ in 0..=MAX_UNKNOWN_KEYS {
        let de = toml::de::Deserializer::from(doc.clone());
        match <validate::RawManifest as serde::Deserialize>::deserialize(de) {
            Ok(r) => {
                raw = Some(r);
                break;
            }
            Err(e) => {
                let span = e.span();
                let removed = e
                    .message()
                    .starts_with("unknown field")
                    .then(|| span.clone())
                    .flatten()
                    .and_then(|s| remove_key_at(doc.get_mut(), &s, ""));
                match removed {
                    Some(key) => sink.errors.push(ManifestError {
                        line: span.map(|s| lines.line(s.start)),
                        key,
                        kind: ErrorKind::UnknownKey,
                        detail: e.message().to_owned(),
                    }),
                    None => {
                        sink.type_error(&e);
                        break;
                    }
                }
            }
        }
    }
    let manifest = raw.map(|r| validate::validate(r, source, &mut sink));
    match manifest {
        Some(m) if sink.errors.is_empty() => Ok(m),
        _ => {
            let mut errors = sink.errors;
            errors.sort_by_key(|e| e.line.unwrap_or(usize::MAX));
            Err(ManifestErrors(errors))
        }
    }
}

/// Collects errors and knows where each key is.
pub(crate) struct Sink<'a> {
    lines: &'a LineIndex,
    spans: &'a HashMap<String, Range<usize>>,
    errors: Vec<ManifestError>,
}

impl Sink<'_> {
    pub(crate) fn push(
        &mut self,
        key: impl Into<String>,
        kind: ErrorKind,
        detail: impl Into<String>,
    ) {
        let key = key.into();
        let line = self.line_of(&key);
        self.errors.push(ManifestError {
            line,
            key,
            kind,
            detail: detail.into(),
        });
    }

    fn line_of(&self, key: &str) -> Option<usize> {
        self.spans.get(key).map(|s| self.lines.line(s.start))
    }

    fn type_error(&mut self, e: &toml::de::Error) {
        let msg = e.message();
        // The innermost key whose value holds the error span.
        let path = e
            .span()
            .and_then(|s| {
                self.spans
                    .iter()
                    .filter(|(_, v)| v.start <= s.start && s.end <= v.end)
                    .min_by_key(|(_, v)| v.end - v.start)
                    .map(|(k, _)| k.clone())
            })
            .unwrap_or_default();
        let key = match msg
            .strip_prefix("missing field `")
            .and_then(|r| r.split('`').next())
        {
            Some(field) if path.is_empty() => field.to_owned(),
            Some(field) => format!("{path}.{field}"),
            None => path,
        };
        self.errors.push(ManifestError {
            line: e.span().map(|s| self.lines.line(s.start)),
            key,
            kind: ErrorKind::Type,
            detail: msg.to_owned(),
        });
    }
}

/// Byte offset to 1-based line.
struct LineIndex(Vec<usize>);

impl LineIndex {
    fn new(src: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        Self(starts)
    }

    fn line(&self, offset: usize) -> usize {
        self.0.partition_point(|&s| s <= offset)
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Records the span of every value under its dotted key.
fn index_table(t: &DeTable<'_>, prefix: &str, out: &mut HashMap<String, Range<usize>>) {
    for (k, v) in t.iter() {
        let path = join(prefix, k.get_ref());
        index_value(v, &path, out);
    }
}

fn index_value(v: &Spanned<DeValue<'_>>, path: &str, out: &mut HashMap<String, Range<usize>>) {
    out.insert(path.to_owned(), v.span());
    match v.get_ref() {
        DeValue::Table(t) => index_table(t, path, out),
        DeValue::Array(a) => {
            for (i, item) in a.iter().enumerate() {
                index_value(item, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// Removes the key whose name sits at `span`; returns its dotted path.
fn remove_key_at(t: &mut DeTable<'_>, span: &Range<usize>, prefix: &str) -> Option<String> {
    let found = t
        .iter()
        .find(|(k, _)| k.span() == *span)
        .map(|(k, _)| k.get_ref().to_string());
    if let Some(name) = found {
        t.remove(name.as_str());
        return Some(join(prefix, &name));
    }
    for (k, v) in t.iter_mut() {
        let path = join(prefix, k.get_ref());
        if let Some(p) = remove_in_value(v.get_mut(), span, &path) {
            return Some(p);
        }
    }
    None
}

fn remove_in_value(v: &mut DeValue<'_>, span: &Range<usize>, path: &str) -> Option<String> {
    match v {
        DeValue::Table(t) => remove_key_at(t, span, path),
        DeValue::Array(a) => a
            .iter_mut()
            .enumerate()
            .find_map(|(i, item)| remove_in_value(item.get_mut(), span, &format!("{path}[{i}]"))),
        _ => None,
    }
}
