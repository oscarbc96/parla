//! A generic keyboard-first picker: a query field over a delegate's matches.
//! The quick switcher is one delegate; others reuse the view.

use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    Render, Window, div, prelude::*,
};
use themes::ActiveTheme;
use themes::metrics::{
    FONT_SIZE_MD, FONT_SIZE_XL, FONT_SIZE_XS, ICON_SIZE_SM, LAYOUT_SWITCHER_INPUT_HEIGHT,
    RADIUS_LG, RADIUS_MD, RADIUS_XS, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_10,
};

use crate::emoji_picker::{Confirm, Dismiss, KEY_CONTEXT, SelectNext, SelectPrev};

/// What a picker does with pasted text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteOutcome {
    Insert,
    Handled,
    Close,
}

/// What a picker lists and does with a choice.
pub trait PickerDelegate: 'static {
    type Item;
    /// Recomputes the matches for `query` (called on every edit).
    fn update_matches(&mut self, query: &str, cx: &mut App);
    fn matches(&self) -> &[Self::Item];
    fn render_item(&self, i: usize, selected: bool, cx: &mut App) -> AnyElement;
    /// The user chose match `i`.
    fn confirm(&mut self, i: usize, cx: &mut App);
    fn placeholder(&self, _cx: &App) -> String {
        String::new()
    }
    fn empty_text(&self, _cx: &App) -> String {
        String::new()
    }
    /// Handles pasted text; by default it is inserted into the query.
    fn paste(&mut self, _text: &str, _window: &mut Window, _cx: &mut App) -> PasteOutcome {
        PasteOutcome::Insert
    }
}

/// Emitted by [`Picker`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerEvent {
    /// Match `i` was chosen (the delegate has been told); the host closes the picker.
    Confirmed(usize),
    Dismissed,
}

/// Rows shown at once; the list scrolls by windowing, so a long match list
/// costs nothing to draw.
pub const MAX_ROWS: usize = 10;

pub struct Picker<D: PickerDelegate> {
    delegate: D,
    query: String,
    selected: usize,
    focus: FocusHandle,
}

impl<D: PickerDelegate> EventEmitter<PickerEvent> for Picker<D> {}

impl<D: PickerDelegate> Picker<D> {
    pub fn new(mut delegate: D, cx: &mut Context<Self>) -> Self {
        delegate.update_matches("", cx);
        Self {
            delegate,
            query: String::new(),
            selected: 0,
            focus: cx.focus_handle(),
        }
    }

    pub fn delegate(&self) -> &D {
        &self.delegate
    }

    /// Changes the delegate (new items arrived), then matches the query again.
    pub fn update_delegate(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut D)) {
        f(&mut self.delegate);
        self.delegate.update_matches(&self.query, cx);
        let n = self.delegate.matches().len();
        self.selected = self.selected.min(n.saturating_sub(1));
        cx.notify();
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Index of the highlighted match.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Replaces the query and recomputes the matches.
    pub fn set_query(&mut self, q: impl Into<String>, cx: &mut Context<Self>) {
        self.query = q.into();
        self.selected = 0;
        self.delegate.update_matches(&self.query, cx);
        cx.notify();
    }

    /// Chooses the highlighted match (nothing when there is none).
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        let i = self.selected;
        if i >= self.delegate.matches().len() {
            return;
        }
        self.delegate.confirm(i, cx);
        cx.emit(PickerEvent::Confirmed(i));
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(PickerEvent::Dismissed);
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.delegate.matches().len();
        if n > 0 {
            self.selected = (self.selected as isize + delta).clamp(0, n as isize - 1) as usize;
        }
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if (m.platform || m.control) && k.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                match self.delegate.paste(&text, window, cx) {
                    PasteOutcome::Handled => return,
                    PasteOutcome::Close => {
                        cx.emit(PickerEvent::Dismissed);
                        return;
                    }
                    PasteOutcome::Insert => {}
                }
                let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let q = format!("{}{joined}", self.query);
                self.set_query(q, cx);
            }
            return;
        }
        if m.platform || m.control || m.alt {
            return;
        }
        match k.key.as_str() {
            "backspace" => {
                let mut q = self.query.clone();
                q.pop();
                self.set_query(q, cx);
            }
            key => {
                let typed = match k.key_char.as_deref() {
                    Some(c) => c,
                    None if key.chars().count() == 1 => key,
                    None => return,
                };
                let q = format!("{}{typed}", self.query);
                self.set_query(q, cx);
            }
        }
    }
}

impl<D: PickerDelegate> Focusable for Picker<D> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<D: PickerDelegate> Render for Picker<D> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let n = self.delegate.matches().len();
        let start = (self.selected + 1).saturating_sub(MAX_ROWS);
        let field = if self.query.is_empty() {
            div()
                .text_color(colors.text_muted)
                .child(self.delegate.placeholder(cx))
        } else {
            div().child(format!("{}|", self.query))
        };
        let mut list = div()
            .id("picker-list")
            .debug_selector(|| "picker-list".into())
            .flex()
            .flex_col()
            .pt(SPACE_2)
            .pb(SPACE_4);
        if n == 0 {
            list = list.child(
                div()
                    .px(SPACE_6)
                    .py(SPACE_6)
                    .text_size(FONT_SIZE_MD)
                    .text_color(colors.text_muted)
                    .child(self.delegate.empty_text(cx)),
            );
        }
        for i in start..n.min(start + MAX_ROWS) {
            let selected = i == self.selected;
            let row = self.delegate.render_item(i, selected, cx);
            list = list.child(
                div()
                    .id(("picker-row", i))
                    .mx(SPACE_4)
                    .px(SPACE_5)
                    .py(SPACE_2)
                    .rounded(RADIUS_MD)
                    .cursor_pointer()
                    .when(selected, |d| d.bg(colors.accent_soft))
                    .when(!selected, |d| d.hover(|s| s.bg(colors.element_hover)))
                    .child(row)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = i;
                        this.confirm(cx);
                    })),
            );
        }
        let hint = |key: &'static str, label: String| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    div()
                        .px(SPACE_2)
                        .rounded(RADIUS_XS)
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.surface_background)
                        .text_color(colors.text)
                        .child(crate::keycap::glyph_row(
                            &key.into(),
                            ICON_SIZE_SM,
                            colors.text,
                            cx,
                        )),
                )
                .child(label)
        };
        let footer = div()
            .flex()
            .items_center()
            .gap(SPACE_6)
            .h(SPACE_10)
            .px(SPACE_6)
            .rounded_b(RADIUS_LG)
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .text_size(FONT_SIZE_XS)
            .text_color(colors.text_muted)
            .child(hint(
                crate::keycap::UP_DOWN,
                t!(cx, "picker-hint-navigate").into_owned(),
            ))
            .child(hint(
                crate::keycap::RETURN,
                t!(cx, "picker-hint-select").into_owned(),
            ))
            .child(hint("esc", t!(cx, "picker-hint-close").into_owned()));
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.confirm(cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.dismiss(cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.step(-1, cx)))
            .flex()
            .flex_col()
            .w_full()
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(RADIUS_LG)
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .id("picker-field")
                    .flex()
                    .items_center()
                    .h(LAYOUT_SWITCHER_INPUT_HEIGHT)
                    .px(SPACE_6)
                    .text_size(FONT_SIZE_XL)
                    .border_b_1()
                    .border_color(colors.border)
                    .child(field),
            )
            .child(list)
            .child(footer)
    }
}
