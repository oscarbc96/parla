//! A multi-line text input for the composer: IME composition (marked text), selection by
//! keyboard and mouse, word moves, clipboard, undo and redo, soft wrap. Written against
//! GPUI's `EntityInputHandler`; the layout (carets, rows, hit testing) comes from GPUI's
//! text system, the text model lives in [`Buffer`].

mod buffer;
mod element;
mod history;

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, IntoElement, KeyBinding, Pixels, Point, Render, SharedString, UTF16Selection,
    Window, actions, div, prelude::*, px,
};
use themes::ActiveTheme;

use crate::keys::binding;

use buffer::utf16_to_utf8_offset;
pub use buffer::{Buffer, Selection};
use element::{Laid, TextElement};
use history::{History, Snapshot};

actions!(
    text_input,
    [
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        LineStart,
        LineEnd,
        DocStart,
        DocEnd,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectWordLeft,
        SelectWordRight,
        SelectLineStart,
        SelectLineEnd,
        SelectDocStart,
        SelectDocEnd,
        SelectAll,
        Backspace,
        Delete,
        DeleteWordLeft,
        Cut,
        Copy,
        Paste,
        Undo,
        Redo
    ]
);

/// Key context of the input (the composer embeds one).
pub const KEY_CONTEXT: &str = "TextInput";

/// Registers the default key bindings (the app loads the same ones from its keymap files).
pub fn init(cx: &mut App) {
    let c = Some(KEY_CONTEXT);
    let mut keys = vec![
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
    ];
    // macOS and the other platforms differ only in the modifier names.
    for (word, line, doc, cmd) in [
        ("alt", "cmd", "cmd", "cmd"),
        ("ctrl", "none", "ctrl", "ctrl"),
    ] {
        keys.extend([
            binding(&format!("{word}-left"), WordLeft, c, cx),
            binding(&format!("{word}-right"), WordRight, c, cx),
            binding(&format!("{word}-shift-left"), SelectWordLeft, c, cx),
            binding(&format!("{word}-shift-right"), SelectWordRight, c, cx),
            binding(&format!("{word}-backspace"), DeleteWordLeft, c, cx),
            binding(&format!("{cmd}-a"), SelectAll, c, cx),
            binding(&format!("{cmd}-x"), Cut, c, cx),
            binding(&format!("{cmd}-c"), Copy, c, cx),
            binding(&format!("{cmd}-z"), Undo, c, cx),
        ]);
        if line != "none" {
            keys.extend([
                binding(&format!("{line}-left"), LineStart, c, cx),
                binding(&format!("{line}-right"), LineEnd, c, cx),
                binding(&format!("{line}-shift-left"), SelectLineStart, c, cx),
                binding(&format!("{line}-shift-right"), SelectLineEnd, c, cx),
            ]);
        }
        keys.extend([
            binding(&format!("{doc}-up"), DocStart, c, cx),
            binding(&format!("{doc}-down"), DocEnd, c, cx),
            binding(&format!("{doc}-shift-up"), SelectDocStart, c, cx),
            binding(&format!("{doc}-shift-down"), SelectDocEnd, c, cx),
        ]);
    }
    keys.extend([
        KeyBinding::new("cmd-shift-z", Redo, c),
        KeyBinding::new("ctrl-shift-z", Redo, c),
        KeyBinding::new("ctrl-y", Redo, c),
        KeyBinding::new("home", LineStart, c),
        KeyBinding::new("end", LineEnd, c),
        KeyBinding::new("shift-home", SelectLineStart, c),
        KeyBinding::new("shift-end", SelectLineEnd, c),
    ]);
    cx.bind_keys(keys);
}

/// Emitted whenever the text changes (not for caret moves).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    Changed,
}

/// Rows kept visible before the input scrolls.
const MAX_ROWS: f32 = 10.0;

/// Resolves the image to draw in place of a span's text (a custom emoji).
pub type SpanImages = Rc<dyn Fn(&domain::Inline, &App) -> Option<domain::ImageRef>>;

