//! Emoji picker: fuzzy search over `emojis` shortcodes and custom emoji, a
//! keyboard-navigable grid, skin tones and a "Recent" row the host seeds from the store.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use domain::{CustomEmoji, EmojiRef, ImageRef};
use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyBinding,
    KeyDownEvent, Render, SharedString, UniformListScrollHandle, Window, actions, div, prelude::*,
    uniform_list,
};
use themes::ActiveTheme;

use crate::reactions::{glyph, tooltip_shared};

actions!(
    picker,
    [
        Confirm,
        SelectNext,
        SelectPrev,
        SelectLeft,
        SelectRight,
        Dismiss
    ]
);

/// Key context of the picker view.
pub const KEY_CONTEXT: &str = "Picker";

/// Grid columns (arrow keys move by one, up/down by a row).
pub const COLUMNS: usize = 8;
/// Rows the grid shows before it scrolls; the rest are virtualised.
const GRID_ROWS: usize = 6;
const MAX_SHOWN: usize = 160;
const MAX_RECENT: usize = 16;

/// Registers the default picker key bindings.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Confirm, Some(KEY_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(KEY_CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(KEY_CONTEXT)),
        KeyBinding::new("left", SelectLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
    ]);
}

/// Emitted when the user confirms an emoji.
#[derive(Clone, Debug, PartialEq)]
pub struct EmojiPicked(pub EmojiRef);

/// Emitted when the picker is dismissed without a choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerDismissed;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerTab {
    Standard,
    Custom,
}

/// Subsequence match score of `query` in `cand` (both compared lowercase); higher is better.
pub fn fuzzy_score(query: &str, cand: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q_len = query.chars().flat_map(char::to_lowercase).count();
    let (mut qi, mut score, mut prev, mut c_len) = (0usize, 0i32, None::<usize>, 0usize);
    let mut q_iter = query.chars().flat_map(char::to_lowercase).peekable();
    for (ci, ch) in cand.chars().flat_map(char::to_lowercase).enumerate() {
        c_len = ci + 1;
        if q_iter.peek() == Some(&ch) {
            q_iter.next();
            score += 10;
            if ci == 0 {
                score += 30;
            }
            if prev.is_some_and(|p| p + 1 == ci) {
                score += 15;
            }
            prev = Some(ci);
            qi += 1;
        }
    }
    if qi < q_len {
        return None;
    }
    if q_len == c_len
        && query
            .chars()
            .flat_map(char::to_lowercase)
            .eq(cand.chars().flat_map(char::to_lowercase))
    {
        score += 1000;
    }
    Some(score - (c_len - q_len) as i32)
}

fn tone(n: u8) -> Option<emojis::SkinTone> {
    Some(match n {
        1 => emojis::SkinTone::Light,
        2 => emojis::SkinTone::MediumLight,
        3 => emojis::SkinTone::Medium,
        4 => emojis::SkinTone::MediumDark,
        5 => emojis::SkinTone::Dark,
        _ => return None,
    })
}

pub struct EmojiPicker {
    query: String,
    custom: Vec<CustomEmoji>,
    /// Image of each custom emoji by shortcode, built once from `custom`.
    images: HashMap<Arc<str>, ImageRef>,
    custom_enabled: bool,
    tab: PickerTab,
    skin_tone: Option<u8>,
    selected: usize,
    /// The keyboard selection is in the Recent row (at this index), not the grid.
    recent_sel: Option<usize>,
    /// Cached grid contents; rebuilt when query, tab or skin tone change.
    cache: Vec<EmojiRef>,
    /// `:shortcode:` tooltip of each cached cell, built with `cache`.
    labels: Vec<SharedString>,
    /// Emoji used in this workspace, newest first (the host persists them).
    recent: Vec<EmojiRef>,
    focus: FocusHandle,
    /// Scroll of the grid, which keeps the keyboard selection in view.
    scroll: UniformListScrollHandle,
}

impl EventEmitter<EmojiPicked> for EmojiPicker {}
impl EventEmitter<PickerDismissed> for EmojiPicker {}

