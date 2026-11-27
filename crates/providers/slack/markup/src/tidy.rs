//! Bots pad their text with blank lines; keep at most one blank line between
//! paragraphs and none at the edges of a block.

use domain::{Block, Card, ContextItem, Inline};

/// Lines of one text run: trailing spaces before a newline go, and three or
/// more newlines in a row become two.
fn squeeze(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut nl = 0;
    let mut pending = String::new();
    for c in t.chars() {
        if c == '\n' {
            pending.clear();
            nl += 1;
            if nl <= 2 {
                out.push('\n');
            }
        } else if c == '\r' {
        } else if c == ' ' || c == '\t' {
            pending.push(c);
        } else {
            out.push_str(&pending);
            pending.clear();
            out.push(c);
            nl = 0;
        }
    }
    out.push_str(&pending);
    out
}

pub(crate) fn inlines(items: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(items.len());
    // Newlines at the end of what has been emitted so far.
    let mut nl = 0usize;
    for it in items {
        match it {
            Inline::LineBreak => {
                if !out.is_empty() && nl < 2 {
                    out.push(Inline::LineBreak);
                    nl += 1;
                }
            }
            Inline::Text { text, style } => {
                let mut t = squeeze(&text);
                if out.is_empty() {
                    t = t.trim_start().to_string();
                } else {
                    let lead = t.chars().take_while(|c| *c == '\n').count();
                    let keep = lead.min(2usize.saturating_sub(nl));
                    t = format!("{}{}", "\n".repeat(keep), &t[lead..]);
                }
                if t.is_empty() {
                    continue;
                }
                nl = t.chars().rev().take_while(|c| *c == '\n').count()
                    + if t.chars().all(|c| c == '\n') { nl } else { 0 };
                out.push(Inline::Text {
                    text: t.into(),
                    style,
                });
            }
            other => {
                nl = 0;
                out.push(other);
            }
        }
    }
    while let Some(last) = out.last_mut() {
        match last {
            Inline::LineBreak => {
                out.pop();
            }
            Inline::Text { text, style } => {
                let t = text.trim_end();
                if t.is_empty() {
                    out.pop();
                } else {
                    if t.len() != text.len() {
                        *last = Inline::Text {
                            text: t.into(),
                            style: *style,
                        };
                    }
                    break;
                }
            }
            _ => break,
        }
    }
    out
}

fn card(c: &mut Card) {
    c.author = c.author.take().map(inlines).filter(|i| !i.is_empty());
    c.body = blocks(std::mem::take(&mut c.body));
    for f in &mut c.fields {
        f.1 = inlines(std::mem::take(&mut f.1));
    }
    c.footer = c.footer.take().map(inlines).filter(|i| !i.is_empty());
}

pub(crate) fn blocks(items: Vec<Block>) -> Vec<Block> {
    items
        .into_iter()
        .filter_map(|b| {
            Some(match b {
                Block::Paragraph(i) => {
                    let i = inlines(i);
                    if i.is_empty() {
                        return None;
                    }
                    Block::Paragraph(i)
                }
                Block::Heading(i) => Block::Heading(inlines(i)),
                Block::Quote(bs) => Block::Quote(blocks(bs)),
                Block::List {
                    style,
                    indent,
                    items,
                } => Block::List {
                    style,
                    indent,
                    items: items.into_iter().map(inlines).collect(),
                },
                Block::Context(cs) => Block::Context(
                    cs.into_iter()
                        .map(|c| match c {
                            ContextItem::Text(t) => ContextItem::Text(inlines(t)),
                            o => o,
                        })
                        .collect(),
                ),
                Block::Section {
                    text,
                    fields,
                    accessory,
                } => Block::Section {
                    text: blocks(text),
                    fields: fields.into_iter().map(inlines).collect(),
                    accessory,
                },
                Block::Card(mut c) => {
                    card(&mut c);
                    Block::Card(c)
                }
                o => o,
            })
        })
        .collect()
}
