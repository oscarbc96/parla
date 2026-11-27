//! The code viewer's model: which files open
//! in it, the document it shows and find within it. Pure; the window lives in
//! the app. A document keeps the text once and an index of line starts, so a
//! 50,000-line file costs its bytes plus four per line, and the window draws
//! only the lines on screen.

use std::ops::Range;
use std::sync::Arc;

use domain::{FileKind, FileRef};
use gpui::HighlightStyle;

use crate::snippet::{Language, language_for_name};
use crate::snippet_highlight::{Class, highlight_within};

/// Largest file the viewer fetches and shows (8 MB); a larger one downloads.
pub const MAX_CODE_BYTES: u64 = 8 * 1024 * 1024;

/// Find stops counting here; the count then reads "10000+".
pub const MAX_MATCHES: usize = 10_000;

/// Bytes of one line drawn; the rest of a longer line (minified code) is not
/// laid out. Copy still copies all of it.
pub const MAX_DRAWN_LINE_BYTES: usize = 4_096;

/// Whether a click on `f` opens the code viewer: a snippet, or a text or source
/// file no larger than [`MAX_CODE_BYTES`].
pub fn is_code(f: &FileRef) -> bool {
    if f.tombstone || crate::clip::is_clip(f) {
        return false;
    }
    let small = f.size <= MAX_CODE_BYTES;
    match f.kind {
        FileKind::Snippet => small || f.preview.is_some(),
        FileKind::Other => {
            small && (f.mime.starts_with("text/") || language_for_name(&f.name).is_some())
        }
        _ => false,
    }
}

/// The provider's preview of `f` when it is the whole file, so nothing needs
/// fetching.
pub fn whole_preview(f: &FileRef) -> Option<&str> {
    let p = f.preview.as_deref()?;
    // No line count and no size: the preview may be only part of the file.
    let whole = match f.lines {
        Some(total) => p.lines().count() >= total as usize,
        None => f.size > 0 && f.size <= p.len() as u64,
    };
    whole.then_some(p)
}

/// The text of a document, shared without copying: a provider preview, or
/// the fetched bytes decoded in place.
#[derive(Clone, Debug)]
pub enum CodeText {
    Preview(Arc<str>),
    Fetched(Arc<String>),
}

impl std::ops::Deref for CodeText {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            Self::Preview(s) => s,
            Self::Fetched(s) => s,
        }
    }
}

/// A text file split into lines.
#[derive(Clone, Debug)]
pub struct CodeDoc {
    text: CodeText,
    /// Byte offset where each line starts.
    starts: Arc<[u32]>,
    language: Option<&'static Language>,
    longest: usize,
}

impl CodeDoc {
    /// A document of `text`, its language guessed from the file name.
    pub fn new(text: impl Into<Arc<str>>, name: &str) -> Self {
        Self::of(CodeText::Preview(text.into()), name)
    }

    fn of(text: CodeText, name: &str) -> Self {
        let mut starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' && i + 1 < text.len() {
                starts.push((i + 1) as u32);
            }
        }
        let mut doc = Self {
            text,
            starts: starts.into(),
            language: language_for_name(name),
            longest: 0,
        };
        doc.longest = (0..doc.line_count())
            .max_by_key(|&i| (doc.line(i).len(), std::cmp::Reverse(i)))
            .unwrap_or(0);
        doc
    }

    /// The document of fetched `bytes`, or `None` when they are not text (a
    /// NUL byte in the first 8 KB). Invalid UTF-8 is replaced.
    /// The buffer is decoded in place: valid UTF-8 is kept as is, so the
    /// document costs about the file's size.
    pub fn from_bytes(bytes: Vec<u8>, name: &str) -> Option<Self> {
        if bytes[..bytes.len().min(8 * 1024)].contains(&0) {
            return None;
        }
        let text = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };
        Some(Self::of(CodeText::Fetched(Arc::new(text)), name))
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// Line `i` without its line end; empty past the last line.
    pub fn line(&self, i: usize) -> &str {
        let Some(&start) = self.starts.get(i) else {
            return "";
        };
        let end = self
            .starts
            .get(i + 1)
            .map_or(self.text.len(), |&e| e as usize);
        let line = &self.text[start as usize..end];
        let line = line.strip_suffix('\n').unwrap_or(line);
        line.strip_suffix('\r').unwrap_or(line)
    }

    /// Line `i` as drawn: cut to [`MAX_DRAWN_LINE_BYTES`] on a character boundary.
    pub fn drawn_line(&self, i: usize) -> &str {
        let line = self.line(i);
        let mut end = line.len().min(MAX_DRAWN_LINE_BYTES);
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        &line[..end]
    }

    /// The whole text, as fetched (Copy puts this on the clipboard).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The shared text (a reference count, no copy).
    pub fn shared_text(&self) -> CodeText {
        self.text.clone()
    }

    pub fn language(&self) -> Option<&'static Language> {
        self.language
    }

    /// The index of the longest line (sizes the unwrapped view's width).
    pub fn longest_line(&self) -> usize {
        self.longest
    }

    /// The syntax spans of drawn line `i`.
    pub fn spans(&self, i: usize) -> Vec<(Range<usize>, Class)> {
        self.language.map_or_else(Vec::new, |l| {
            highlight_within(l, self.drawn_line(i), MAX_DRAWN_LINE_BYTES)
        })
    }
}