impl EmojiPicker {
    /// `custom_enabled` mirrors `Capabilities::custom_emoji`.
    pub fn new(custom: Vec<CustomEmoji>, custom_enabled: bool, cx: &mut Context<Self>) -> Self {
        let images = custom
            .iter()
            .map(|c| (c.shortcode.clone(), c.image.clone()))
            .collect();
        let mut p = Self {
            query: String::new(),
            custom,
            images,
            custom_enabled,
            tab: PickerTab::Standard,
            skin_tone: None,
            selected: 0,
            recent_sel: None,
            cache: Vec::new(),
            labels: Vec::new(),
            recent: Vec::new(),
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        };
        p.refresh();
        p
    }

    pub fn tabs(&self) -> Vec<PickerTab> {
        if self.custom_enabled {
            vec![PickerTab::Standard, PickerTab::Custom]
        } else {
            vec![PickerTab::Standard]
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, q: String, cx: &mut Context<Self>) {
        self.query = q;
        self.selected = 0;
        self.recent_sel = None;
        self.refresh();
        cx.notify();
    }

    pub fn select_tab(&mut self, tab: PickerTab, cx: &mut Context<Self>) {
        if self.tabs().contains(&tab) {
            self.tab = tab;
            self.selected = 0;
            self.recent_sel = None;
            self.refresh();
            cx.notify();
        }
    }

    /// Skin tone 1..=5 (light to dark); `None` for the default yellow.
    pub fn set_skin_tone(&mut self, t: Option<u8>, cx: &mut Context<Self>) {
        self.skin_tone = t.filter(|n| (1..=5).contains(n));
        self.refresh();
        cx.notify();
    }

    /// Seeds the Recent row (the workspace's persisted list, newest first).
    pub fn set_recent(&mut self, mut list: Vec<EmojiRef>, cx: &mut Context<Self>) {
        list.truncate(MAX_RECENT);
        self.recent = list;
        self.recent_sel = None;
        cx.notify();
    }

    /// The Recent row: the seeded list with this picker's picks in front, newest first.
    pub fn recent(&self) -> Vec<EmojiRef> {
        self.recent.clone()
    }

    /// Current grid contents: matches of the query in the active tab, best first.
    pub fn results(&self) -> Vec<EmojiRef> {
        self.cache.clone()
    }

    fn refresh(&mut self) {
        self.cache = self.compute_results();
        self.labels = self
            .cache
            .iter()
            .map(|e| SharedString::from(format!(":{}:", e.shortcode)))
            .collect();
    }

    /// Matching custom emoji, best score first, ties by shortcode (so an empty
    /// query lists every one alphabetically).
    fn custom_results(&self) -> Vec<(i32, EmojiRef)> {
        let mut v: Vec<(i32, EmojiRef)> = self
            .custom
            .iter()
            .filter_map(|c| {
                fuzzy_score(&self.query, &c.shortcode).map(|s| {
                    (
                        s,
                        EmojiRef {
                            shortcode: c.shortcode.clone(),
                            unicode: None,
                            skin_tone: None,
                        },
                    )
                })
            })
            .collect();
        v.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.shortcode.cmp(&b.1.shortcode))
        });
        v
    }

    fn compute_results(&self) -> Vec<EmojiRef> {
        match self.tab {
            PickerTab::Custom => self.custom_results().into_iter().map(|(_, e)| e).collect(),
            PickerTab::Standard => {
                let mut v: Vec<(i32, &'static emojis::Emoji, &'static str)> = emojis::iter()
                    .filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default))
                    .filter_map(|e| {
                        e.shortcodes()
                            .filter_map(|s| fuzzy_score(&self.query, s).map(|n| (n, s)))
                            .max_by_key(|(n, _)| *n)
                            .map(|(n, s)| (n, e, s))
                    })
                    .collect();
                let mut rows: Vec<(i32, String, EmojiRef)> = v
                    .drain(..)
                    .map(|(s, e, code)| (s, code.to_string(), self.standard_ref(e, code)))
                    .collect();
                if self.query.is_empty() {
                    rows.truncate(MAX_SHOWN);
                } else {
                    // The Emoji tab searches the workspace's custom emoji too.
                    if self.custom_enabled {
                        rows.extend(
                            self.custom_results()
                                .into_iter()
                                .map(|(s, e)| (s, e.shortcode.to_string(), e)),
                        );
                    }
                    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
                    rows.truncate(MAX_SHOWN);
                }
                rows.into_iter().map(|(_, _, e)| e).collect()
            }
        }
    }

    fn standard_ref(&self, e: &'static emojis::Emoji, code: &str) -> EmojiRef {
        let toned = self
            .skin_tone
            .and_then(tone)
            .and_then(|t| e.with_skin_tone(t));
        EmojiRef {
            shortcode: code.into(),
            unicode: Some(toned.unwrap_or(e).as_str().into()),
            skin_tone: toned.and(self.skin_tone),
        }
    }

    /// Emits [`EmojiPicked`] for the selected cell and remembers it as recent.
    pub fn confirm(&mut self, cx: &mut Context<Self>) {
        let e = match self.recent_sel {
            Some(i) => self.recent.get(i).cloned(),
            None => self.cache.get(self.selected).cloned(),
        };
        let Some(e) = e else {
            return;
        };
        self.pick(e, cx);
    }

    /// The Recent row is drawn: no query, Standard tab, something recent.
    fn recent_shown(&self) -> bool {
        self.query.is_empty() && self.tab == PickerTab::Standard && !self.recent.is_empty()
    }

    /// Index of the keyboard selection inside the Recent row, if it is there.
    pub fn recent_selected(&self) -> Option<usize> {
        self.recent_sel.filter(|_| self.recent_shown())
    }

    fn pick(&mut self, e: EmojiRef, cx: &mut Context<Self>) {
        self.recent.retain(|r| r.shortcode != e.shortcode);
        self.recent.insert(0, e.clone());
        self.recent.truncate(MAX_RECENT);
        cx.emit(EmojiPicked(e));
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let vertical = delta.unsigned_abs() == COLUMNS;
        if let Some(i) = self.recent_selected() {
            // In the Recent row: left and right walk it, down returns to the grid.
            if !vertical {
                let last = self.recent.len() - 1;
                self.recent_sel = Some((i as isize + delta).clamp(0, last as isize) as usize);
            } else if delta > 0 {
                self.recent_sel = None;
                self.selected = i.min(self.cache.len().saturating_sub(1));
            }
            cx.notify();
            return;
        }
        if vertical && delta < 0 && self.selected < COLUMNS && self.recent_shown() {
            // Up from the grid's first row enters the Recent row.
            self.recent_sel = Some(self.selected.min(self.recent.len() - 1));
            cx.notify();
            return;
        }
        let n = self.cache.len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, n as isize - 1) as usize;
        self.scroll
            .scroll_to_item(self.selected / COLUMNS, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if m.control || m.platform || m.alt {
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

    /// One emoji as drawn: its custom image when the cache can fetch it, else
    /// the glyph (or `:shortcode:`).
    fn cell_content(&self, e: &EmojiRef, cx: &App) -> AnyElement {
        let image = if e.unicode.is_none() {
            self.images.get(&e.shortcode)
        } else {
            None
        };
        match (image, cx.has_global::<crate::media::ImageCache>()) {
            (Some(image), true) => crate::media::emoji_hook(image, &e.shortcode),
            _ => glyph(e).into_any_element(),
        }
    }

    /// A row of [`COLUMNS`] grid cells; `uniform_list` draws only the visible ones.
    fn render_rows(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = *cx.theme().colors();
        let recent_sel = self.recent_selected();
        let mut out = Vec::with_capacity(range.len());
        for row in range {
            let mut line = div()
                .id(("picker-row", row))
                .flex()
                .h(themes::metrics::SPACE_9);
            for col in 0..COLUMNS {
                let i = row * COLUMNS + col;
                let Some(e) = self.cache.get(i).cloned() else {
                    break;
                };
                let tooltip = self
                    .labels
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!(":{}:", e.shortcode).into());
                let e2 = e.clone();
                line = line.child(
                    div()
                        .id(("picker-cell", i))
                        .w(themes::metrics::SPACE_9)
                        .h(themes::metrics::SPACE_9)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .overflow_hidden()
                        .when(i == self.selected && recent_sel.is_none(), |d| {
                            d.bg(colors.accent_soft)
                        })
                        .child(self.cell_content(&e, cx))
                        .tooltip(tooltip_shared(tooltip))
                        .on_click(cx.listener(move |this, _, _, cx| this.pick(e2.clone(), cx))),
                );
            }
            out.push(line.into_any_element());
        }
        out
    }
}

