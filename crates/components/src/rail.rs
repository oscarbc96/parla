//! The workspace rail: one badge per workspace with mention and unread marks,
//! organisations grouped, switching by click, `cmd-1`..`cmd-9` and next/previous.

use std::rc::Rc;

use domain::{AccountInfo, ImageRef, Session, WorkspaceState};
use gpui::{
    AnyElement, App, FontWeight, Hsla, InteractiveElement as _, IntoElement as _, MouseButton,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    actions, div, prelude::FluentBuilder as _, px, rgb,
};
use settings::WorkspaceKey;
use themes::{ActiveTheme as _, color, parla_tokens};

use crate::aggregate_views::AggregateKind;
use crate::badge::{initials, workspace_badge_sized};

actions!(
    workspace,
    [
        NextWorkspace,
        PreviousWorkspace,
        ActivateWorkspace1,
        ActivateWorkspace2,
        ActivateWorkspace3,
        ActivateWorkspace4,
        ActivateWorkspace5,
        ActivateWorkspace6,
        ActivateWorkspace7,
        ActivateWorkspace8,
        ActivateWorkspace9,
    ]
);

/// One workspace on the rail.
#[derive(Clone, Debug, PartialEq)]
pub struct RailWorkspace {
    pub key: WorkspaceKey,
    pub name: SharedString,
    pub icon: Option<ImageRef>,
    pub initials: SharedString,
    pub mentions: u32,
    pub unread: bool,
    pub active: bool,
    pub expired: bool,
    pub lean: bool,
    /// The user's own notifications are paused (snooze or schedule window).
    pub paused: bool,
    pub accent: Option<Hsla>,
}

/// A rail entry: a workspace, or an organisation with its workspaces nested.
#[derive(Clone, Debug, PartialEq)]
pub enum RailItem {
    Workspace(RailWorkspace),
    Org {
        name: SharedString,
        children: Vec<RailWorkspace>,
    },
}

/// `#rrggbb` only.
pub fn parse_accent(s: &str) -> Option<Hsla> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16)
        .ok()
        // design: allow accent is a user-supplied hex string parsed at runtime
        .map(|v| color::rgb_to_hsla(rgb(v)))
}

/// Rail entries in account order; members of one organisation are grouped at
/// the position of the first.
pub fn rail_items(
    accounts: &[AccountInfo],
    states: &[(WorkspaceKey, &WorkspaceState)],
    active: Option<&WorkspaceKey>,
    accent: &dyn Fn(&WorkspaceKey) -> Option<Hsla>,
    now_unix: i64,
) -> Vec<RailItem> {
    let one = |a: &AccountInfo| {
        let state = states.iter().find(|(k, _)| k == &a.key).map(|(_, s)| *s);
        let totals = state.map(WorkspaceState::totals).unwrap_or_default();
        let state_name = state.map(|s| s.name.as_ref()).filter(|n| !n.is_empty());
        let name = state_name.unwrap_or_else(|| {
            if a.name.is_empty() {
                a.key.id.0.as_str()
            } else {
                &a.name
            }
        });
        RailWorkspace {
            key: a.key.clone(),
            name: SharedString::from(name.to_string()),
            icon: a.icon.clone(),
            initials: SharedString::from(initials(name)),
            mentions: totals.mentions,
            unread: totals.conversations > 0,
            active: active == Some(&a.key),
            expired: state.is_some_and(|s| s.session == Session::Expired),
            lean: state.is_some_and(|s| s.lean),
            paused: state.is_some_and(|s| {
                s.self_user
                    .as_ref()
                    .and_then(|me| s.dnd.get(me))
                    .is_some_and(|d| d.active(now_unix))
            }),
            accent: accent(&a.key),
        }
    };
    let mut items: Vec<RailItem> = Vec::new();
    // Organisation id -> index of its group in `items`.
    let mut groups: Vec<(&str, usize)> = Vec::new();
    for a in accounts {
        let w = one(a);
        let Some(org) = &a.org else {
            items.push(RailItem::Workspace(w));
            continue;
        };
        match groups.iter().find(|(id, _)| *id == &*org.id) {
            Some(&(_, at)) => {
                if let RailItem::Org { children, .. } = &mut items[at] {
                    children.push(w);
                }
            }
            None => {
                groups.push((&org.id, items.len()));
                items.push(RailItem::Org {
                    name: SharedString::from(org.name.to_string()),
                    children: vec![w],
                });
            }
        }
    }
    items
}

