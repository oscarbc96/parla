//! Block structure and inline formatting for Slack mrkdwn.
//!
//! Inline parsing is two passes, both linear: tokenise the line (code spans,
//! angle tokens and emoji become opaque atoms, `*_~` become delimiter
//! candidates), then resolve delimiters with a small stack.

use domain::{Block, Inline, Mention, RichText};

use crate::entities::decode;

mod blocks;
mod inline;
mod tokenize;

use blocks::blocks_of;

pub fn parse(text: &str) -> RichText {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("```") {
        let after = &rest[open + 3..];
        let Some(close) = after.find("```") else {
            break;
        };
        blocks_of(&rest[..open], &mut out);
        let mut code = &after[..close];
        code = code.strip_prefix('\n').unwrap_or(code);
        code = code.strip_suffix('\n').unwrap_or(code);
        out.push(Block::Code {
            lang: None,
            text: decode(code).into(),
        });
        rest = &after[close + 3..];
    }
    blocks_of(rest, &mut out);
    RichText { blocks: out.into() }
}

pub fn mentions(r: &RichText) -> Vec<Mention> {
    fn visit(items: &[Inline], out: &mut Vec<Mention>) {
        for i in items {
            let m = match i {
                Inline::User { id, .. } => Mention::User(id.clone()),
                Inline::Group { id, .. } => Mention::Group(id.clone()),
                Inline::Broadcast(b) => Mention::Broadcast(*b),
                _ => continue,
            };
            if !out.contains(&m) {
                out.push(m);
            }
        }
    }
    fn walk(b: &Block, out: &mut Vec<Mention>) {
        match b {
            Block::Paragraph(i) | Block::Heading(i) => visit(i, out),
            Block::Quote(bs) => bs.iter().for_each(|b| walk(b, out)),
            Block::List { items, .. } => items.iter().for_each(|i| visit(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    r.blocks.iter().for_each(|b| walk(b, &mut out));
    out
}