impl Focusable for EmojiPicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for EmojiPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let rows = self.cache.len().div_ceil(COLUMNS);
        let grid_h = themes::metrics::SPACE_9 * rows.min(GRID_ROWS) as f32;
        let recent_sel = self.recent_selected();
        let recent = if self.query.is_empty() && self.tab == PickerTab::Standard {
            self.recent()
        } else {
            Vec::new()
        };

        let mut tabs = div().flex().flex_none().gap_2();
        for (i, t) in self.tabs().into_iter().enumerate() {
            let active = t == self.tab;
            tabs = tabs.child(
                div()
                    .id(("picker-tab", i))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .font_weight(themes::metrics::FONT_WEIGHT_MEDIUM)
                    .text_color(if active {
                        colors.text
                    } else {
                        colors.text_muted
                    })
                    .when(active, |d| d.bg(colors.accent_soft))
                    .child(match t {
                        PickerTab::Standard => t!(cx, "picker-tab-emoji").into_owned(),
                        PickerTab::Custom => t!(cx, "picker-tab-custom").into_owned(),
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.select_tab(t, cx))),
            );
        }
        let mut tones = div().flex().flex_none().gap_1().text_xs();
        for (i, n) in [None, Some(1u8), Some(2), Some(3), Some(4), Some(5)]
            .into_iter()
            .enumerate()
        {
            let label = match n {
                None => "\u{270B}".to_string(),
                Some(n) => tone(n)
                    .and_then(|t| emojis::get("\u{270B}").and_then(|e| e.with_skin_tone(t)))
                    .map(|e| e.as_str().to_string())
                    .unwrap_or_default(),
            };
            tones = tones.child(
                div()
                    .id(("picker-tone", i))
                    .px_1()
                    .when(self.skin_tone == n, |d| d.bg(colors.accent_soft))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_skin_tone(n, cx))),
            );
        }

        let grid = div()
            .id("picker-grid")
            .debug_selector(|| "picker-grid".into())
            .w(themes::metrics::SPACE_9 * COLUMNS as f32)
            .child(
                uniform_list("picker-rows", rows, cx.processor(Self::render_rows))
                    .track_scroll(&self.scroll)
                    .w_full()
                    .h(grid_h),
            );
        let mut root = div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.confirm(cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(COLUMNS as isize, cx)))
            .on_action(
                cx.listener(|this, _: &SelectPrev, _, cx| this.step(-(COLUMNS as isize), cx)),
            )
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(PickerDismissed)))
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            // Capped: the grid scrolls, the search, tabs and skin tones stay in view.
            .max_h(themes::metrics::LAYOUT_POPOVER_MAX_HEIGHT)
            .overflow_hidden()
            .rounded(themes::metrics::RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .child(
                div()
                    .h(themes::metrics::LAYOUT_INPUT_MIN)
                    .flex_none()
                    .flex()
                    .items_center()
                    .px_3()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .text_size(themes::metrics::FONT_SIZE_SM)
                    .child(if self.query.is_empty() {
                        div()
                            .text_color(colors.text_muted)
                            .child(t!(cx, "picker-search").into_owned())
                    } else {
                        div().child(self.query.clone())
                    }),
            )
            .child(tabs);
        if !recent.is_empty() {
            let mut row = div().flex().flex_none().gap_1();
            for (i, e) in recent.into_iter().enumerate() {
                let e2 = e.clone();
                row = row.child(
                    div()
                        .id(("picker-recent", i))
                        .debug_selector(move || format!("picker-recent-{i}"))
                        .rounded_md()
                        .when(recent_sel == Some(i), |d| d.bg(colors.accent_soft))
                        .child(self.cell_content(&e, cx))
                        .tooltip(tooltip_shared(format!(":{}:", e.shortcode).into()))
                        .on_click(cx.listener(move |this, _, _, cx| this.pick(e2.clone(), cx))),
                );
            }
            root = root
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(t!(cx, "picker-recent").into_owned()),
                )
                .child(row);
        }
        root.child(grid).child(tones)
    }
}