/// One find match: a byte range of a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub range: Range<usize>,
}

/// Find within a document: every match of the query (ASCII case ignored), in
/// order, and the current one.
#[derive(Clone, Debug, Default)]
pub struct Find {
    query: String,
    matches: Vec<Match>,
    current: Option<usize>,
    capped: bool,
}

impl Find {
    /// Finds `query` in `doc`; the current match is the first.
    pub fn search(&mut self, doc: &CodeDoc, query: &str) {
        self.search_from(doc, query, 0);
    }

    /// Finds `query` in `doc`; the current match is the first at or after
    /// `line` (the first line on screen), else the first.
    /// The scan starts at `line` and wraps, so when more than [`MAX_MATCHES`]
    /// exist the ones counted are those from the view on.
    pub fn search_from(&mut self, doc: &CodeDoc, query: &str, line: usize) {
        self.query = query.to_string();
        self.matches.clear();
        self.capped = false;
        self.current = None;
        if query.is_empty() {
            return;
        }
        let n = doc.line_count();
        let line = if line < n { line } else { 0 };
        'lines: for i in (line..n).chain(0..line) {
            for range in find_in(doc.line(i), query) {
                if self.matches.len() == MAX_MATCHES {
                    self.capped = true;
                    break 'lines;
                }
                self.matches.push(Match { line: i, range });
            }
        }
        // Back in document order; the current match is the first from `line`.
        let wrapped = self.matches.partition_point(|m| m.line >= line);
        self.matches.rotate_left(wrapped);
        if !self.matches.is_empty() {
            let at = self.matches.len() - wrapped;
            self.current = Some(if at < self.matches.len() { at } else { 0 });
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn len(&self) -> usize {
        self.matches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// More matches exist than were counted ([`MAX_MATCHES`]).
    pub fn capped(&self) -> bool {
        self.capped
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn current(&self) -> Option<&Match> {
        self.matches.get(self.current?)
    }

    /// Steps to the next match, wrapping to the first.
    pub fn step_next(&mut self) -> Option<&Match> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(self.current.map_or(0, |c| (c + 1) % n));
        self.current()
    }

    /// Steps to the previous match, wrapping to the last.
    pub fn step_prev(&mut self) -> Option<&Match> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        self.current = Some(self.current.map_or(n - 1, |c| (c + n - 1) % n));
        self.current()
    }

    /// The matches on `line`, in order.
    pub fn on_line(&self, line: usize) -> &[Match] {
        let a = self.matches.partition_point(|m| m.line < line);
        let b = self.matches.partition_point(|m| m.line <= line);
        &self.matches[a..b]
    }

    /// Drops the query and its matches.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Byte ranges of `needle` in `hay`, not overlapping: ASCII case ignored when
/// `needle` is ASCII (a match then starts and ends on character boundaries),
/// else exact.
fn find_in(hay: &str, needle: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    if needle.is_ascii() {
        let (h, n) = (hay.as_bytes(), needle.as_bytes());
        let mut i = 0;
        while i + n.len() <= h.len() {
            if h[i..i + n.len()].eq_ignore_ascii_case(n) {
                out.push(i..i + n.len());
                i += n.len();
            } else {
                i += 1;
            }
        }
    } else {
        out.extend(hay.match_indices(needle).map(|(i, m)| i..i + m.len()));
    }
    out
}

/// `base` styles (syntax colours) with `over` styles (find marks) laid on top,
/// split so no two ranges overlap, in order. Both inputs are in order and
/// non-overlapping within themselves.
pub fn merge_styles(
    base: &[(Range<usize>, HighlightStyle)],
    over: &[(Range<usize>, HighlightStyle)],
) -> Vec<(Range<usize>, HighlightStyle)> {
    if over.is_empty() {
        return base.to_vec();
    }
    let mut cuts: Vec<usize> = base
        .iter()
        .chain(over)
        .flat_map(|(r, _)| [r.start, r.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let covering = |list: &[(Range<usize>, HighlightStyle)], a: usize| {
        list.iter()
            .find(|(r, _)| r.start <= a && a < r.end)
            .map(|(_, s)| *s)
    };
    let mut out: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let style = match (covering(base, a), covering(over, a)) {
            (None, None) => continue,
            (Some(s), None) | (None, Some(s)) => s,
            (Some(s), Some(o)) => s.highlight(o),
        };
        out.push((a..b, style));
    }
    out
}
