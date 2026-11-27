//! Legacy attachments and link unfurls into `Block::Card`.

use domain::{Block, Card, ImageRef, Inline, Style};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::blocks::flatten;
use crate::mrkdwn;

pub fn parse_attachments(json: &RawValue) -> Vec<Block> {
    match serde_json::from_str::<Value>(json.get()) {
        Ok(v) => crate::tidy::blocks(attachments_from_value(&v)),
        Err(_) => Vec::new(),
    }
}

pub(crate) fn attachments_from_value(v: &Value) -> Vec<Block> {
    let mut out = Vec::new();
    for a in v.as_array().into_iter().flatten() {
        if a.is_object() {
            attachment(a, &mut out);
        }
    }
    out
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|t| !t.is_empty())
}

fn color(v: &Value) -> Option<u32> {
    let c = s(v, "color")?.trim_start_matches('#');
    match c {
        "good" => Some(0x2eb886),
        "warning" => Some(0xdaa038),
        "danger" => Some(0xa30200),
        _ if c.len() == 6 => u32::from_str_radix(c, 16).ok(),
        _ => None,
    }
}

fn plain(t: &str) -> Vec<Inline> {
    vec![Inline::Text {
        text: t.into(),
        style: Style::default(),
    }]
}

fn rich(t: &str, md: bool) -> Vec<Inline> {
    if md {
        flatten(&mrkdwn::parse(t).blocks)
    } else {
        plain(t)
    }
}

fn linked(label: &str, url: Option<&str>) -> Inline {
    match url {
        Some(u) => Inline::Link {
            url: u.into(),
            label: Some(label.into()),
            style: Style::default(),
        },
        None => Inline::Text {
            text: label.into(),
            style: Style::default(),
        },
    }
}

/// The text the platform puts in `fallback` when it has no preview.
fn is_placeholder(t: &str) -> bool {
    t.trim().eq_ignore_ascii_case("[no preview available]")
}

fn attachment(a: &Value, out: &mut Vec<Block>) {
    // Without `mrkdwn_in`, text, pretext and fields are parsed as mrkdwn.
    let md: Vec<&str> = a
        .get("mrkdwn_in")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).collect())
        .unwrap_or_else(|| vec!["text", "pretext", "fields"]);
    let in_md = |k: &str| md.contains(&k);

    if let Some(p) = s(a, "pretext") {
        out.push(Block::Paragraph(rich(p, in_md("pretext"))));
    }
    let author = s(a, "author_name").map(|n| vec![linked(n, s(a, "author_link"))]);
    let title = s(a, "title").map(|t| linked(t, s(a, "title_link")));
    let mut body = Vec::new();
    if let Some(t) = s(a, "text") {
        if in_md("text") {
            body = mrkdwn::parse(t).blocks.to_vec();
        } else {
            body.push(Block::Paragraph(plain(t)));
        }
    }
    let fields: Vec<_> = a
        .get("fields")
        .and_then(Value::as_array)
        .map(|fs| {
            fs.iter()
                .filter_map(|f| {
                    let title = s(f, "title").unwrap_or("");
                    let value = s(f, "value")?;
                    Some((title.into(), rich(value, in_md("fields"))))
                })
                .collect()
        })
        .unwrap_or_default();
    let image = s(a, "image_url").map(|u| ImageRef::Web(u.into()));
    let thumb = s(a, "thumb_url").map(|u| ImageRef::Web(u.into()));
    let footer = s(a, "footer").map(|f| flatten(&mrkdwn::parse(f).blocks));
    if author.is_none()
        && title.is_none()
        && body.is_empty()
        && fields.is_empty()
        && let Some(fb) = s(a, "fallback")
        && !is_placeholder(fb)
    {
        body.push(Block::Paragraph(plain(fb)));
    }
    // An unfurl with nothing to show (only the placeholder fallback) is hidden.
    if author.is_none()
        && title.is_none()
        && body.is_empty()
        && fields.is_empty()
        && image.is_none()
        && thumb.is_none()
        && footer.is_none()
    {
        return;
    }
    out.push(Block::Card(Card {
        color: color(a),
        author,
        title,
        body,
        fields,
        image,
        thumb,
        footer,
    }));
}
