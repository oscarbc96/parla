//! The text of a [`super::TextInput`]: a string, a selection and the IME marked range,
//! with every offset in bytes on char boundaries. The platform speaks UTF-16; the
//! conversions live here so they stay testable without a window.

use std::ops::Range;

use domain::{ComposedSpan, Inline};
use unicode_segmentation::UnicodeSegmentation;

/// UTF-16 code units to a UTF-8 byte offset, snapping inside a character to its end.
pub(crate) fn utf16_to_utf8_offset(text: &str, utf16: usize) -> usize {
    let mut consumed = 0;
    for (byte, ch) in text.char_indices() {
        if consumed >= utf16 {
            return byte;
        }
        consumed += ch.len_utf16();
    }
    text.len()
}

/// UTF-8 bytes to a UTF-16 code-unit offset, snapping inside a character to its end.
pub(crate) fn utf8_to_utf16_offset(text: &str, utf8: usize) -> usize {
    text.char_indices()
        .take_while(|(byte, _)| *byte < utf8)
        .map(|(_, ch)| ch.len_utf16())
        .sum()
}

/// A selection in bytes. `anchor` stays put while `head` (the caret) moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The selected bytes in text order.
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Buffer {
    text: String,
    sel: Selection,
    /// The IME composition (underlined text the platform may still replace).
    marked: Option<Range<usize>>,
    /// Picked mentions and custom emoji: atomic ranges of `text`, sorted, never overlapping.
    spans: Vec<ComposedSpan>,
}

impl Buffer {
    pub fn spans(&self) -> &[ComposedSpan] {
        &self.spans
    }

    /// Replaces `range` with `label` and makes it one atomic span.
    pub fn insert_span(&mut self, range: Range<usize>, label: &str, inline: Inline) {
        self.replace(range, label);
        let end = self.sel.head;
        let start = end - label.len();
        let at = self.spans.partition_point(|s| s.range.start < start);
        self.spans.insert(
            at,
            ComposedSpan {
                range: start..end,
                inline,
            },
        );
    }

    /// Widens `range` over every span it cuts into (removing those), then shifts the spans
    /// after it for a replacement of `new_len` bytes. Returns the range to replace.
    fn absorb(&mut self, range: Range<usize>, new_len: usize) -> Range<usize> {
        let (mut start, mut end) = (range.start, range.end);
        loop {
            let hit = self
                .spans
                .iter()
                .position(|s| s.range.start < end && start < s.range.end);
            let Some(i) = hit else { break };
            let s = self.spans.remove(i);
            start = start.min(s.range.start);
            end = end.max(s.range.end);
        }
        for s in &mut self.spans {
            if s.range.start >= end {
                s.range = (s.range.start + new_len - (end - start))
                    ..(s.range.end + new_len - (end - start));
            }
        }
        start..end
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Selection {
        self.sel
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Replaces everything and puts the caret at the end.
    pub fn set_text(&mut self, s: &str) {
        self.text = s.to_string();
        self.sel = Selection::caret(self.text.len());
        self.marked = None;
        self.spans.clear();
    }

    /// Like [`set_text`](Self::set_text), keeping the spans that fit `s` (sorted, no overlap).
    pub fn set_text_with_spans(&mut self, s: &str, spans: &[ComposedSpan]) {
        self.set_text(s);
        let mut end = 0;
        for sp in spans {
            let r = &sp.range;
            if r.start >= end
                && r.start < r.end
                && r.end <= s.len()
                && s.is_char_boundary(r.start)
                && s.is_char_boundary(r.end)
            {
                self.spans.push(sp.clone());
                end = r.end;
            }
        }
    }

    pub fn restore(&mut self, text: String, sel: Selection, spans: Vec<ComposedSpan>) {
        self.text = text;
        self.spans = spans;
        self.sel = Selection {
            anchor: self.snap(sel.anchor),
            head: self.snap(sel.head),
        };
        self.marked = None;
    }

    /// Clamps `i` into the text and moves it back onto a char boundary.
    pub fn snap(&self, i: usize) -> usize {
        let mut i = i.min(self.text.len());
        while !self.text.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        self.sel = Selection {
            anchor: self.snap(anchor),
            head: self.snap(head),
        };
    }

    pub fn utf16_to_byte(&self, u: usize) -> usize {
        utf16_to_utf8_offset(&self.text, u)
    }

    pub fn byte_to_utf16(&self, b: usize) -> usize {
        utf8_to_utf16_offset(&self.text, self.snap(b))
    }

    pub fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        let (a, b) = (self.utf16_to_byte(r.start), self.utf16_to_byte(r.end));
        a.min(b)..a.max(b)
    }

    pub fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.byte_to_utf16(r.start)..self.byte_to_utf16(r.end)
    }

