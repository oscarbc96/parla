//! JSON shape inference and Markdown rendering for `recon extract`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde_json::Value;

/// Most enum values kept for a string field; more than this means "just a string".
const MAX_ENUMS: usize = 8;

/// Field names whose string values are worth listing (a closed vocabulary).
const ENUM_KEYS: [&str; 3] = ["type", "subtype", "error"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    Null,
    Bool,
    Number,
    String {
        enums: BTreeSet<String>,
    },
    Array(Box<Shape>),
    Object(BTreeMap<String, Field>),
    /// Several distinct shapes seen for one slot. Empty means "nothing seen" (empty array).
    Mixed(Vec<Shape>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub shape: Shape,
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExchangeKind {
    Http,
    Ws,
}

fn shape_of(value: &Value, key: Option<&str>) -> Shape {
    match value {
        Value::Null => Shape::Null,
        Value::Bool(_) => Shape::Bool,
        Value::Number(_) => Shape::Number,
        Value::String(s) => {
            let mut enums = BTreeSet::new();
            if key.is_some_and(|k| ENUM_KEYS.contains(&k)) {
                enums.insert(s.clone());
            }
            Shape::String { enums }
        }
        Value::Array(items) => Shape::Array(Box::new(
            items
                .iter()
                .map(|v| shape_of(v, key))
                .reduce(merge)
                .unwrap_or(Shape::Mixed(Vec::new())),
        )),
        Value::Object(map) => Shape::Object(
            map.iter()
                .map(|(k, v)| {
                    let field = Field {
                        shape: shape_of(v, Some(k)),
                        optional: false,
                    };
                    (k.clone(), field)
                })
                .collect(),
        ),
    }
}

fn variants(shape: Shape) -> Vec<Shape> {
    match shape {
        Shape::Mixed(v) => v,
        other => vec![other],
    }
}

/// Merge two shapes. Enum sets are unioned without a cap; [`infer`] caps them once at the end.
pub fn merge(a: Shape, b: Shape) -> Shape {
    let mut out: Vec<Shape> = Vec::new();
    for v in variants(a).into_iter().chain(variants(b)) {
        let slot = out
            .iter()
            .position(|o| std::mem::discriminant(o) == std::mem::discriminant(&v));
        match slot {
            Some(i) => {
                let existing = std::mem::replace(&mut out[i], Shape::Null);
                out[i] = merge_same(existing, v);
            }
            None => out.push(v),
        }
    }
    match out.len() {
        1 => out.remove(0),
        _ => Shape::Mixed(out),
    }
}

/// Merge two shapes of the same variant.
fn merge_same(a: Shape, b: Shape) -> Shape {
    match (a, b) {
        (Shape::String { mut enums }, Shape::String { enums: more }) => {
            enums.extend(more);
            Shape::String { enums }
        }
        (Shape::Array(a), Shape::Array(b)) => Shape::Array(Box::new(merge(*a, *b))),
        (Shape::Object(mut a), Shape::Object(mut b)) => {
            let keys: BTreeSet<String> = a.keys().chain(b.keys()).cloned().collect();
            let mut out = BTreeMap::new();
            for k in keys {
                let field = match (a.remove(&k), b.remove(&k)) {
                    (Some(x), Some(y)) => Field {
                        shape: merge(x.shape, y.shape),
                        optional: x.optional || y.optional,
                    },
                    (Some(x), None) | (None, Some(x)) => Field {
                        shape: x.shape,
                        optional: true,
                    },
                    (None, None) => unreachable!("key came from one of the maps"),
                };
                out.insert(k, field);
            }
            Shape::Object(out)
        }
        (a, _) => a,
    }
}

fn cap_enums(shape: &mut Shape) {
    match shape {
        Shape::String { enums } if enums.len() > MAX_ENUMS => enums.clear(),
        Shape::Array(inner) => cap_enums(inner),
        Shape::Object(map) => map.values_mut().for_each(|f| cap_enums(&mut f.shape)),
        Shape::Mixed(list) => list.iter_mut().for_each(cap_enums),
        _ => {}
    }
}

/// Infer one shape from many samples. A key missing from any sample is optional.
pub fn infer(samples: &[Value]) -> Shape {
    let mut shape = samples
        .iter()
        .map(|v| shape_of(v, None))
        .reduce(merge)
        .unwrap_or(Shape::Mixed(Vec::new()));
    cap_enums(&mut shape);
    shape
}

pub fn label(shape: &Shape) -> String {
    match shape {
        Shape::Null => "null".into(),
        Shape::Bool => "bool".into(),
        Shape::Number => "number".into(),
        Shape::String { .. } => "string".into(),
        Shape::Array(inner) => match &**inner {
            Shape::Mixed(v) if v.is_empty() => "array".into(),
            inner => format!("array<{}>", label(inner)),
        },
        Shape::Object(_) => "object".into(),
        Shape::Mixed(list) => list.iter().map(label).collect::<Vec<_>>().join(" | "),
    }
}

fn enum_values(shape: &Shape) -> Vec<&str> {
    match shape {
        Shape::String { enums } => enums.iter().map(String::as_str).collect(),
        Shape::Mixed(list) => list.iter().flat_map(enum_values).collect(),
        _ => Vec::new(),
    }
}

struct Row {
    path: String,
    ty: String,
    optional: bool,
    values: String,
}

fn children(path: &str, shape: &Shape, rows: &mut Vec<Row>) {
    match shape {
        Shape::Object(map) => {
            for (k, f) in map {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                let values = enum_values(&f.shape)
                    .iter()
                    .map(|v| format!("`{v}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                rows.push(Row {
                    path: p.clone(),
                    ty: label(&f.shape),
                    optional: f.optional,
                    values,
                });
                children(&p, &f.shape, rows);
            }
        }
        Shape::Array(inner) => children(&format!("{path}[]"), inner, rows),
        Shape::Mixed(list) => list.iter().for_each(|s| children(path, s, rows)),
        _ => {}
    }
}

/// Draft documentation section for one method or event.
pub fn render_markdown(
    name: &str,
    kind: ExchangeKind,
    params: &BTreeSet<String>,
    shape: &Shape,
) -> String {
    let mut out = String::new();
    let kind = match kind {
        ExchangeKind::Http => "http",
        ExchangeKind::Ws => "ws",
    };
    let _ = writeln!(out, "### {name}\n\nKind: {kind}\n");
    if !params.is_empty() {
        let list: Vec<String> = params.iter().map(|p| format!("`{p}`")).collect();
        let _ = writeln!(out, "Request params: {}\n", list.join(", "));
    }
    let mut rows = Vec::new();
    children("", shape, &mut rows);
    if rows.is_empty() {
        let _ = writeln!(out, "Response shape: {}\n", label(shape));
    } else {
        out.push_str("| Field | Type | Optional | Values seen |\n|---|---|---|---|\n");
        for r in rows {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} |",
                r.path,
                r.ty,
                if r.optional { "yes" } else { "no" },
                r.values
            );
        }
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}
