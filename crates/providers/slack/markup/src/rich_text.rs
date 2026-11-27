//! The `rich_text` block and its elements.

use domain::{Block, Broadcast, ConversationId, EmojiRef, Inline, ListStyle, Style, UserId};
use serde_json::Value;

use crate::emoji_ref;

pub(crate) fn parse(b: &Value) -> Vec<Block> {
    let Some(els) = b.get("elements").and_then(Value::as_array) else {
        return Vec::new();
    };
    els.iter().filter_map(element).collect()
}

fn element(e: &Value) -> Option<Block> {
    let inlines = || inline_run(e.get("elements"));
    match e.get("type")?.as_str()? {
        "rich_text_section" => Some(Block::Paragraph(inlines())),
        "rich_text_quote" => Some(Block::Quote(vec![Block::Paragraph(inlines())])),
        "rich_text_preformatted" => Some(Block::Code {
            lang: None,
            text: raw_text(e.get("elements")).into(),
        }),
        "rich_text_list" => {
            let style = match e.get("style").and_then(Value::as_str) {
                Some("ordered") => ListStyle::Ordered {
                    start: (e.get("offset").and_then(Value::as_u64).unwrap_or(0) as u32)
                        .saturating_add(1),
                },
                _ => ListStyle::Bullet,
            };
            let indent = e
                .get("indent")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(255) as u8;
            let items = e
                .get("elements")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|i| inline_run(i.get("elements")))
                        .collect()
                })
                .unwrap_or_default();
            Some(Block::List {
                style,
                indent,
                items,
            })
        }
        _ => None,
    }
}

fn raw_text(els: Option<&Value>) -> String {
    els.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| e.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

fn inline_run(els: Option<&Value>) -> Vec<Inline> {
    els.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(inline).collect())
        .unwrap_or_default()
}

fn style_of(e: &Value) -> (Style, bool) {
    let s = e.get("style");
    let flag = |k: &str| {
        s.and_then(|s| s.get(k))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    (
        Style {
            bold: flag("bold"),
            italic: flag("italic"),
            strike: flag("strike"),
        },
        flag("code"),
    )
}

fn s<'a>(e: &'a Value, k: &str) -> Option<&'a str> {
    e.get(k).and_then(Value::as_str)
}

fn inline(e: &Value) -> Option<Inline> {
    match s(e, "type")? {
        "text" => {
            let text = s(e, "text")?;
            let (style, code) = style_of(e);
            Some(if code {
                Inline::Code(text.into())
            } else {
                Inline::Text {
                    text: text.into(),
                    style,
                }
            })
        }
        "link" => {
            let (style, _) = style_of(e);
            Some(Inline::Link {
                url: s(e, "url")?.into(),
                label: s(e, "text").filter(|t| !t.is_empty()).map(Into::into),
                style,
            })
        }
        "user" => Some(Inline::User {
            id: UserId(s(e, "user_id")?.into()),
            label: None,
        }),
        "channel" => Some(Inline::Conversation {
            id: ConversationId(s(e, "channel_id")?.into()),
            label: None,
        }),
        "usergroup" => Some(Inline::Group {
            id: s(e, "usergroup_id")?.into(),
            label: None,
        }),
        "broadcast" => Some(Inline::Broadcast(match s(e, "range")? {
            "here" => Broadcast::Here,
            "channel" => Broadcast::Channel,
            "everyone" => Broadcast::Everyone,
            _ => return None,
        })),
        "emoji" => Some(Inline::Emoji(emoji(e)?)),
        "date" => Some(Inline::Date {
            unix: e.get("timestamp").and_then(Value::as_i64)?,
            fallback: s(e, "fallback").unwrap_or("").into(),
        }),
        "color" => Some(Inline::Text {
            text: s(e, "value")?.into(),
            style: Style::default(),
        }),
        _ => None,
    }
}

fn emoji(e: &Value) -> Option<EmojiRef> {
    let mut r = emoji_ref(s(e, "name")?);
    if let Some(hex) = s(e, "unicode") {
        let chars: Option<String> = hex
            .split('-')
            .map(|h| u32::from_str_radix(h, 16).ok().and_then(char::from_u32))
            .collect();
        if let Some(c) = chars.filter(|c| !c.is_empty()) {
            r.unicode = Some(c.into());
        }
    }
    Some(r)
}
