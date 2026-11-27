//! Second inline pass: resolve delimiters into styled inlines.

use std::sync::Arc;

use domain::{Inline, Style};

use super::tokenize::{Tok, tokenize};
use crate::entities::decode;

pub(super) fn text(s: &str) -> Inline {
    Inline::Text {
        text: s.into(),
        style: Style::default(),
    }
}

struct Frame {
    ch: char,
    children: Vec<Inline>,
}

fn slot(ch: char) -> usize {
    match ch {
        '*' => 0,
        '_' => 1,
        _ => 2,
    }
}

fn styled(i: Inline, ch: char) -> Inline {
    let set = |s: &mut Style| match ch {
        '*' => s.bold = true,
        '_' => s.italic = true,
        _ => s.strike = true,
    };
    match i {
        Inline::Text { text, mut style } => {
            set(&mut style);
            Inline::Text { text, style }
        }
        Inline::Link {
            url,
            label,
            mut style,
        } => {
            set(&mut style);
            Inline::Link { url, label, style }
        }
        o => o,
    }
}

pub(super) fn inline(line: &str) -> Vec<Inline> {
    let mut frames = vec![Frame {
        ch: '\0',
        children: Vec::new(),
    }];
    let mut present = [false; 3];
    // Fold every frame above `keep` into its parent, markers back as text.
    fn collapse(frames: &mut Vec<Frame>, present: &mut [bool; 3], keep: usize) {
        while frames.len() > keep + 1 {
            let f = frames.pop().expect("len checked");
            present[slot(f.ch)] = false;
            let parent = frames.last_mut().expect("base frame");
            parent.children.push(text(&f.ch.to_string()));
            parent.children.extend(f.children);
        }
    }
    for tok in tokenize(line) {
        match tok {
            Tok::Text(s) => frames
                .last_mut()
                .expect("base")
                .children
                .push(text(&decode(s))),
            Tok::Atom(a) => frames.last_mut().expect("base").children.push(a),
            Tok::Mark { ch, open, close } => {
                if close && present[slot(ch)] {
                    let idx = frames.iter().rposition(|f| f.ch == ch).expect("present");
                    collapse(&mut frames, &mut present, idx);
                    if !frames[idx].children.is_empty() {
                        let f = frames.pop().expect("idx frame");
                        present[slot(ch)] = false;
                        let parent = frames.last_mut().expect("base");
                        parent
                            .children
                            .extend(f.children.into_iter().map(|c| styled(c, ch)));
                        continue;
                    }
                    frames
                        .last_mut()
                        .expect("base")
                        .children
                        .push(text(&ch.to_string()));
                } else if open && !present[slot(ch)] {
                    present[slot(ch)] = true;
                    frames.push(Frame {
                        ch,
                        children: Vec::new(),
                    });
                } else {
                    frames
                        .last_mut()
                        .expect("base")
                        .children
                        .push(text(&ch.to_string()));
                }
            }
        }
    }
    collapse(&mut frames, &mut present, 0);
    merge(frames.pop().expect("base").children)
}

fn merge(items: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(items.len());
    let mut pending: Option<(String, Style)> = None;
    let flush = |p: &mut Option<(String, Style)>, out: &mut Vec<Inline>| {
        if let Some((t, style)) = p.take() {
            out.push(Inline::Text {
                text: Arc::from(t),
                style,
            });
        }
    };
    for i in items {
        match i {
            Inline::Text { text, style } => match &mut pending {
                Some((buf, s)) if *s == style => buf.push_str(&text),
                _ => {
                    flush(&mut pending, &mut out);
                    pending = Some((text.to_string(), style));
                }
            },
            other => {
                flush(&mut pending, &mut out);
                out.push(other);
            }
        }
    }
    flush(&mut pending, &mut out);
    out
}