pub struct TextInput {
    buf: Buffer,
    history: History,
    focus: FocusHandle,
    placeholder: SharedString,
    /// The placeholder's colour; `None` is the theme's muted text.
    placeholder_color: Option<gpui::Hsla>,
    /// The last painted layout, for hit testing and vertical moves.
    laid: Option<Laid>,
    /// Horizontal target kept across consecutive Up/Down moves.
    vertical_x: Option<Pixels>,
    /// The byte where End left the caret at the end of a soft-wrapped row: it
    /// is the same offset as the start of the next row, so the caret keeps
    /// its row only while the head is still there.
    upstream_at: Option<usize>,
    scroll_y: Pixels,
    selecting: bool,
    /// The host shows a popup that owns Down: the input leaves that key to it.
    intercept_down: bool,
    /// Shown as bullets; copy, cut and IME reads of the value are refused.
    masked: bool,
    /// One line: newlines in pasted or committed text become spaces.
    single_line: bool,
    /// The last text change came from the user (typing, paste, delete, undo),
    /// not from a program setting the text.
    user_edit: bool,
    /// The range the platform typed into the input on the last change, if it
    /// was typing (not paste, an insert or a program setting the text).
    last_typed: Option<Range<usize>>,
    /// Draws an image in place of a span's text (custom emoji), when it
    /// returns one and the image cache is loaded.
    span_images: Option<SpanImages>,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    Typing,
    Other,
}