/// Every workspace in rail order, organisation children inline.
pub fn flatten(items: &[RailItem]) -> Vec<&RailWorkspace> {
    let mut out = Vec::new();
    for i in items {
        match i {
            RailItem::Workspace(w) => out.push(w),
            RailItem::Org { children, .. } => out.extend(children.iter()),
        }
    }
    out
}

/// The `index`th workspace of the flattened rail.
pub fn key_at(items: &[RailItem], index: usize) -> Option<WorkspaceKey> {
    flatten(items).get(index).map(|w| w.key.clone())
}

/// The workspace `delta` places from the active one, wrapping at both ends.
/// With no active workspace, forward is the first and backward the last.
pub fn key_step(items: &[RailItem], delta: isize) -> Option<WorkspaceKey> {
    let flat = flatten(items);
    let n = flat.len() as isize;
    if n == 0 {
        return None;
    }
    let target = match flat.iter().position(|w| w.active) {
        Some(i) => (i as isize + delta).rem_euclid(n),
        None if delta >= 0 => 0,
        None => n - 1,
    };
    flat.get(target as usize).map(|w| w.key.clone())
}

type KeyFn = Rc<dyn Fn(&WorkspaceKey, &mut Window, &mut App)>;
type ContextFn = Rc<dyn Fn(&WorkspaceKey, gpui::Point<gpui::Pixels>, &mut Window, &mut App)>;
/// A rail button's action (add workspace, Settings, own profile).
pub type AddFn = Rc<dyn Fn(&mut Window, &mut App)>;
type AggregateFn = Rc<dyn Fn(AggregateKind, &mut Window, &mut App)>;

/// What the rail calls back into.
#[derive(Clone)]
pub struct RailActions {
    /// Click on a workspace.
    pub activate: KeyFn,
    /// Right click on a workspace, at the point: the host opens its menu
    /// (pause notifications, remove workspace).
    pub context: ContextFn,
    /// The add-workspace entry.
    pub add: AddFn,
    /// The "All unreads", "All mentions" and "All scheduled" entries.
    pub aggregate: AggregateFn,
    /// The cross-workspace view shown now, if any.
    pub aggregate_open: Option<AggregateKind>,
    /// The Settings button at the bottom: opens the Settings window.
    pub settings: Option<AddFn>,
    /// The signed-in user's avatar under Settings.
    pub me: Option<RailSelf>,
    /// The workspace list's scroll position, kept across frames.
    pub scroll: RailScroll,
}

/// Scroll state of the workspace list: the handle, and the active workspace
/// last scrolled into view with the list's height then, so only a selection
/// or window height change scrolls (never a redraw, which would fight the
/// user's own scrolling).
#[derive(Clone, Default)]
pub struct RailScroll {
    handle: gpui::ScrollHandle,
    seen: Rc<std::cell::RefCell<Option<(WorkspaceKey, i32)>>>,
    pending: Rc<std::cell::Cell<bool>>,
}

impl RailScroll {
    /// The active workspace could not be scrolled into view yet: the list has
    /// no measured bounds (its first frame). The host draws one more frame.
    pub fn pending(&self) -> bool {
        self.pending.replace(false)
    }
}

/// The signed-in user of the shown workspace, at the bottom of the rail.
#[derive(Clone)]
pub struct RailSelf {
    pub name: SharedString,
    /// The avatar's letters, computed once from `name`.
    pub initials: SharedString,
    /// Own presence: active, away, or unknown (no dot).
    pub active: Option<bool>,
    /// Own picture (initials while it loads and when there is none).
    pub avatar: Option<domain::ImageRef>,
    /// Opens the own profile (status and presence).
    pub open: AddFn,
}

impl RailSelf {
    pub fn new(name: SharedString, active: Option<bool>, open: AddFn) -> Self {
        Self {
            initials: initials(&name).into(),
            name,
            active,
            avatar: None,
            open,
        }
    }

