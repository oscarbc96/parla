//! The keyboard shortcuts sheet (the FirstRunTour board's "shortcuts"
//! overlay): every bound action grouped by area, with a filter field, a close
//! button and a way into the keymap. The host builds the groups from its live
//! keymap; this view only shows and filters them. Esc (`picker::Dismiss`)
//! closes it.

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Render, SharedString,
    Subscription, Window, div, prelude::*, relative,
};
use themes::{ActiveTheme as _, metrics};

use crate::controls::{ButtonKind, button_frame, compact, icon_button, on_activate_app, ring};
use crate::emoji_picker::{Dismiss, KEY_CONTEXT};
use crate::icon::{Icon, IconName};
use crate::text_input::{TextInput, TextInputEvent};

/// One shortcut: its label, the keymap keys ("cmd-k") and their key caps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutItem {
    pub label: SharedString,
    pub keystrokes: SharedString,
    pub keys: Vec<SharedString>,
}

/// An area's shortcuts under its title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutGroup {
    pub title: SharedString,
    pub items: Vec<ShortcutItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutsSheetEvent {
    /// Esc or the close button.
    Dismissed,
    /// The "Edit keymap" button.
    EditKeymap,
}

pub struct ShortcutsSheet {
    groups: Vec<ShortcutGroup>,
    /// The key caps of the binding that opens the sheet, for the header hint.
    open_keys: Vec<SharedString>,
    input: Entity<TextInput>,
    _sub: Subscription,
}

impl EventEmitter<ShortcutsSheetEvent> for ShortcutsSheet {}

/// The caps of keymap keys ("cmd-/"): glyphs on macOS, words elsewhere.
fn caps(binding: &str, mac: bool) -> Vec<SharedString> {
    crate::notify_menu::display_binding(binding, mac)
        .split(if mac { "" } else { "+" })
        .filter(|s| !s.is_empty())
        .map(|s| SharedString::from(s.to_string()))
        .collect()
}

fn item_matches(item: &ShortcutItem, q: &str) -> bool {
    q.is_empty()
        || item.label.to_lowercase().contains(q)
        || item.keystrokes.to_lowercase().contains(q)
        || item
            .keys
            .iter()
            .map(|k| k.as_ref())
            .collect::<String>()
            .to_lowercase()
            .contains(q)
}

impl ShortcutsSheet {
    /// `open_binding` is the keymap keys that open the sheet ("cmd-/").
    pub fn new(
        groups: Vec<ShortcutGroup>,
        open_binding: &str,
        mac: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(TextInput::new);
        let placeholder = t!(cx, "shortcuts-filter").into_owned();
        input.update(cx, |t, _| {
            t.set_single_line(true);
            t.set_placeholder(placeholder);
        });
        let sub = cx.subscribe(&input, |_, _, _: &TextInputEvent, cx| cx.notify());
        Self {
            groups,
            open_keys: caps(open_binding, mac),
            input,
            _sub: sub,
        }
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).text().trim().to_lowercase()
    }

    /// Replaces the filter text.
    pub fn set_query(&mut self, q: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |t, cx| t.set_text(q, cx));
        cx.notify();
    }

    /// The groups with only the items the filter keeps; empty groups dropped.
    fn visible(&self, cx: &App) -> Vec<ShortcutGroup> {
        let q = self.query(cx);
        self.groups
            .iter()
            .filter_map(|g| {
                let items: Vec<ShortcutItem> = g
                    .items
                    .iter()
                    .filter(|i| item_matches(i, &q))
                    .cloned()
                    .collect();
                (!items.is_empty()).then(|| ShortcutGroup {
                    title: g.title.clone(),
                    items,
                })
            })
            .collect()
    }

    /// The labels shown now, in order (for tests).
    pub fn visible_labels(&self, cx: &App) -> Vec<String> {
        self.visible(cx)
            .into_iter()
            .flat_map(|g| g.items.into_iter().map(|i| i.label.to_string()))
            .collect()
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(ShortcutsSheetEvent::Dismissed);
    }
}

impl Focusable for ShortcutsSheet {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

/// One key cap.
pub fn key_cap(key: SharedString, colors: &themes::ThemeColors, cx: &gpui::App) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .min_w(metrics::SPACE_7)
        .h(metrics::SPACE_7)
        .px(metrics::SPACE_2)
        .rounded(metrics::RADIUS_XS)
        .border_1()
        .border_color(colors.border)
        .bg(colors.surface_background)
        .text_size(metrics::FONT_SIZE_XS)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .text_color(colors.text)
        .child(crate::keycap::glyph_row(
            &key,
            metrics::ICON_SIZE_SM,
            colors.text,
            cx,
        ))
}

/// A row of key caps.
pub fn key_caps(keys: &[SharedString], colors: &themes::ThemeColors, cx: &gpui::App) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .gap(metrics::SPACE_1)
        .children(keys.iter().map(|k| key_cap(k.clone(), colors, cx)))
}

