//! Composer formatting: the pure text edits behind the toolbar and its shortcuts.
//!
//! An edit is a list of small insertions and deletions rather than one
//! replacement, so the mentions and emoji inside a formatted selection keep
//! their spans.

use std::ops::Range;

/// A formatting command of the composer (links are a separate flow).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bold,
    Italic,
    Strike,
    Code,
    CodeBlock,
    Quote,
    BulletList,
    NumberedList,
}

impl Format {
    pub const ALL: [Format; 8] = [
        Format::Bold,
        Format::Italic,
        Format::Strike,
        Format::Code,
        Format::CodeBlock,
        Format::Quote,
        Format::BulletList,
        Format::NumberedList,
    ];

    fn marker(self) -> Option<&'static str> {
        match self {
            Self::Bold => Some("*"),
            Self::Italic => Some("_"),
            Self::Strike => Some("~"),
            Self::Code => Some("`"),
            _ => None,
        }
    }

    fn prefix(self) -> Option<&'static str> {
        match self {
            Self::Quote => Some("> "),
            Self::BulletList => Some("\u{2022} "),
            Self::NumberedList => Some("1. "), // the first line; see `prefix_for`
            _ => None,
        }
    }
}

/// Insertions and deletions to apply to the text, then the selection to leave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatEdit {
    /// `(range, replacement)` in the coordinates of the original text, ordered from
    /// the end of the text to its start so each can be applied in turn.
    pub edits: Vec<(Range<usize>, String)>,
    /// The selection afterwards, in the coordinates of the new text.
    pub select: Range<usize>,
}

impl FormatEdit {
    /// The text after the edits (what the input ends up with).
    pub fn apply_to(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (r, s) in &self.edits {
            out.replace_range(r.clone(), s);
        }
        out
    }
}

/// The edit that applies `f` to `text` with `sel` selected (an empty `sel` is the caret).
///
/// Inline styles wrap the selection in their plain marker, or unwrap it when it is
/// already wrapped; with no selection they insert the empty pair with the caret inside.
/// Quote and list prefix every line the selection touches, or remove the prefix
/// when they all have it.
pub fn format_edit(f: Format, text: &str, sel: Range<usize>) -> FormatEdit {
    if let Some(m) = f.marker() {
        return wrap(text, sel, m, m, true);
    }
    if f.prefix().is_some() {
        return prefix_lines(text, sel, f);
    }
    wrap(text, sel, "```\n", "\n```", false)
}

fn wrap(text: &str, sel: Range<usize>, open: &str, close: &str, toggle: bool) -> FormatEdit {
    let (o, c) = (open.len(), close.len());
    let inner = &text[sel.clone()];
    if toggle && !sel.is_empty() {
        // Markers inside the selection: `*word*` selected.
        if inner.len() >= o + c && inner.starts_with(open) && inner.ends_with(close) {
            return FormatEdit {
                edits: vec![
                    (sel.end - c..sel.end, String::new()),
                    (sel.start..sel.start + o, String::new()),
                ],
                select: sel.start..sel.end - o - c,
            };
        }
        // Markers just outside it: `*` `word` `*`.
        if sel.start >= o && text[..sel.start].ends_with(open) && text[sel.end..].starts_with(close)
        {
            return FormatEdit {
                edits: vec![
                    (sel.end..sel.end + c, String::new()),
                    (sel.start - o..sel.start, String::new()),
                ],
                select: sel.start - o..sel.end - o,
            };
        }
    }
    FormatEdit {
        edits: vec![
            (sel.end..sel.end, close.to_string()),
            (sel.start..sel.start, open.to_string()),
        ],
        select: sel.start + o..sel.end + o,
    }
}

/// Length of the `f` prefix `line` starts with (0 when it has none): `> `, `\u{2022} `, or
/// any `N. ` for a numbered list.
fn prefix_len(f: Format, line: &str) -> usize {
    match f {
        Format::NumberedList => {
            let digits = line.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 && line[digits..].starts_with(". ") {
                digits + 2
            } else {
                0
            }
        }
        _ => f
            .prefix()
            .filter(|p| line.starts_with(p))
            .map_or(0, str::len),
    }
}

/// The prefix `f` adds to line number `k` (0 based): numbered lists count up.
fn prefix_for(f: Format, k: usize) -> String {
    match f {
        Format::NumberedList => format!("{}. ", k + 1),
        _ => f.prefix().unwrap_or_default().to_string(),
    }
}

fn prefix_lines(text: &str, sel: Range<usize>, f: Format) -> FormatEdit {
    let first = text[..sel.start].rfind('\n').map_or(0, |i| i + 1);
    // A line break right after a non-empty selection does not pull the next line in.
    let probe = if sel.end > sel.start && text[..sel.end].ends_with('\n') {
        sel.end - 1
    } else {
        sel.end
    };
    let last = text[probe..].find('\n').map_or(text.len(), |i| probe + i);
    let mut starts = vec![first];
    starts.extend(
        text[first..last]
            .match_indices('\n')
            .map(|(i, _)| first + i + 1),
    );
    let lens: Vec<usize> = starts.iter().map(|&s| prefix_len(f, &text[s..])).collect();
    let all = lens.iter().all(|&l| l > 0);
    let adds: Vec<String> = (0..starts.len()).map(|k| prefix_for(f, k)).collect();
    let edits = starts
        .iter()
        .zip(lens.iter().zip(&adds))
        .rev()
        .map(|(&s, (&l, add))| {
            if all {
                (s..s + l, String::new())
            } else {
                (s..s, add.clone())
            }
        })
        .collect();
    let removed: usize = lens.iter().sum();
    let added: usize = adds.iter().map(String::len).sum();
    let select = if sel.is_empty() {
        let at = if all {
            sel.start.saturating_sub(lens[0]).max(first)
        } else {
            sel.start + adds[0].len()
        };
        at..at
    } else if all {
        first..last - removed
    } else {
        first..last + added
    };
    FormatEdit { edits, select }
}