    /// Replaces `range` and puts the caret after the new text. Ends any composition.
    pub fn replace(&mut self, range: Range<usize>, new: &str) {
        let range = self.snap(range.start)..self.snap(range.end);
        let range = self.absorb(range, new.len());
        self.text.replace_range(range.clone(), new);
        self.sel = Selection::caret(range.start + new.len());
        self.marked = None;
    }

    /// Replaces `range` with composition text. `select` is relative to the new text.
    pub fn replace_and_mark(
        &mut self,
        range: Range<usize>,
        new: &str,
        select: Option<Range<usize>>,
    ) {
        let range = self.snap(range.start)..self.snap(range.end);
        let range = self.absorb(range, new.len());
        self.text.replace_range(range.clone(), new);
        let start = range.start;
        self.marked = (!new.is_empty()).then(|| start..start + new.len());
        self.sel = match select {
            Some(r) => {
                let end = new.len();
                let (a, b) = (r.start.min(end), r.end.min(end));
                let (a, b) = (start + a, start + b);
                let (a, b) = (self.snap(a), self.snap(b));
                Selection { anchor: a, head: b }
            }
            None => Selection::caret(start + new.len()),
        };
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    pub fn prev_boundary(&self, i: usize) -> usize {
        self.text[..self.snap(i)]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(b, _)| b)
    }

    pub fn next_boundary(&self, i: usize) -> usize {
        let i = self.snap(i);
        self.text[i..]
            .graphemes(true)
            .next()
            .map_or(self.text.len(), |g| i + g.len())
    }

    /// Start of the word before `i` (skipping spaces and punctuation).
    pub fn word_left(&self, i: usize) -> usize {
        let i = self.snap(i);
        self.text
            .split_word_bound_indices()
            .take_while(|(b, _)| *b < i)
            .filter(|(_, w)| is_wordy(w))
            .last()
            .map_or(0, |(b, _)| b)
    }

    /// End of the word after `i` (skipping spaces and punctuation).
    pub fn word_right(&self, i: usize) -> usize {
        let i = self.snap(i);
        self.text
            .split_word_bound_indices()
            .find(|(b, w)| is_wordy(w) && b + w.len() > i)
            .map_or(self.text.len(), |(b, w)| b + w.len())
    }

    /// Start of the hard line (after the previous `\n`) holding `i`.
    pub fn line_start(&self, i: usize) -> usize {
        let i = self.snap(i);
        self.text[..i].rfind('\n').map_or(0, |n| n + 1)
    }

    /// End of the hard line holding `i`, before its `\n`.
    pub fn line_end(&self, i: usize) -> usize {
        let i = self.snap(i);
        self.text[i..].find('\n').map_or(self.text.len(), |n| i + n)
    }
}

