//! Block Kit layout blocks into neutral blocks. Never panics.

use std::sync::Arc;

use domain::{Block, ContextItem, Control, ControlKind, ImageRef, Inline, RemoteUrl, Style};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::{mrkdwn, rich_text};

pub fn parse_blocks(json: &RawValue) -> Vec<Block> {
    match serde_json::from_str::<Value>(json.get()) {
        Ok(v) => crate::tidy::blocks(blocks_from_value(&v)),
        Err(_) => Vec::new(),
    }
}

pub(crate) fn blocks_from_value(v: &Value) -> Vec<Block> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for b in arr {
        block(b, &mut out);
    }
    out
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Text of a `{type, text}` composition object (or a bare string).
fn text_of(v: Option<&Value>) -> Option<&str> {
    match v? {
        Value::String(s) => Some(s),
        o => o.get("text").and_then(Value::as_str),
    }
}

fn plain_inline(s: &str) -> Vec<Inline> {
    vec![Inline::Text {
        text: s.into(),
        style: Style::default(),
    }]
}

/// Inlines of a text object: mrkdwn is parsed, plain_text kept literal.
pub(crate) fn text_object_inlines(v: Option<&Value>) -> Vec<Inline> {
    let Some(t) = text_of(v) else {
        return Vec::new();
    };
    let is_md = v
        .and_then(|o| o.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|t| t == "mrkdwn");
    if is_md {
        flatten(&mrkdwn::parse(t).blocks)
    } else {
        plain_inline(t)
    }
}

/// Collapse blocks into one inline run, paragraphs separated by line breaks.
pub(crate) fn flatten(blocks: &[Block]) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    for b in blocks {
        let part = match b {
            Block::Paragraph(i) | Block::Heading(i) => i.clone(),
            Block::Code { text, .. } => vec![Inline::Code(text.clone())],
            Block::List { items, .. } => items
                .iter()
                .flat_map(|i| i.iter().cloned().chain([Inline::LineBreak]))
                .collect(),
            Block::Quote(bs) => flatten(bs),
            _ => continue,
        };
        if !out.is_empty() {
            out.push(Inline::LineBreak);
        }
        out.extend(part);
    }
    out
}

pub(crate) fn image_ref(v: &Value) -> Option<ImageRef> {
    if let Some(u) = str_of(v, "image_url") {
        return Some(ImageRef::Web(u.into()));
    }
    let f = v.get("slack_file")?;
    let u = str_of(f, "url").or_else(|| str_of(f, "id"))?;
    Some(ImageRef::Provider(RemoteUrl(u.into())))
}

fn fallback_text(v: &Value) -> Arc<str> {
    let t = text_of(v.get("text"))
        .or_else(|| text_of(v.get("title")))
        .or_else(|| str_of(v, "alt_text"))
        .or_else(|| str_of(v, "fallback"))
        .unwrap_or("");
    t.into()
}

fn control_kind(ty: &str) -> ControlKind {
    match ty {
        "button" => ControlKind::Button,
        t if t.ends_with("_select") => ControlKind::Select,
        t if t.contains("_select_") => ControlKind::Select,
        _ => ControlKind::Other,
    }
}

fn control(v: &Value, label_fallback: Option<&str>) -> Control {
    let ty = str_of(v, "type").unwrap_or("");
    let label = text_of(v.get("text"))
        .or_else(|| text_of(v.get("placeholder")))
        .or(label_fallback)
        .unwrap_or("");
    Control {
        label: label.into(),
        kind: control_kind(ty),
    }
}

fn block(b: &Value, out: &mut Vec<Block>) {
    let Some(ty) = str_of(b, "type") else {
        out.push(Block::Unsupported {
            fallback: fallback_text(b),
        });
        return;
    };
    match ty {
        "section" => out.push(section(b)),
        "header" => out.push(Block::Heading(text_object_inlines(b.get("text")))),
        "divider" => out.push(Block::Divider),
        "call" => out.push(Block::Call(crate::calls::call_block_card(b))),
        "image" => out.push(image(b)),
        "context" => out.push(Block::Context(
            b.get("elements")
                .and_then(Value::as_array)
                .map(|els| els.iter().filter_map(context_item).collect())
                .unwrap_or_default(),
        )),
        "actions" => out.push(Block::Actions(
            b.get("elements")
                .and_then(Value::as_array)
                .map(|els| els.iter().map(|e| control(e, None)).collect())
                .unwrap_or_default(),
        )),
        "input" => {
            let label = text_of(b.get("label"));
            let c = match b.get("element") {
                Some(e) => control(e, label),
                None => Control {
                    label: label.unwrap_or("").into(),
                    kind: ControlKind::Other,
                },
            };
            let label = label
                .map(Arc::<str>::from)
                .unwrap_or_else(|| c.label.clone());
            out.push(Block::Actions(vec![Control { label, ..c }]));
        }
        "rich_text" => out.extend(rich_text::parse(b)),
        "table" => out.push(table(b)),
        _ => out.push(Block::Unsupported {
            fallback: fallback_text(b),
        }),
    }
}

fn section(b: &Value) -> Block {
    let text = match text_of(b.get("text")) {
        Some(_) => {
            let inl = text_object_inlines(b.get("text"));
            // Keep mrkdwn paragraph structure.
            let is_md = b.pointer("/text/type").and_then(Value::as_str) == Some("mrkdwn");
            if is_md {
                mrkdwn::parse(text_of(b.get("text")).unwrap_or(""))
                    .blocks
                    .to_vec()
            } else {
                vec![Block::Paragraph(inl)]
            }
        }
        None => Vec::new(),
    };
    let fields = b
        .get("fields")
        .and_then(Value::as_array)
        .map(|fs| fs.iter().map(|f| text_object_inlines(Some(f))).collect())
        .unwrap_or_default();
    let accessory = b.get("accessory").map(|a| {
        Box::new(match str_of(a, "type") {
            Some("image") => image(a),
            Some(_) => Block::Actions(vec![control(a, None)]),
            None => Block::Unsupported {
                fallback: fallback_text(a),
            },
        })
    });
    Block::Section {
        text,
        fields,
        accessory,
    }
}

fn image(b: &Value) -> Block {
    match image_ref(b) {
        Some(source) => Block::Image {
            source,
            alt: str_of(b, "alt_text").unwrap_or("").into(),
            title: text_of(b.get("title")).map(Into::into),
        },
        None => Block::Unsupported {
            fallback: fallback_text(b),
        },
    }
}

fn context_item(e: &Value) -> Option<ContextItem> {
    match str_of(e, "type")? {
        "image" => Some(ContextItem::Image {
            source: image_ref(e)?,
            alt: str_of(e, "alt_text").unwrap_or("").into(),
        }),
        "mrkdwn" | "plain_text" => Some(ContextItem::Text(text_object_inlines(Some(e)))),
        _ => None,
    }
}

fn table(b: &Value) -> Block {
    let rows = b
        .get("rows")
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .map(|r| {
                    r.as_array()
                        .map(|cells| cells.iter().map(cell).collect())
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    Block::Table {
        header: b.get("header").and_then(Value::as_bool).unwrap_or(false),
        rows,
    }
}

fn cell(c: &Value) -> Vec<Inline> {
    match str_of(c, "type") {
        Some("rich_text") => flatten(&rich_text::parse(c)),
        _ => text_of(Some(c)).map(plain_inline).unwrap_or_default(),
    }
}
