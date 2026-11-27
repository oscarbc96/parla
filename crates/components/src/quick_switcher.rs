//! Quick switcher (`workspace::QuickSwitch`): fuzzy jump to a conversation in any workspace.

use domain::{ConversationId, UserId};
use gpui::{AnyElement, App, SharedString, Window, actions, div, prelude::*};
use settings::WorkspaceKey;
use themes::ActiveTheme;
use themes::metrics::{
    FONT_SIZE_2XS, FONT_SIZE_MD, FONT_SIZE_XS, FONT_WEIGHT_BOLD, FONT_WEIGHT_REGULAR,
    FONT_WEIGHT_SEMIBOLD, RADIUS_FULL, SPACE_3, SPACE_4, SPACE_5, SPACE_9,
};

use crate::emoji_picker::fuzzy_score;
use crate::picker::{Picker, PickerDelegate};

actions!(workspace, [QuickSwitch, GoBack, GoForward]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchTarget {
    Conversation(ConversationId),
    Person(UserId),
}

/// What the switcher was opened for: the host builds the items to match, the
/// switcher names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwitchScope {
    /// `workspace::QuickSwitch`: every conversation and person.
    #[default]
    All,
    /// The sidebar's New message: a channel or person to write to.
    Compose,
    /// The Channels "+": channels only.
    Channels,
    /// The Direct messages "+": people and direct messages only.
    People,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchItem {
    pub workspace: WorkspaceKey,
    pub target: SwitchTarget,
    pub label: SharedString,
    pub workspace_name: SharedString,
    pub unread: bool,
    pub mentions: u32,
    /// The user is in it (always for people); a channel found by browsing
    /// is not, and its row says so.
    pub joined: bool,
}

/// Most matches kept; the picker shows a window of them.
const MAX_MATCHES: usize = 50;

/// Indices into `items`, best first: fuzzy score of the label, plus a bonus for
/// the conversations opened lately (newest most) and for unread ones. An empty
/// query lists recent, then unread, then the rest. Equal scores keep the order
/// of `items`, so a caller lists the active workspace first.
pub fn rank(
    items: &[SwitchItem],
    query: &str,
    recent: &[(WorkspaceKey, ConversationId)],
) -> Vec<usize> {
    let query = query.trim();
    let recent_pos = |it: &SwitchItem| match &it.target {
        SwitchTarget::Conversation(c) => recent
            .iter()
            .position(|(w, rc)| *w == it.workspace && rc == c),
        SwitchTarget::Person(_) => None,
    };
    let mut scored: Vec<(i64, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            let fuzzy = if query.is_empty() {
                0
            } else {
                i64::from(fuzzy_score(query, &it.label)?)
            };
            let recent = recent_pos(it);
            let bonus = if query.is_empty() {
                match recent {
                    Some(p) => 2000 - p as i64,
                    None if it.unread || it.mentions > 0 => 1000,
                    None => 0,
                }
            } else {
                recent.map_or(0, |p| 10 + (20i64 - p as i64).max(0) / 2)
                    + if it.unread || it.mentions > 0 { 5 } else { 0 }
            };
            Some((fuzzy + bonus, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

type PasteHandler =
    std::rc::Rc<dyn Fn(String, &mut gpui::Window, &mut App) -> crate::picker::PasteOutcome>;

pub struct SwitchDelegate {
    items: Vec<SwitchItem>,
    /// What the provider found for the query, not already in `items`.
    found: Vec<SwitchItem>,
    recent: Vec<(WorkspaceKey, ConversationId)>,
    matches: Vec<SwitchItem>,
    chosen: Option<SwitchItem>,
    on_paste: Option<PasteHandler>,
    scope: SwitchScope,
    /// The mark of a channel the user is not in, built with the matches (not
    /// per row per frame).
    not_joined: SharedString,
}

impl SwitchDelegate {
    /// `items` lists the active workspace first; `recent` is newest first.
    pub fn new(items: Vec<SwitchItem>, recent: Vec<(WorkspaceKey, ConversationId)>) -> Self {
        let mut d = Self {
            items,
            found: Vec::new(),
            recent,
            matches: Vec::new(),
            chosen: None,
            on_paste: None,
            scope: SwitchScope::All,
            not_joined: SharedString::default(),
        };
        d.refresh("");
        d
    }

    pub fn with_scope(mut self, scope: SwitchScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn scope(&self) -> SwitchScope {
        self.scope
    }

    pub fn with_paste_handler(mut self, f: PasteHandler) -> Self {
        self.on_paste = Some(f);
        self
    }

    fn refresh(&mut self, query: &str) {
        let mut matches: Vec<SwitchItem> = rank(&self.items, query, &self.recent)
            .into_iter()
            .take(MAX_MATCHES)
            .map(|i| self.items[i].clone())
            .collect();
        // Found channels follow what the user is in.
        let room = MAX_MATCHES.saturating_sub(matches.len());
        matches.extend(
            rank(&self.found, query, &[])
                .into_iter()
                .take(room)
                .map(|i| self.found[i].clone()),
        );
        self.matches = matches;
    }

    /// Channels the provider found for the query (not in `items`); replaces
    /// what the last search found.
    pub fn set_found(&mut self, mut found: Vec<SwitchItem>) {
        found.retain(|f| {
            !self
                .items
                .iter()
                .any(|i| i.workspace == f.workspace && i.target == f.target)
        });
        self.found = found;
    }

    /// The item the user confirmed.
    pub fn chosen(&self) -> Option<&SwitchItem> {
        self.chosen.as_ref()
    }
}

impl PickerDelegate for SwitchDelegate {
    type Item = SwitchItem;

    fn update_matches(&mut self, query: &str, cx: &mut App) {
        self.refresh(query);
        let mark = t!(cx, "switcher-not-joined");
        if self.not_joined != mark.as_ref() {
            self.not_joined = mark.into_owned().into();
        }
    }

    fn matches(&self) -> &[SwitchItem] {
        &self.matches
    }

    fn render_item(&self, i: usize, selected: bool, cx: &mut App) -> AnyElement {
        let colors = *cx.theme().colors();
        let Some(it) = self.matches.get(i) else {
            return div().into_any_element();
        };
        let mut name = div()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .min_w_0()
            .text_size(FONT_SIZE_MD)
            .when(selected, |d| d.text_color(colors.text_accent))
            .child(
                div()
                    .truncate()
                    .font_weight(if it.unread {
                        FONT_WEIGHT_BOLD
                    } else {
                        FONT_WEIGHT_REGULAR
                    })
                    .child(it.label.clone()),
            );
        if it.mentions > 0 {
            name = name.child(
                div()
                    .flex_none()
                    .text_size(FONT_SIZE_2XS)
                    .font_weight(FONT_WEIGHT_SEMIBOLD)
                    .px(SPACE_3)
                    .rounded(RADIUS_FULL)
                    .bg(colors.text_accent)
                    .text_color(colors.background)
                    .child(it.mentions.to_string()),
            );
        }
        let mut side = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .text_size(FONT_SIZE_XS)
            .text_color(colors.text_muted);
        if !it.joined {
            side = side.child(
                div()
                    .debug_selector(move || format!("switcher-not-joined-{i}"))
                    // text.subtle: 4.7:1 or more on the list and its selected row.
                    .text_color(themes::parla_tokens(cx.theme()).text_subtle)
                    .child(self.not_joined.clone()),
            );
        }
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(SPACE_5)
            .h(SPACE_9)
            .child(name)
            .child(side.child(it.workspace_name.clone()))
            .into_any_element()
    }

    fn confirm(&mut self, i: usize, _cx: &mut App) {
        self.chosen = self.matches.get(i).cloned();
    }

    fn placeholder(&self, cx: &App) -> String {
        match self.scope {
            SwitchScope::All => t!(cx, "switcher-placeholder"),
            SwitchScope::Compose => t!(cx, "switcher-placeholder-compose"),
            SwitchScope::Channels => t!(cx, "switcher-placeholder-channels"),
            SwitchScope::People => t!(cx, "switcher-placeholder-people"),
        }
        .into_owned()
    }

    fn empty_text(&self, cx: &App) -> String {
        match self.scope {
            SwitchScope::All | SwitchScope::Compose => t!(cx, "switcher-empty"),
            SwitchScope::Channels => t!(cx, "switcher-empty-channels"),
            SwitchScope::People => t!(cx, "switcher-empty-people"),
        }
        .into_owned()
    }

    fn paste(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> crate::picker::PasteOutcome {
        if let Some(f) = &self.on_paste {
            f(text.to_string(), window, cx)
        } else {
            crate::picker::PasteOutcome::Insert
        }
    }
}

pub type QuickSwitcher = Picker<SwitchDelegate>;