impl Render for ShortcutsSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let this = cx.entity();
        let close = icon_button(
            "shortcuts-close",
            IconName::X,
            t!(cx, "shortcuts-close").into_owned(),
            &colors,
            move |_, cx| this.update(cx, |v, cx| v.dismiss(cx)),
        )
        .debug_selector(|| "shortcuts-close".into());
        let field = div()
            .debug_selector(|| "shortcuts-filter".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(metrics::SPACE_3)
            .w(metrics::LAYOUT_SHEET_SEARCH)
            .h(metrics::LAYOUT_INPUT_MIN)
            .px(metrics::SPACE_4)
            .rounded(metrics::RADIUS_MD)
            .border_1()
            .border_color(tokens.border_control)
            .bg(colors.surface_background)
            .text_size(metrics::FONT_SIZE_SM)
            .when(focused, |d| {
                d.border_color(gpui::transparent_black())
                    .shadow(ring(&colors))
            })
            .child(
                Icon::new(IconName::Search)
                    .size(metrics::ICON_SIZE_BASE)
                    .color(tokens.text_subtle),
            )
            .child(div().flex_1().min_w_0().child(self.input.clone()));
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(metrics::SPACE_5)
            .px(metrics::SPACE_7)
            .py(metrics::SPACE_6)
            .border_b_1()
            .border_color(colors.border)
            .child(
                Icon::new(IconName::Keyboard)
                    .size(metrics::ICON_SIZE_XL)
                    .color(tokens.signal),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_LG)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(t!(cx, "shortcuts-title").into_owned()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_2)
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(tokens.text_subtle)
                    .child(t!(cx, "shortcuts-open-hint").into_owned())
                    .child(key_caps(&self.open_keys, &colors, cx)),
            )
            .child(div().flex_1())
            .child(field)
            .child(close);

        let groups = self.visible(cx);
        let body = if groups.is_empty() {
            div()
                .debug_selector(|| "shortcuts-empty".into())
                .p(metrics::SPACE_8)
                .text_size(metrics::FONT_SIZE_SM)
                .text_color(colors.text_muted)
                .child(t!(cx, "shortcuts-empty").into_owned())
        } else {
            div()
                .flex()
                .flex_wrap()
                .px(metrics::SPACE_5)
                .py(metrics::SPACE_7)
                .children(groups.into_iter().map(|g| {
                    let selector = format!("shortcuts-group-{}", g.title);
                    div()
                        .debug_selector(move || selector.clone())
                        .flex()
                        .flex_col()
                        .w(relative(1.0 / 3.0))
                        .px(metrics::SPACE_5)
                        .pb(metrics::SPACE_7)
                        .child(
                            div()
                                .pb(metrics::SPACE_4)
                                .text_size(metrics::FONT_SIZE_2XS)
                                .font_weight(metrics::FONT_WEIGHT_BOLD)
                                .text_color(tokens.text_subtle)
                                .child(g.title.to_uppercase()),
                        )
                        .children(g.items.into_iter().map(|i| {
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(metrics::SPACE_5)
                                .min_h(metrics::LAYOUT_ROW_COMPACT)
                                .text_size(metrics::FONT_SIZE_SM)
                                .child(div().flex_1().min_w_0().py(metrics::SPACE_2).child(i.label))
                                .child(key_caps(&i.keys, &colors, cx))
                        }))
                }))
        };

        let this = cx.entity();
        let edit = on_activate_app(
            compact(
                button_frame(
                    "shortcuts-edit-keymap",
                    ButtonKind::Secondary,
                    true,
                    &colors,
                )
                .gap(metrics::SPACE_3)
                .child(Icon::new(IconName::Pencil).size(metrics::ICON_SIZE_SM))
                .child(t!(cx, "shortcuts-edit-keymap").into_owned()),
            )
            .debug_selector(|| "shortcuts-edit-keymap".into()),
            move |_, cx| this.update(cx, |_, cx| cx.emit(ShortcutsSheetEvent::EditKeymap)),
        );
        let footer = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(metrics::SPACE_4)
            .px(metrics::SPACE_7)
            .py(metrics::SPACE_5)
            .border_t_1()
            .border_color(colors.border)
            .bg(tokens.bg_sunken)
            .child(
                div()
                    .flex_1()
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(colors.text_muted)
                    .child(t!(cx, "shortcuts-footer").into_owned()),
            )
            .child(edit);

        div()
            .id("shortcuts-sheet")
            .debug_selector(|| "shortcuts-sheet".into())
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.dismiss(cx)))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => this.dismiss(cx),
                    "tab" if ev.keystroke.modifiers.shift => window.focus_prev(cx),
                    "tab" => window.focus_next(cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .flex()
            .flex_col()
            .w_full()
            .max_w(metrics::LAYOUT_SHEET_WIDTH)
            .max_h_full()
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded(metrics::RADIUS_XL)
            .shadow_lg()
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .id("shortcuts-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(footer)
    }
}