impl TextInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            buf: Buffer::default(),
            history: History::default(),
            focus: cx.focus_handle(),
            placeholder: SharedString::default(),
            placeholder_color: None,
            laid: None,
            vertical_x: None,
            upstream_at: None,
            scroll_y: px(0.0),
            selecting: false,
            intercept_down: false,
            masked: false,
            single_line: false,
            user_edit: false,
            last_typed: None,
            span_images: None,
        }
    }

    /// Whether the last text change was the user's own edit (typing, paste,
    /// delete, undo) rather than a program setting the text (a restored draft).
    pub fn user_edit(&self) -> bool {
        self.user_edit
    }

    /// The range the platform typed on the last change; `None` for paste, an
    /// insert via [`Self::insert_span`], undo and a restored draft.
    pub(crate) fn typed_range(&self) -> Option<Range<usize>> {
        self.last_typed.clone()
    }

    /// Paints an image in place of a span's text when `f` returns one and the
    /// image cache is loaded. The default (`None`) paints every span as text.
    pub fn set_span_images(&mut self, f: SpanImages) {
        self.span_images = Some(f);
    }

    pub(crate) fn has_span_images(&self) -> bool {
        self.span_images.is_some()
    }

    pub(crate) fn span_image(&self, inline: &domain::Inline, cx: &App) -> Option<domain::ImageRef> {
        self.span_images.as_ref().and_then(|f| f(inline, cx))
    }

    pub fn text(&self) -> &str {
        self.buf.text()
    }

    pub fn selection(&self) -> Selection {
        self.buf.selection()
    }

    /// The picked mentions and custom emoji in the text, as atomic byte ranges.
    pub fn spans(&self) -> &[domain::ComposedSpan] {
        self.buf.spans()
    }

    /// The caret, when nothing is selected.
    pub fn caret(&self) -> Option<usize> {
        let s = self.buf.selection();
        s.is_empty().then_some(s.head)
    }

    /// Shows bullets instead of the text (a secret). The value stays readable through
    /// [`Self::text`] for the host, but never reaches the clipboard or the IME.
    pub fn set_masked(&mut self, on: bool) {
        self.masked = on;
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }

    /// Keeps the text on one line: newlines in pasted or committed text become spaces.
    pub fn set_single_line(&mut self, on: bool) {
        self.single_line = on;
    }

    fn one_line(&self, s: &str) -> String {
        if self.single_line {
            s.replace("\r\n", " ").replace(['\r', '\n'], " ")
        } else {
            s.to_string()
        }
    }

    /// While set, Down is propagated to the host instead of moving the caret.
    pub fn set_intercept_down(&mut self, on: bool) {
        self.intercept_down = on;
    }

    /// Replaces `r` with `label` as one atomic span: deleting any of it deletes all of it.
    pub fn insert_span(
        &mut self,
        r: Range<usize>,
        label: &str,
        inline: domain::Inline,
        cx: &mut Context<Self>,
    ) {
        self.apply(Edit::Other, cx, |b| b.insert_span(r, label, inline));
    }

    /// Replaces `r` (bytes) with `s`; the caret goes after it.
    pub fn replace_range(&mut self, r: Range<usize>, s: &str, cx: &mut Context<Self>) {
        self.apply(Edit::Other, cx, |b| b.replace(r, s));
    }

    /// Whether an IME composition is in progress.
    pub fn is_composing(&self) -> bool {
        self.buf.marked().is_some()
    }

    /// Draws the placeholder in `color` (a field on a dark ground, such as
    /// the sidebar's, where the theme's muted text is too faint).
    pub fn set_placeholder_color(&mut self, color: gpui::Hsla) {
        self.placeholder_color = Some(color);
    }

    /// The grey text shown while empty. Set while the host renders, so it takes no repaint.
    pub fn set_placeholder(&mut self, p: impl Into<SharedString>) {
        self.placeholder = p.into();
    }

    /// [`Self::set_placeholder`] for a host that sets it on every render: an
    /// unchanged text is not copied again.
    pub fn set_placeholder_str(&mut self, p: &str) {
        if self.placeholder.as_ref() != p {
            self.placeholder = SharedString::from(p.to_string());
        }
    }

    /// Moves the caret one row up (the host's Up key when it has nothing else to do).
    pub fn move_up(&mut self, cx: &mut Context<Self>) {
        self.vertical(false, false, cx);
    }

    /// Replaces the text (a restored draft, an edit): caret at the end, history cleared.
    pub fn set_text(&mut self, s: &str, cx: &mut Context<Self>) {
        if self.buf.text() == s && !self.is_composing() {
            self.buf.set_selection(s.len(), s.len());
            self.history.clear();
            self.last_typed = None;
            cx.notify();
            return;
        }
        self.buf.set_text(s);
        self.history.clear();
        self.vertical_x = None;
        self.upstream_at = None;
        self.user_edit = false;
        self.last_typed = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    /// Replaces the text and its spans (a restored draft): caret at the end, history cleared.
    pub fn set_composed(
        &mut self,
        s: &str,
        spans: &[domain::ComposedSpan],
        cx: &mut Context<Self>,
    ) {
        self.buf.set_text_with_spans(s, spans);
        self.history.clear();
        self.vertical_x = None;
        self.upstream_at = None;
        self.user_edit = false;
        self.last_typed = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    /// Applies `edits` (`(range, replacement)`, ordered from the end of the text to its
    /// start) as one undo step, then selects `select`.
    pub fn edit_ranges(
        &mut self,
        edits: &[(Range<usize>, String)],
        select: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        self.apply(Edit::Other, cx, |b| {
            for (r, s) in edits {
                b.replace(r.clone(), s);
            }
            b.set_selection(select.start, select.end);
        });
    }

    /// Selects `r` (bytes, snapped to char boundaries).
    pub fn select(&mut self, r: Range<usize>, cx: &mut Context<Self>) {
        self.history.break_run();
        self.buf.set_selection(r.start, r.end);
        self.vertical_x = None;
        self.upstream_at = None;
        cx.notify();
    }

    pub fn replace_selection(&mut self, s: &str, cx: &mut Context<Self>) {
        let r = self.buf.selection().range();
        self.apply(Edit::Other, cx, |b| b.replace(r, s));
    }

    /// Commits a composition in progress, keeping what was typed.
    pub fn commit_composition(&mut self, cx: &mut Context<Self>) {
        if self.buf.marked().is_some() {
            self.buf.unmark();
            self.history.break_run();
            cx.notify();
        }
    }

    #[doc(hidden)]
    pub fn focus_handle_for_test(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Runs one edit: history, change event, repaint.
    fn apply(&mut self, kind: Edit, cx: &mut Context<Self>, f: impl FnOnce(&mut Buffer)) {
        self.apply_typed(kind, None, cx, f);
    }

    /// [`Self::apply`] for a change the platform typed, recording its range so
    /// the host can react to the text as typed.
    fn apply_typed(
        &mut self,
        kind: Edit,
        typed: Option<Range<usize>>,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Buffer),
    ) {
        if !self.is_composing() {
            let before = self.snapshot();
            self.history.record(before, kind == Edit::Typing);
        }
        f(&mut self.buf);
        self.vertical_x = None;
        self.upstream_at = None;
        self.user_edit = true;
        self.last_typed = typed;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.buf.text().to_string(),
            sel: self.buf.selection(),
            spans: self.buf.spans().to_vec(),
        }
    }

    fn move_head(&mut self, head: usize, extend: bool, cx: &mut Context<Self>) {
        self.history.break_run();
        self.upstream_at = None;
        let anchor = if extend {
            self.buf.selection().anchor
        } else {
            head
        };
        self.buf.set_selection(anchor, head);
        cx.notify();
    }

    /// Left or right by one cluster; without shift a selection collapses to its edge.
    fn step(&mut self, forward: bool, extend: bool, cx: &mut Context<Self>) {
        let s = self.buf.selection();
        self.vertical_x = None;
        self.upstream_at = None;
        let head = if !extend && !s.is_empty() {
            let r = s.range();
            if forward { r.end } else { r.start }
        } else if forward {
            self.buf.next_boundary(s.head)
        } else {
            self.buf.prev_boundary(s.head)
        };
        self.move_head(head, extend, cx);
    }

    fn word(&mut self, forward: bool, extend: bool, cx: &mut Context<Self>) {
        let h = self.buf.selection().head;
        let head = if self.masked {
            // A masked value gives no word boundaries away.
            if forward { self.buf.text().len() } else { 0 }
        } else if forward {
            self.buf.word_right(h)
        } else {
            self.buf.word_left(h)
        };
        self.vertical_x = None;
        self.upstream_at = None;
        self.move_head(head, extend, cx);
    }

    /// Start or end of the visual row (of the hard line when nothing wraps).
    fn line_edge(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        let h = self.buf.selection().head;
        let (head, upstream) = match self.wrapped_layout() {
            Some(l) => l.row_edge(h, end, self.caret_upstream()),
            None if end => (self.buf.line_end(h), false),
            None => (self.buf.line_start(h), false),
        };
        self.vertical_x = None;
        self.upstream_at = None;
        self.move_head(head, extend, cx);
        // Collapsed at a wrap boundary, the caret stays on this row.
        self.upstream_at = (upstream && !extend).then_some(head);
    }

    /// One row up or down: through the layout when the text wraps or has several lines,
    /// otherwise by hard line (same column, first and last line stop at the text edge).
    fn vertical(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let h = self.buf.selection().head;
        if let Some(l) = self.wrapped_layout() {
            let (head, x, upstream) = l.vertical(h, down, self.vertical_x, self.caret_upstream());
            self.move_head(head, extend, cx);
            self.vertical_x = x;
            self.upstream_at = (upstream && !extend).then_some(head);
            return;
        }
        let head = if down {
            let end = self.buf.line_end(h);
            if end >= self.buf.text().len() {
                end
            } else {
                self.column_on(end + 1, self.column_of(h))
            }
        } else {
            let start = self.buf.line_start(h);
            if start == 0 {
                0
            } else {
                self.column_on(self.buf.line_start(start - 1), self.column_of(h))
            }
        };
        self.move_head(head, extend, cx);
    }

    /// Characters between the start of the hard line and `i`.
    fn column_of(&self, i: usize) -> usize {
        self.buf.text()[self.buf.line_start(i)..self.buf.snap(i)]
            .chars()
            .count()
    }

    /// The offset `col` characters into the hard line starting at `start` (clamped to its end).
    fn column_on(&self, start: usize, col: usize) -> usize {
        let end = self.buf.line_end(start);
        self.buf.text()[start..end]
            .char_indices()
            .nth(col)
            .map_or(end, |(b, _)| start + b)
    }

    /// Whether the caret sits at the end of the row before its offset.
    fn caret_upstream(&self) -> bool {
        let s = self.buf.selection();
        s.is_empty() && self.upstream_at == Some(s.head)
    }

    #[doc(hidden)]
    pub fn caret_row_for_test(&self) -> Option<usize> {
        self.fresh_layout()?
            .row_of(self.buf.selection().head, self.caret_upstream())
    }

    /// The painted layout when it still has more than one row.
    fn wrapped_layout(&self) -> Option<&Laid> {
        self.fresh_layout().filter(|l| l.rows() > 1)
    }

    /// The painted layout when it still matches the text.
    fn fresh_layout(&self) -> Option<&Laid> {
        self.laid.as_ref().filter(|l| l.matches(self.buf.text()))
    }

    fn doc_edge(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        let head = if end { self.buf.text().len() } else { 0 };
        self.vertical_x = None;
        self.upstream_at = None;
        self.move_head(head, extend, cx);
    }

    fn delete_range(&mut self, r: Range<usize>, cx: &mut Context<Self>) {
        if !r.is_empty() {
            self.apply(Edit::Other, cx, |b| b.replace(r, ""));
        }
    }

    fn backspace(&mut self, cx: &mut Context<Self>) {
        let s = self.buf.selection();
        let r = if s.is_empty() {
            self.buf.prev_boundary(s.head)..s.head
        } else {
            s.range()
        };
        self.delete_range(r, cx);
    }

    fn delete_forward(&mut self, cx: &mut Context<Self>) {
        let s = self.buf.selection();
        let r = if s.is_empty() {
            s.head..self.buf.next_boundary(s.head)
        } else {
            s.range()
        };
        self.delete_range(r, cx);
    }

    fn delete_word_left(&mut self, cx: &mut Context<Self>) {
        let s = self.buf.selection();
        let r = if s.is_empty() && self.masked {
            0..s.head
        } else if s.is_empty() {
            self.buf.word_left(s.head)..s.head
        } else {
            s.range()
        };
        self.delete_range(r, cx);
    }

    fn selected_text(&self) -> Option<String> {
        let r = self.buf.selection().range();
        (!r.is_empty()).then(|| self.buf.text()[r].to_string())
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if self.masked {
            return;
        }
        if let Some(t) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(t));
        }
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        if self.masked {
            return;
        }
        if let Some(t) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(t));
            let r = self.buf.selection().range();
            self.delete_range(r, cx);
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
            let t = if self.single_line {
                self.one_line(t.trim())
            } else {
                t.replace("\r\n", "\n").replace('\r', "\n")
            };
            self.replace_selection(&t, cx);
        }
    }

    fn restore(&mut self, s: Snapshot, cx: &mut Context<Self>) {
        self.buf.restore(s.text, s.sel, s.spans);
        self.user_edit = true;
        self.vertical_x = None;
        self.upstream_at = None;
        self.last_typed = None;
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let cur = self.snapshot();
        if let Some(s) = self.history.undo(cur) {
            self.restore(s, cx);
        }
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        let cur = self.snapshot();
        if let Some(s) = self.history.redo(cur) {
            self.restore(s, cx);
        }
    }

    // Mouse, called by the element with positions in window coordinates.

    pub(crate) fn mouse_down(
        &mut self,
        p: Point<Pixels>,
        clicks: usize,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(l) = self.fresh_layout() else {
            return;
        };
        match clicks {
            0 | 1 => {
                let i = l.index_at(p);
                self.selecting = true;
                self.move_head(i, shift, cx);
            }
            2 => {
                let r = l.word_at(p);
                self.selecting = true;
                self.select_range(r, cx);
            }
            _ => {
                // A hard line, whatever the layout wraps it into.
                let i = l.index_at(p);
                let r = self.buf.line_start(i)..self.buf.line_end(i);
                self.selecting = true;
                self.select_range(r, cx);
            }
        }
        self.vertical_x = None;
        self.upstream_at = None;
    }

    pub(crate) fn mouse_drag(&mut self, p: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        if let Some(i) = self.fresh_layout().map(|l| l.index_at(p)) {
            let a = self.buf.selection().anchor;
            self.buf.set_selection(a, i);
            cx.notify();
        }
    }

    pub(crate) fn mouse_up(&mut self) {
        self.selecting = false;
    }

    fn select_range(&mut self, r: Range<usize>, cx: &mut Context<Self>) {
        self.history.break_run();
        self.buf.set_selection(r.start, r.end);
        cx.notify();
    }

    fn set_laid(&mut self, laid: Laid, scroll_y: Pixels) {
        self.laid = Some(laid);
        self.scroll_y = scroll_y;
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        if self.masked {
            return None;
        }
        let r = self.buf.range_from_utf16(&range_utf16);
        *adjusted = Some(self.buf.range_to_utf16(&r));
        Some(self.buf.text()[r].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let s = self.buf.selection();
        Some(UTF16Selection {
            range: self.buf.range_to_utf16(&s.range()),
            reversed: s.head < s.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.buf.marked().map(|r| self.buf.range_to_utf16(&r))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.commit_composition(cx);
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range_utf16
            .map(|r| self.buf.range_from_utf16(&r))
            .or_else(|| self.buf.marked())
            .unwrap_or_else(|| self.buf.selection().range());
        let typing = r.is_empty() && !text.contains('\n') && !self.is_composing();
        let kind = if typing { Edit::Typing } else { Edit::Other };
        let text = self.one_line(text);
        if typing {
            let typed = r.start..r.start + text.len();
            self.apply_typed(kind, Some(typed), cx, |b| b.replace(r, &text));
        } else {
            self.apply(kind, cx, |b| b.replace(r, &text));
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range_utf16
            .map(|r| self.buf.range_from_utf16(&r))
            .or_else(|| self.buf.marked())
            .unwrap_or_else(|| self.buf.selection().range());
        // Byte offsets inside the new text, from the platform's UTF-16 ones.
        let select = new_selected_utf16.map(|s| {
            let u = |o: usize| utf16_to_utf8_offset(new_text, o);
            u(s.start)..u(s.end)
        });
        self.apply(Edit::Other, cx, |b| b.replace_and_mark(r, new_text, select));
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let r = self.buf.range_from_utf16(&range_utf16);
        self.fresh_layout()?.bounds_of(r)
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let i = self.fresh_layout()?.index_at(point);
        Some(self.buf.byte_to_utf16(i))
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let me = cx.entity();
        let mut root = div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .w_full()
            .text_color(colors.text)
            .debug_selector(|| "text-input".into());
        macro_rules! on {
            ($($action:ident => |$t:ident, $cx:ident| $body:expr),* $(,)?) => {
                $(root = root.on_action(cx.listener(|$t, _: &$action, _, $cx| $body));)*
            };
        }
        on!(
            Left => |t, cx| t.step(false, false, cx),
            Right => |t, cx| t.step(true, false, cx),
            Up => |t, cx| t.vertical(false, false, cx),
            Down => |t, cx| {
                if t.intercept_down {
                    cx.propagate()
                } else {
                    t.vertical(true, false, cx)
                }
            },
            WordLeft => |t, cx| t.word(false, false, cx),
            WordRight => |t, cx| t.word(true, false, cx),
            LineStart => |t, cx| t.line_edge(false, false, cx),
            LineEnd => |t, cx| t.line_edge(true, false, cx),
            DocStart => |t, cx| t.doc_edge(false, false, cx),
            DocEnd => |t, cx| t.doc_edge(true, false, cx),
            SelectLeft => |t, cx| t.step(false, true, cx),
            SelectRight => |t, cx| t.step(true, true, cx),
            SelectUp => |t, cx| t.vertical(false, true, cx),
            SelectDown => |t, cx| t.vertical(true, true, cx),
            SelectWordLeft => |t, cx| t.word(false, true, cx),
            SelectWordRight => |t, cx| t.word(true, true, cx),
            SelectLineStart => |t, cx| t.line_edge(false, true, cx),
            SelectLineEnd => |t, cx| t.line_edge(true, true, cx),
            SelectDocStart => |t, cx| t.doc_edge(false, true, cx),
            SelectDocEnd => |t, cx| t.doc_edge(true, true, cx),
            SelectAll => |t, cx| {
                let n = t.buf.text().len();
                t.select(0..n, cx)
            },
            Backspace => |t, cx| t.backspace(cx),
            Delete => |t, cx| t.delete_forward(cx),
            DeleteWordLeft => |t, cx| t.delete_word_left(cx),
            Cut => |t, cx| t.cut(cx),
            Copy => |t, cx| t.copy(cx),
            Paste => |t, cx| t.paste(cx),
            Undo => |t, cx| t.undo(cx),
            Redo => |t, cx| t.redo(cx),
        );
        root.child(TextElement::new(me, MAX_ROWS))
    }
}

/// Convenience for hosts that own the entity.
pub fn new(cx: &mut App) -> Entity<TextInput> {
    cx.new(TextInput::new)
}
