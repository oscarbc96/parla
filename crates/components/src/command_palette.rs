//! Command palette: fuzzy search over actions, dispatches the chosen one.

use gpui::{AnyElement, App, SharedString, div, prelude::*};
use themes::ActiveTheme;
use themes::metrics::{FONT_SIZE_MD, FONT_SIZE_XS, FONT_WEIGHT_MEDIUM, SPACE_5, SPACE_9};

use crate::emoji_picker::fuzzy_score;
use crate::picker::{Picker, PickerDelegate};

/// One action shown in the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandItem {
    /// Stable action name (e.g. `app::Quit`).
    pub name: SharedString,
    /// Localized title shown to the user.
    pub title: SharedString,
    /// Display string of the first keybinding, if the action is bound.
    pub binding: Option<SharedString>,
}

/// Whether `name` belongs in the command palette. Text-input editing actions
/// and the picker's own actions are internal and never listed.
pub fn is_palette_action(name: &str) -> bool {
    !name.starts_with("text_input::") && !name.starts_with("picker::")
}

/// The subset of `known` action names the palette lists, in order.
pub fn palette_action_names<'a>(known: &[&'a str]) -> Vec<&'a str> {
    known
        .iter()
        .copied()
        .filter(|n| is_palette_action(n))
        .collect()
}

/// A picker delegate over actions.
pub struct CommandDelegate {
    items: Vec<CommandItem>,
    matches: Vec<CommandItem>,
    chosen: Option<CommandItem>,
}

impl CommandDelegate {
    /// `items` are the actions to list; pass only palette actions.
    pub fn new(items: Vec<CommandItem>) -> Self {
        let mut d = Self {
            items,
            matches: Vec::new(),
            chosen: None,
        };
        d.refresh("");
        d
    }

    fn refresh(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            self.matches = self.items.clone();
            return;
        }
        let mut scored: Vec<(i32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                let by_title = fuzzy_score(query, &it.title);
                let by_name = fuzzy_score(query, &it.name);
                let score = match (by_title, by_name) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (a, b) => a.or(b),
                }?;
                Some((score, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.matches = scored
            .into_iter()
            .map(|(_, i)| self.items[i].clone())
            .collect();
    }

    /// The item the user confirmed.
    pub fn chosen(&self) -> Option<&CommandItem> {
        self.chosen.as_ref()
    }
}

impl PickerDelegate for CommandDelegate {
    type Item = CommandItem;

    fn update_matches(&mut self, query: &str, _cx: &mut App) {
        self.refresh(query);
    }

    fn matches(&self) -> &[CommandItem] {
        &self.matches
    }

    fn render_item(&self, i: usize, _selected: bool, cx: &mut App) -> AnyElement {
        let colors = *cx.theme().colors();
        let Some(it) = self.matches.get(i) else {
            return div().into_any_element();
        };
        let mut row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(SPACE_5)
            .w_full()
            .h(SPACE_9)
            .child(
                div()
                    .text_size(FONT_SIZE_MD)
                    .font_weight(FONT_WEIGHT_MEDIUM)
                    .min_w_0()
                    .child(it.title.clone()),
            );
        if let Some(binding) = &it.binding {
            row = row.child(
                div()
                    .flex_none()
                    .text_size(FONT_SIZE_XS)
                    .text_color(colors.text_muted)
                    .child(binding.clone()),
            );
        }
        row.into_any_element()
    }

    fn confirm(&mut self, i: usize, _cx: &mut App) {
        self.chosen = self.matches.get(i).cloned();
    }

    fn placeholder(&self, cx: &App) -> String {
        t!(cx, "command-palette-placeholder").into_owned()
    }

    fn empty_text(&self, cx: &App) -> String {
        t!(cx, "command-palette-empty").into_owned()
    }
}

/// The command palette view.
pub type CommandPalette = Picker<CommandDelegate>;