fn is_wordy(s: &str) -> bool {
    s.chars().any(char::is_alphanumeric)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_skip_spaces_and_punctuation() {
        let mut b = Buffer::default();
        b.set_text("hello, big  world");
        assert_eq!(b.word_left(17), 12);
        assert_eq!(b.word_left(12), 7);
        assert_eq!(b.word_left(7), 0);
        assert_eq!(b.word_right(0), 5);
        assert_eq!(b.word_right(5), 10);
        assert_eq!(b.word_right(17), 17);
    }

    #[test]
    fn graphemes_move_over_emoji_and_combining_marks() {
        let mut b = Buffer::default();
        b.set_text("a👍🏽e\u{301}");
        assert_eq!(b.next_boundary(1), 1 + "👍🏽".len());
        assert_eq!(b.prev_boundary(b.text().len()), 1 + "👍🏽".len());
    }

    #[test]
    fn utf16_offsets_match_encode_utf16() {
        for text in ["ASCII only", "café e\u{301}", "a👍🏽b", "日本語"] {
            let mut utf16 = 0;
            for (byte, ch) in text.char_indices() {
                assert_eq!(utf8_to_utf16_offset(text, byte), utf16, "{text:?}");
                assert_eq!(utf16_to_utf8_offset(text, utf16), byte, "{text:?}");
                utf16 += ch.len_utf16();
            }
            assert_eq!(utf16, text.encode_utf16().count(), "{text:?}");
            assert_eq!(utf8_to_utf16_offset(text, text.len()), utf16, "{text:?}");
            assert_eq!(utf16_to_utf8_offset(text, utf16), text.len(), "{text:?}");
            assert_eq!(utf8_to_utf16_offset(text, text.len() + 1), utf16);
            assert_eq!(utf16_to_utf8_offset(text, utf16 + 1), text.len());
        }
    }

    #[test]
    fn utf16_offsets_never_split_a_character() {
        let text = "A😀B";
        assert_eq!(utf16_to_utf8_offset(text, 2), 5);
        assert_eq!(utf8_to_utf16_offset(text, 2), 3);
        assert!(text.is_char_boundary(utf16_to_utf8_offset(text, 2)));
    }

    #[test]
    fn utf16_round_trip_around_emoji() {
        let mut b = Buffer::default();
        b.set_text("a👍日");
        assert_eq!(b.byte_to_utf16(5), 3);
        assert_eq!(b.utf16_to_byte(3), 5);
        assert_eq!(b.range_from_utf16(&(1..3)), 1..5);
        assert_eq!(b.range_to_utf16(&(1..5)), 1..3);
    }

    #[test]
    fn marking_tracks_the_replaced_text() {
        let mut b = Buffer::default();
        b.set_text("ab");
        b.replace_and_mark(2..2, "にほ", None);
        assert_eq!(b.marked(), Some(2..8));
        b.replace_and_mark(2..8, "に", Some(0..3));
        assert_eq!(b.marked(), Some(2..5));
        assert_eq!(b.selection(), Selection { anchor: 2, head: 5 });
        b.replace(2..5, "日");
        assert_eq!(b.text(), "ab日");
        assert_eq!(b.marked(), None);
    }

    fn user(id: &str) -> Inline {
        Inline::User {
            id: domain::UserId(id.into()),
            label: None,
        }
    }

    #[test]
    fn spans_shift_with_edits_before_them() {
        let mut b = Buffer::default();
        b.set_text("hi  x");
        b.insert_span(3..3, "@Ana", user("U1"));
        assert_eq!(b.text(), "hi @Ana x");
        assert_eq!(b.spans()[0].range, 3..7);
        b.replace(0..0, "oh ");
        assert_eq!(b.spans()[0].range, 6..10);
        b.replace(10..10, "!");
        assert_eq!(b.spans()[0].range, 6..10);
    }

    #[test]
    fn a_cut_into_a_span_removes_all_of_it() {
        let mut b = Buffer::default();
        b.set_text("a ");
        b.insert_span(2..2, "@Ana Lopez", user("U1"));
        let end = b.text().len();
        b.replace(end - 1..end, "");
        assert_eq!(b.text(), "a ");
        assert!(b.spans().is_empty());
        assert_eq!(b.selection(), Selection::caret(2));
    }

    #[test]
    fn typing_at_a_span_edge_keeps_it() {
        let mut b = Buffer::default();
        b.set_text("");
        b.insert_span(0..0, "@Ana", user("U1"));
        b.replace(4..4, " x");
        b.replace(0..0, "y ");
        assert_eq!(b.spans().len(), 1);
        assert_eq!(b.spans()[0].range, 2..6);
    }
}