    /// With the signed-in user's picture.
    pub fn with_avatar(mut self, avatar: Option<domain::ImageRef>) -> Self {
        self.avatar = avatar;
        self
    }
}

const TILE: gpui::Pixels = themes::metrics::LAYOUT_RAIL_TILE;

fn workspace_entry(w: &RailWorkspace, actions: &RailActions, cx: &App) -> AnyElement {
    let status = cx.theme().status();
    let key = w.key.clone();
    let key_r = w.key.clone();
    let (on_click, on_context) = (actions.activate.clone(), actions.context.clone());
    let tokens = parla_tokens(cx.theme());
    let ring = w.accent.unwrap_or(tokens.sidebar_text_strong);
    let mut badge = div()
        .debug_selector(|| "rail-badge".into())
        .p(themes::metrics::SPACE_1)
        .rounded(themes::metrics::RADIUS_XL)
        .border_2()
        .border_color(if w.active || w.accent.is_some() {
            ring
        } else {
            gpui::transparent_black()
        })
        .child(workspace_badge_sized(
            &w.name,
            w.icon.clone(),
            TILE,
            themes::metrics::RADIUS_LG,
        ));
    if w.lean && !w.active {
        badge = badge.opacity(0.7);
    }
    let entry = div()
        .id(SharedString::from(format!("rail-{}", w.key)))
        .debug_selector({
            let sel = format!("rail-workspace-{}", w.key.id.0);
            move || sel
        })
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .w(themes::metrics::LAYOUT_RAIL)
        .cursor_pointer()
        .tab_stop(true)
        .focusable()
        .focus_visible(crate::controls::focus_style(cx.theme().colors()))
        .on_mouse_down(MouseButton::Right, move |ev, window, cx| {
            on_context(&key_r, ev.position, window, cx)
        })
        .tooltip({
            let name = w.name.clone();
            crate::reactions::tooltip_lazy(move |_| name.to_string())
        })
        .child(badge);
    let mut entry =
        crate::controls::on_activate_app(entry, move |window, cx| on_click(&key, window, cx));
    if w.active {
        entry = entry.child(
            div()
                .debug_selector(|| "rail-active".into())
                .absolute()
                .left_0()
                .w(themes::metrics::SPACE_2)
                .h(themes::metrics::SPACE_9)
                .rounded_r(themes::metrics::SPACE_2)
                .bg(tokens.signal),
        );
    }
    if w.mentions > 0 {
        let label = if w.mentions > 99 {
            "99+".to_string()
        } else {
            w.mentions.to_string()
        };
        entry = entry.child(
            div()
                .debug_selector(|| "rail-mentions".into())
                .absolute()
                .right(themes::metrics::SPACE_2)
                .top(px(-4.0))
                .min_w(px(16.0))
                .h(px(16.0))
                .px(themes::metrics::SPACE_2)
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(tokens.sidebar_rail)
                .bg(status.error)
                .text_color(parla_tokens(cx.theme()).danger_text)
                .text_size(themes::metrics::FONT_SIZE_3XS)
                .font_weight(FontWeight::BOLD)
                .child(label),
        );
    }
    // Beside the mention badge when both apply; never on the active tile.
    if w.unread && !w.active {
        entry = entry.child(
            div()
                .debug_selector(|| "rail-unread".into())
                .absolute()
                .left_0()
                .w(themes::metrics::SPACE_2)
                .h(themes::metrics::SPACE_4)
                .rounded_r(themes::metrics::SPACE_2)
                .bg(tokens.sidebar_muted),
        );
    }
    if w.paused {
        // Two bars on a small disc, top left: notifications are paused.
        let bar = || {
            div()
                .w(px(2.0))
                .h(px(6.0))
                .rounded_sm()
                .bg(tokens.sidebar_text_strong)
        };
        entry = entry.child(
            div()
                .debug_selector(|| "rail-paused".into())
                .absolute()
                .left(px(6.0))
                .top(px(-2.0))
                .size(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(2.0))
                .rounded_full()
                .border_1()
                .border_color(tokens.sidebar_border)
                .bg(tokens.sidebar_rail)
                .child(bar())
                .child(bar()),
        );
    }
    if w.expired {
        entry = entry.child(
            div()
                .debug_selector(|| "rail-expired".into())
                .absolute()
                .left(px(6.0))
                .bottom(px(-2.0))
                .size(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(status.warning)
                .text_color(parla_tokens(cx.theme()).warning_text)
                .text_size(themes::metrics::FONT_SIZE_3XS)
                .font_weight(FontWeight::BOLD)
                .child("!"),
        );
    }
    entry.into_any_element()
}

/// The three cross-workspace entries above the workspaces.
fn aggregate_entries(actions: &RailActions, cx: &App) -> AnyElement {
    let tokens = parla_tokens(cx.theme());
    let mut col = div()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .pb(themes::metrics::SPACE_5)
        .border_b_1()
        .border_color(tokens.sidebar_border)
        .w(themes::metrics::LAYOUT_RAIL_CONTROL);
    for (kind, glyph, sel) in [
        (AggregateKind::Unreads, "\u{25CF}", "rail-all-unreads"),
        (AggregateKind::Mentions, "@", "rail-all-mentions"),
        (AggregateKind::Scheduled, "\u{2026}", "rail-all-scheduled"),
    ] {
        let on = actions.aggregate.clone();
        let open = actions.aggregate_open == Some(kind);
        col = col.child(
            div()
                .id(sel)
                .debug_selector(move || sel.into())
                .size(px(32.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .cursor_pointer()
                .text_sm()
                .text_color(if open {
                    tokens.sidebar_text_strong
                } else {
                    tokens.sidebar_muted
                })
                .when(open, |d| d.bg(tokens.sidebar_hover))
                .hover(|h| {
                    h.bg(tokens.sidebar_hover)
                        .text_color(tokens.sidebar_text_strong)
                })
                .on_click(move |_, window, cx| on(kind, window, cx))
                .tooltip(crate::reactions::tooltip_lazy(move |cx| {
                    let l = i18n::gpui::localizer(cx);
                    match kind {
                        AggregateKind::Unreads => t!(l, "rail-all-unreads"),
                        AggregateKind::Mentions => t!(l, "rail-all-mentions"),
                        AggregateKind::Scheduled => t!(l, "rail-all-scheduled"),
                    }
                    .into_owned()
                }))
                .child(glyph),
        );
    }
    col.into_any_element()
}

/// The rail's workspace column: a scrolling list (the cross-workspace entries,
/// then badges with organisations boxed) and the add-workspace entry pinned
/// below it, so it stays reachable with any number of workspaces. The caller
/// puts both in a fixed-height flex column.
pub fn render_rail(items: &[RailItem], actions: &RailActions, cx: &App) -> Vec<AnyElement> {
    let tokens = parla_tokens(cx.theme());
    let mut out = vec![aggregate_entries(actions, cx)];
    // The list's measured height from the last frame, read once.
    let height = f32::from(actions.scroll.handle.bounds().size.height).round() as i32;
    for item in items {
        let active = match item {
            RailItem::Workspace(w) => w.active.then_some(&w.key),
            RailItem::Org { children, .. } => children.iter().find(|w| w.active).map(|w| &w.key),
        };
        if let Some(key) = active {
            // Child 0 of the list is the aggregate block.
            let at = out.len();
            let mut seen = actions.scroll.seen.borrow_mut();
            if seen.as_ref().is_none_or(|(k, h)| k != key || *h != height) {
                // gpui runs the scroll request in the list's prepaint against
                // the viewport of the previous frame, so before the first
                // layout (height 0) it is consumed against a zero-size
                // viewport and does nothing. Wait for a measured height; the
                // unmeasured attempt is recorded, so the host redraws once per
                // change, never per frame.
                if height > 0 {
                    actions.scroll.handle.scroll_to_item(at);
                } else {
                    actions.scroll.pending.set(true);
                }
                *seen = Some((key.clone(), height));
            }
        }
        match item {
            RailItem::Workspace(w) => out.push(workspace_entry(w, actions, cx)),
            RailItem::Org { name, children } => {
                out.push(
                    div()
                        .id(SharedString::from(format!("rail-org-{name}")))
                        .debug_selector({
                            let sel = format!("rail-org-{name}");
                            move || sel
                        })
                        .py_1()
                        .w(themes::metrics::LAYOUT_RAIL)
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .rounded_xl()
                        .bg(tokens.sidebar_hover)
                        .tooltip(crate::reactions::tooltip_view(name.to_string()))
                        .children(children.iter().map(|w| workspace_entry(w, actions, cx)))
                        .into_any_element(),
                );
            }
        }
    }
    let add = actions.add.clone();
    let list = div()
        .id("rail-list")
        .debug_selector(|| "rail-list".into())
        .flex()
        .flex_col()
        .items_center()
        .gap(themes::metrics::SPACE_5)
        .w(themes::metrics::LAYOUT_RAIL)
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&actions.scroll.handle)
        .children(out)
        .into_any_element();
    let mut out = vec![list];
    out.push(
        crate::controls::on_activate_app(
            div()
                .id("rail-add")
                .flex_none()
                .debug_selector(|| "rail-add".into())
                .size(TILE)
                .flex()
                .items_center()
                .justify_center()
                .rounded(themes::metrics::RADIUS_LG)
                .border_2()
                .border_dashed()
                .border_color(tokens.sidebar_muted)
                .text_color(tokens.sidebar_muted)
                .cursor_pointer()
                .tab_stop(true)
                .focusable()
                .focus_visible(crate::controls::focus_style(cx.theme().colors()))
                .tooltip(crate::reactions::tooltip_lazy(|cx| {
                    let l = i18n::gpui::localizer(cx);
                    t!(l, "rail-add-workspace").into_owned()
                }))
                .child("+"),
            move |window, cx| add(window, cx),
        )
        .into_any_element(),
    );
    if actions.settings.is_none() && actions.me.is_none() {
        return out;
    }
    // Settings and the own avatar sit at the bottom of the rail.
    if let Some(open) = actions.settings.clone() {
        out.push(
            crate::controls::on_activate_app(
                div()
                    .id("rail-settings")
                    .debug_selector(|| "rail-settings".into())
                    .flex_none()
                    // An auto top margin pushes it (and the avatar) to the bottom.
                    .mt_auto()
                    .size(themes::metrics::SPACE_10)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(themes::metrics::RADIUS_LG)
                    .cursor_pointer()
                    .hover(|h| h.bg(tokens.sidebar_hover))
                    .tooltip(crate::reactions::tooltip_lazy(|cx| {
                        let l = i18n::gpui::localizer(cx);
                        t!(l, "rail-settings").into_owned()
                    }))
                    .child(
                        crate::icon::Icon::new(crate::icon::IconName::Sliders)
                            .size(themes::metrics::ICON_SIZE_XL)
                            .color(tokens.sidebar_muted),
                    )
                    .tab_stop(true)
                    .focusable()
                    .focus_visible(crate::controls::focus_style(cx.theme().colors())),
                move |window, cx| open(window, cx),
            )
            .into_any_element(),
        );
    }
    if let Some(me) = actions.me.as_ref() {
        let name = me.name.clone();
        let open = me.open.clone();
        out.push(
            crate::controls::on_activate_app(
                crate::message_row::avatar_with_presence(
                    &me.name,
                    me.initials.clone(),
                    cx.theme(),
                    themes::metrics::LAYOUT_MSG_AVATAR,
                    me.active,
                    ("rail-self-presence", tokens.sidebar_rail),
                    me.avatar.as_ref(),
                )
                .id("rail-self")
                .debug_selector(|| "rail-self".into())
                .when(actions.settings.is_none(), |d| d.mt_auto())
                .cursor_pointer()
                .tooltip(crate::reactions::tooltip_lazy(move |cx| {
                    let l = i18n::gpui::localizer(cx);
                    t!(l, "rail-self", name = name.to_string()).into_owned()
                }))
                .tab_stop(true)
                .focusable()
                .focus_visible(crate::controls::focus_style(cx.theme().colors())),
                move |window, cx| open(window, cx),
            )
            .into_any_element(),
        );
    }
    out
}
