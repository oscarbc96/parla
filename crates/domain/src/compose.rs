//! Composer output: text plus structured spans a provider encodes to its
//! own markup.

use std::ops::Range;
use std::sync::Arc;

use crate::ids::{ConversationId, UserId};
use crate::rich::{Broadcast, EmojiRef, Inline, Style};

/// A mention or emoji over a byte range of [`ComposedText::text`].
#[derive(Clone, Debug, PartialEq)]
pub struct ComposedSpan {
    pub range: Range<usize>,
    /// `User`, `Conversation`, `Group`, `Broadcast`, `Emoji` or `Link`.
    pub inline: Inline,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ComposedText {
    pub text: Arc<str>,
    pub spans: Vec<ComposedSpan>,
}

impl ComposedText {
    pub fn plain(text: &str) -> ComposedText {
        ComposedText {
            text: Arc::from(text),
            spans: Vec::new(),
        }
    }

    /// Spans must lie on char boundaries of `text` (checked in debug builds).
    pub fn new(text: &str, spans: Vec<ComposedSpan>) -> ComposedText {
        for s in &spans {
            debug_assert!(
                text.is_char_boundary(s.range.start)
                    && text.is_char_boundary(s.range.end)
                    && s.range.start <= s.range.end,
                "span range {:?} is not on char boundaries of the text",
                s.range
            );
            debug_assert!(
                matches!(
                    s.inline,
                    Inline::User { .. }
                        | Inline::Conversation { .. }
                        | Inline::Group { .. }
                        | Inline::Broadcast(_)
                        | Inline::Emoji(_)
                        | Inline::Link { .. }
                ),
                "composed spans are mentions, emoji or links"
            );
        }
        ComposedText {
            text: Arc::from(text),
            spans,
        }
    }
}

impl ComposedText {
    /// The same text with the label of some spans replaced: `label` returns the
    /// new text of a span (`None` keeps it). Later spans shift with the text.
    /// Used to show a mention by name where only its id is known.
    pub fn relabel(&self, label: impl Fn(&Inline) -> Option<String>) -> ComposedText {
        let mut spans: Vec<&ComposedSpan> = self.spans.iter().collect();
        spans.sort_by_key(|s| s.range.start);
        let mut text = String::with_capacity(self.text.len());
        let mut out = Vec::with_capacity(spans.len());
        let mut at = 0;
        for s in spans {
            if s.range.start < at || s.range.end > self.text.len() {
                continue;
            }
            text.push_str(&self.text[at..s.range.start]);
            let start = text.len();
            match label(&s.inline) {
                Some(l) => text.push_str(&l),
                None => text.push_str(&self.text[s.range.clone()]),
            }
            out.push(ComposedSpan {
                range: start..text.len(),
                inline: s.inline.clone(),
            });
            at = s.range.end;
        }
        text.push_str(&self.text[at..]);
        ComposedText::new(&text, out)
    }
}

impl From<&str> for ComposedText {
    fn from(s: &str) -> Self {
        Self::plain(s)
    }
}

impl From<String> for ComposedText {
    fn from(s: String) -> Self {
        Self::plain(&s)
    }
}

/// The spans as JSON for a saved draft, `None` when there are none.
pub fn spans_to_json(spans: &[ComposedSpan]) -> Option<String> {
    use serde_json::{Value, json};
    let rows: Vec<Value> = spans
        .iter()
        .filter_map(|s| {
            let (kind, extra) = match &s.inline {
                Inline::User { id, label } => ("user", json!({"id": &*id.0, "label": label.as_deref()})),
                Inline::Conversation { id, label } => {
                    ("conversation", json!({"id": &*id.0, "label": label.as_deref()}))
                }
                Inline::Group { id, label } => ("group", json!({"id": &**id, "label": label.as_deref()})),
                Inline::Broadcast(b) => (
                    "broadcast",
                    json!({"which": match b {
                        Broadcast::Here => "here",
                        Broadcast::Channel => "channel",
                        Broadcast::Everyone => "everyone",
                    }}),
                ),
                Inline::Emoji(e) => (
                    "emoji",
                    json!({"shortcode": &*e.shortcode, "unicode": e.unicode.as_deref(), "tone": e.skin_tone}),
                ),
                Inline::Link { url, label, .. } => ("link", json!({"url": &**url, "label": label.as_deref()})),
                _ => return None,
            };
            let mut v = extra;
            v["k"] = kind.into();
            v["s"] = s.range.start.into();
            v["e"] = s.range.end.into();
            Some(v)
        })
        .collect();
    (!rows.is_empty()).then(|| Value::Array(rows).to_string())
}

/// Spans read back from [`spans_to_json`]; entries that do not fit `text` are dropped.
pub fn spans_from_json(json: &str, text: &str) -> Vec<ComposedSpan> {
    let Ok(serde_json::Value::Array(rows)) = serde_json::from_str(json) else {
        return Vec::new();
    };
    let mut out: Vec<ComposedSpan> = rows
        .iter()
        .filter_map(|v| span_from_value(v, text))
        .collect();
    out.sort_by_key(|s| s.range.start);
    // Spans never overlap; keep the first of any that do.
    let mut end = 0;
    out.retain(|s| {
        let keep = s.range.start >= end;
        if keep {
            end = s.range.end;
        }
        keep
    });
    out
}

fn span_from_value(v: &serde_json::Value, text: &str) -> Option<ComposedSpan> {
    let int = |k: &str| v.get(k)?.as_u64().map(|n| n as usize);
    let string = |k: &str| v.get(k)?.as_str().map(Arc::<str>::from);
    let opt = |k: &str| v.get(k).and_then(|x| x.as_str()).map(Arc::<str>::from);
    let (start, end) = (int("s")?, int("e")?);
    if start >= end
        || end > text.len()
        || !text.is_char_boundary(start)
        || !text.is_char_boundary(end)
    {
        return None;
    }
    let inline = match v.get("k")?.as_str()? {
        "user" => Inline::User {
            id: UserId(string("id")?),
            label: opt("label"),
        },
        "conversation" => Inline::Conversation {
            id: ConversationId(string("id")?),
            label: opt("label"),
        },
        "group" => Inline::Group {
            id: string("id")?,
            label: opt("label"),
        },
        "broadcast" => Inline::Broadcast(match v.get("which")?.as_str()? {
            "here" => Broadcast::Here,
            "channel" => Broadcast::Channel,
            "everyone" => Broadcast::Everyone,
            _ => return None,
        }),
        "emoji" => Inline::Emoji(EmojiRef {
            shortcode: string("shortcode")?,
            unicode: opt("unicode"),
            skin_tone: v.get("tone").and_then(|t| t.as_u64()).map(|t| t as u8),
        }),
        "link" => Inline::Link {
            url: string("url")?,
            label: opt("label"),
            style: Style::default(),
        },
        _ => return None,
    };
    Some(ComposedSpan {
        range: start..end,
        inline,
    })
}

/// Turns composed text into the provider's wire markup.
pub trait ContentEncoder: Send + Sync {
    fn encode(&self, t: &ComposedText) -> String;

    /// The inverse of [`Self::encode`]: wire markup of an existing message back
    /// into composed text, its mentions and links as spans, so editing keeps
    /// them. Plain text by default.
    fn decode(&self, markup: &str) -> ComposedText {
        ComposedText::plain(markup)
    }
}

/// Span labels as typed, no markup.
pub struct PlainEncoder;

impl ContentEncoder for PlainEncoder {
    fn encode(&self, t: &ComposedText) -> String {
        t.text.to_string()
    }
}
