//! Neutral rich-text model that provider markup parsers render into.

use std::sync::Arc;

use crate::ids::{ConversationId, RemoteUrl, UserId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageRef {
    Provider(RemoteUrl),
    Web(Arc<str>),
}

/// `unicode` is set for standard emoji, `None` for custom (looked up by `shortcode`).
#[derive(Clone, Debug, PartialEq)]
pub struct EmojiRef {
    pub shortcode: Arc<str>,
    pub unicode: Option<Arc<str>>,
    pub skin_tone: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Broadcast {
    Here,
    Channel,
    Everyone,
}

/// Who a message addresses.
#[derive(Clone, Debug, PartialEq)]
pub enum Mention {
    User(UserId),
    Group(Arc<str>),
    Broadcast(Broadcast),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text {
        text: Arc<str>,
        style: Style,
    },
    Code(Arc<str>),
    Link {
        url: Arc<str>,
        label: Option<Arc<str>>,
        style: Style,
    },
    User {
        id: UserId,
        label: Option<Arc<str>>,
    },
    Conversation {
        id: ConversationId,
        label: Option<Arc<str>>,
    },
    Group {
        id: Arc<str>,
        label: Option<Arc<str>>,
    },
    Broadcast(Broadcast),
    Emoji(EmojiRef),
    Date {
        unix: i64,
        fallback: Arc<str>,
    },
    LineBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListStyle {
    Bullet,
    Ordered { start: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Button,
    Select,
    Other,
}

/// Always rendered disabled.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub label: Arc<str>,
    pub kind: ControlKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContextItem {
    Text(Vec<Inline>),
    Image { source: ImageRef, alt: Arc<str> },
}

/// Link unfurls and legacy attachments.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub color: Option<u32>,
    pub author: Option<Vec<Inline>>,
    pub title: Option<Inline>,
    pub body: Vec<Block>,
    pub fields: Vec<(Arc<str>, Vec<Inline>)>,
    pub image: Option<ImageRef>,
    pub thumb: Option<ImageRef>,
    pub footer: Option<Vec<Inline>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CallKind {
    Huddle,
    External { provider: Arc<str> },
}

/// A huddle or third-party call shown in a message.
#[derive(Clone, Debug, PartialEq)]
pub struct CallCard {
    pub kind: CallKind,
    pub title: Option<Arc<str>>,
    pub join_url: Option<Arc<str>>,
    /// Unix seconds.
    pub started: Option<i64>,
    pub ended: Option<i64>,
    pub participants: Arc<[UserId]>,
    /// Who started the call (the room's creator), not the message's sender,
    /// which may be a bot posting on the creator's behalf.
    pub created_by: Option<UserId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading(Vec<Inline>),
    Quote(Vec<Block>),
    Code {
        lang: Option<Arc<str>>,
        text: Arc<str>,
    },
    List {
        style: ListStyle,
        indent: u8,
        items: Vec<Vec<Inline>>,
    },
    Divider,
    Image {
        source: ImageRef,
        alt: Arc<str>,
        title: Option<Arc<str>>,
    },
    Context(Vec<ContextItem>),
    Section {
        text: Vec<Block>,
        fields: Vec<Vec<Inline>>,
        accessory: Option<Box<Block>>,
    },
    Actions(Vec<Control>),
    Table {
        header: bool,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Card(Card),
    Call(CallCard),
    Unsupported {
        fallback: Arc<str>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RichText {
    pub blocks: Arc<[Block]>,
}

impl RichText {
    pub fn plain(text: &str) -> RichText {
        let inline = Inline::Text {
            text: text.into(),
            style: Style::default(),
        };
        RichText {
            blocks: Arc::from(vec![Block::Paragraph(vec![inline])]),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Labels for mentions as `@label`, `#label`, emoji as unicode or
    /// `:shortcode:`, blocks separated by newlines, controls omitted.
    pub fn plain_text(&self) -> String {
        let mut parts = Vec::new();
        for b in self.blocks.iter() {
            block_plain(b, &mut parts);
        }
        parts.join("\n")
    }
}

fn inlines_plain(items: &[Inline]) -> String {
    let mut out = String::new();
    for i in items {
        match i {
            Inline::Text { text, .. } => out.push_str(text),
            Inline::Code(t) => out.push_str(t),
            Inline::Link { url, label, .. } => out.push_str(label.as_deref().unwrap_or(url)),
            Inline::User { id, label } => {
                out.push('@');
                out.push_str(label.as_deref().unwrap_or(&id.0));
            }
            Inline::Conversation { id, label } => {
                out.push('#');
                out.push_str(label.as_deref().unwrap_or(&id.0));
            }
            Inline::Group { id, label } => {
                let l = label.as_deref().unwrap_or(id);
                if !l.starts_with('@') {
                    out.push('@');
                }
                out.push_str(l);
            }
            Inline::Broadcast(b) => out.push_str(match b {
                Broadcast::Here => "@here",
                Broadcast::Channel => "@channel",
                Broadcast::Everyone => "@everyone",
            }),
            Inline::Emoji(e) => match &e.unicode {
                Some(u) => out.push_str(u),
                None => {
                    out.push(':');
                    out.push_str(&e.shortcode);
                    out.push(':');
                }
            },
            Inline::Date { fallback, .. } => out.push_str(fallback),
            Inline::LineBreak => out.push('\n'),
        }
    }
    out
}

fn block_plain(b: &Block, out: &mut Vec<String>) {
    match b {
        Block::Paragraph(i) | Block::Heading(i) => out.push(inlines_plain(i)),
        Block::Quote(bs) => bs.iter().for_each(|b| block_plain(b, out)),
        Block::Code { text, .. } => out.push(text.to_string()),
        Block::List { items, .. } => items.iter().for_each(|i| out.push(inlines_plain(i))),
        Block::Divider | Block::Actions(_) => {}
        Block::Image { alt, .. } => out.push(alt.to_string()),
        Block::Context(items) => {
            for i in items {
                match i {
                    ContextItem::Text(t) => out.push(inlines_plain(t)),
                    ContextItem::Image { alt, .. } => out.push(alt.to_string()),
                }
            }
        }
        Block::Section {
            text,
            fields,
            accessory,
        } => {
            text.iter().for_each(|b| block_plain(b, out));
            fields.iter().for_each(|f| out.push(inlines_plain(f)));
            if let Some(a) = accessory {
                block_plain(a, out);
            }
        }
        Block::Table { rows, .. } => {
            for r in rows {
                out.push(
                    r.iter()
                        .map(|c| inlines_plain(c))
                        .collect::<Vec<_>>()
                        .join("\t"),
                );
            }
        }
        Block::Card(c) => {
            if let Some(a) = &c.author {
                out.push(inlines_plain(a));
            }
            if let Some(t) = &c.title {
                out.push(inlines_plain(std::slice::from_ref(t)));
            }
            c.body.iter().for_each(|b| block_plain(b, out));
            for (k, v) in &c.fields {
                out.push(format!("{k}: {}", inlines_plain(v)));
            }
            if let Some(f) = &c.footer {
                out.push(inlines_plain(f));
            }
        }
        Block::Call(c) => out.push(match &c.kind {
            CallKind::Huddle => "Huddle".to_string(),
            CallKind::External { provider } => format!("Call: {provider}"),
        }),
        Block::Unsupported { fallback } => out.push(fallback.to_string()),
    }
}
