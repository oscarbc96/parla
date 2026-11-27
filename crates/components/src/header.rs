//! The strip above the timeline: the open conversation's name, topic and
//! member count.

use domain::{Conversation, ConversationKind, WorkspaceState};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use themes::{Theme, parla_tokens};

use crate::rich_text::{OpenConversation, OpenUser, RichColors, RichTextCx, render_rich_text};
use crate::sidebar::{MemberIndex, conversation_label, group_member_names, presence_dot};
use crate::timeline::TimelineSource;

/// Height of the header strip (the row with the name and the buttons).
pub const HEIGHT: gpui::Pixels = themes::metrics::LAYOUT_HEADER;

/// Height of the whole header: the strip plus the tab row. Anchors under the
/// header (the pins popover) start here.
pub const TOTAL_HEIGHT: gpui::Pixels = themes::metrics::LAYOUT_HEADER_TOTAL;

/// The header text parts for a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderInfo {
    /// Name with its `# ` or lock prefix.
    pub title: String,
    /// Empty when the conversation has no topic.
    pub topic: String,
    pub member_count: Option<u32>,
    /// A direct message's peer is active (`Some(true)`) or away; `None` when
    /// unknown or not a direct message.
    pub presence: Option<bool>,
}

/// The peer's presence for a direct message (`None` for other kinds or unknown).
pub fn dm_presence(state: &WorkspaceState, c: &Conversation) -> Option<bool> {
    match (&c.kind, &c.peer) {
        (ConversationKind::Direct, Some(p)) => state.presence.get(p).copied(),
        _ => None,
    }
}

/// The line under a direct message's name: the peer's presence, their local
/// time (when their profile has a zone) and their status, joined by a middle
/// dot. `None` for other kinds, or when nothing is known.
pub fn dm_subtitle(
    state: &WorkspaceState,
    c: &Conversation,
    now: jiff::Timestamp,
    clock: settings::Clock,
    l: &i18n::Localizer,
) -> Option<String> {
    let (ConversationKind::Direct, Some(peer)) = (&c.kind, &c.peer) else {
        return None;
    };
    let profile = state.profiles.get(peer);
    let mut parts = Vec::new();
    if let Some(active) = state.presence.get(peer) {
        parts.push(if *active {
            t!(l, "profile-presence-active").into_owned()
        } else {
            t!(l, "profile-presence-away").into_owned()
        });
    }
    if let Some((name, tz)) = profile
        .and_then(|p| p.tz.as_deref())
        .and_then(|z| Some((z, jiff::tz::TimeZone::get(z).ok()?)))
    {
        let time = l.time(now, &tz, crate::message_row::hour_cycle(clock));
        // The zone's city: "Asia/Tokyo" is Tokyo, "America/New_York" New York.
        let city = name
            .rsplit_once('/')
            .map(|(_, c)| c.replace('_', " "))
            .filter(|c| !c.is_empty());
        parts.push(
            match city {
                Some(city) => t!(l, "header-dm-local-time-city", time = time, city = city),
                None => t!(l, "header-dm-local-time", time = time),
            }
            .into_owned(),
        );
    }
    let status = state
        .members
        .get(peer)
        .and_then(|m| m.status.as_ref())
        .or_else(|| profile.and_then(|p| p.status.as_ref()))
        .filter(|s| domain::status_visible(s, now.as_second()))
        .filter(|s| s.emoji.is_some() || !s.text.trim().is_empty());
    if let Some(s) = status {
        parts.push(match (&s.emoji, s.text.trim()) {
            (Some(e), "") => crate::reactions::glyph(e),
            (Some(e), t) => format!("{} {t}", crate::reactions::glyph(e)),
            (None, t) => t.to_string(),
        });
    }
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

/// What the header shows for `c`.
pub fn header_info(state: &WorkspaceState, c: &Conversation, l: &i18n::Localizer) -> HeaderInfo {
    // The handle index (a walk over every member) is only read when the group's
    // member ids do not resolve; build it just then.
    let name = match c.kind {
        ConversationKind::Group if group_member_names(state, c).is_none() => {
            conversation_label(state, &MemberIndex::build(state), c, l)
        }
        _ => conversation_label(state, &MemberIndex::default(), c, l),
    };
    let prefix = match c.kind {
        ConversationKind::Channel => "# ",
        ConversationKind::PrivateChannel => "\u{1F512} ",
        ConversationKind::Direct | ConversationKind::Group => "",
    };
    HeaderInfo {
        title: format!("{prefix}{name}"),
        topic: c.topic.trim().to_string(),
        presence: dm_presence(state, c),
        member_count: match c.kind {
            ConversationKind::Direct => None,
            // Exact when the provider lists the members (the signed-in user is
            // one of them); otherwise the names in the group's title plus self.
            ConversationKind::Group if c.member_count.is_none() && !c.member_ids.is_empty() => {
                Some(c.member_ids.len() as u32)
            }
            ConversationKind::Group if c.member_count.is_none() => {
                // The participants may or may not list the signed-in user.
                let me = state
                    .self_user
                    .as_ref()
                    .and_then(|u| state.members.get(u))
                    .map(|m| &m.handle)
                    .filter(|h| !h.is_empty());
                let others = c.participants.iter().filter(|p| Some(*p) != me).count();
                (!c.participants.is_empty()).then_some(others as u32 + 1)
            }
            _ => c.member_count,
        },
    }
}

/// The topic on one line: parsed with the provider's markup, so links, mentions,
/// channels and emoji render like in messages; it ends in an ellipsis.
fn topic_element(
    source: &dyn TimelineSource,
    topic: &str,
    theme: &Theme,
    actions: &HeaderActions,
    cx: &App,
) -> AnyElement {
    let rich = source.content().parse_text(topic);
    let mut rcx = RichTextCx::new(
        source,
        RichColors::from_theme(theme),
        i18n::gpui::localizer(cx),
        crate::time_zone(cx),
    );
    rcx.on_open_conversation = actions.conversation.clone();
    rcx.on_open_user = actions.user.clone();
    if cx.has_global::<crate::media::ImageCache>() {
        rcx.emoji = Some(&crate::media::emoji_hook);
    }
    div()
        .debug_selector(|| "conversation-topic".into())
        .flex_1()
        .min_w_0()
        .max_h(px(20.0))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(themes::metrics::FONT_SIZE_XS)
        .text_color(parla_tokens(theme).text_subtle)
        .child(render_rich_text(&rich, &rcx))
        .into_any_element()
}

/// What a click on a channel link or a mention in the topic does, and what the
/// header's buttons do. A button without an action is not drawn.
#[derive(Clone, Default)]
pub struct HeaderActions {
    pub conversation: Option<OpenConversation>,
    pub user: Option<OpenUser>,
    /// The bell: opens the notification controls (level and mute) of the
    /// conversation under the button.
    pub notify_menu: Option<NotifyMenuFn>,
    /// The More button: opens the conversation's action menu (details, pop out)
    /// under the button. It does not repeat the notification controls.
    pub more_menu: Option<NotifyMenuFn>,
    /// The pins button (present only when the provider has pins): toggles the
    /// conversation's pins popover.
    pub pins: Option<PinsFn>,
    /// The star button: toggles the conversation's star. Present only when the
    /// provider keeps stars; `starred` is its state.
    pub star: Option<ToggleFn>,
    pub starred: bool,
    /// The details button: toggles the details panel; `details_open` marks it
    /// as pressed.
    pub details: Option<ToggleFn>,
    pub details_open: bool,
    /// The member count and the avatar stack: open the details panel on its
    /// Members tab (and keep it open when it already is).
    pub members: Option<ToggleFn>,
    /// The Huddle button. The host passes it only where the huddle bar would
    /// offer Start or Join (nothing while the user is in that huddle or its
    /// call is open); it runs the bar's own action.
    pub huddle: Option<ToggleFn>,
    /// The clock a direct message peer's local time is shown with; `None`
    /// is the 24-hour clock.
    pub clock: Option<settings::Clock>,
    /// The tab shown under the header.
    pub tab: ConversationTab,
    /// Switches the tab. Without it only the Messages tab is drawn.
    pub switch_tab: Option<TabFn>,
}

/// The views of a conversation under its header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConversationTab {
    #[default]
    Messages,
    /// The files shared in the conversation.
    Files,
}

/// Shows a tab of the open conversation.
pub type TabFn = std::rc::Rc<dyn Fn(ConversationTab, &mut gpui::Window, &mut App)>;

/// Runs an action for a conversation (toggle pins, star or details, show its
/// members, start or join its huddle).
pub type ToggleFn = std::rc::Rc<dyn Fn(&domain::ConversationId, &mut gpui::Window, &mut App)>;

/// Toggles the pins popover of a conversation.
pub type PinsFn = ToggleFn;

/// Opens a menu of a conversation at a point: the bottom left of the button
/// that was activated.
pub type NotifyMenuFn = std::rc::Rc<
    dyn Fn(&domain::ConversationId, gpui::Point<gpui::Pixels>, &mut gpui::Window, &mut App),
>;

/// What a header icon button runs when activated.
type ButtonFn = Box<dyn Fn(&mut gpui::Window, &mut App)>;

/// The bounds a header button was last painted at, for anchoring its menu.
type Anchor = std::rc::Rc<std::cell::Cell<gpui::Bounds<gpui::Pixels>>>;

/// Records the painted bounds of `el` in `anchor`.
fn track_bounds(el: gpui::Stateful<gpui::Div>, anchor: Anchor) -> gpui::Stateful<gpui::Div> {
    el.relative().child(
        gpui::canvas(|_, _, _| (), move |bounds, _, _, _| anchor.set(bounds))
            .absolute()
            .size_full(),
    )
}

/// The point a menu opens at: under the button's left edge.
pub fn menu_anchor(b: gpui::Bounds<gpui::Pixels>) -> gpui::Point<gpui::Pixels> {
    gpui::point(b.left(), b.bottom() + themes::metrics::SPACE_2)
}

/// People whose avatars the header stacks, at most.
const STACK_MAX: usize = 3;

/// A text-and-icon button on the header strip (Huddle, the member count): a tab
/// stop with the focus ring that Enter and Space activate.
fn text_button(
    id: &'static str,
    label: String,
    lead: AnyElement,
    colors: &themes::ThemeColors,
    on_activate: impl Fn(&mut gpui::Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let hover = colors.element_hover;
    crate::controls::on_activate_app(
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .flex()
            .flex_none()
            .items_center()
            .gap(themes::metrics::SPACE_3)
            .h(themes::metrics::LAYOUT_TITLEBAR_BUTTON)
            .px(themes::metrics::SPACE_4)
            .rounded(themes::metrics::RADIUS_MD)
            .text_size(themes::metrics::FONT_SIZE_SM)
            .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
            .text_color(colors.text)
            .cursor_pointer()
            .tab_stop(true)
            .focusable()
            .focus_visible(crate::controls::focus_style(colors))
            .hover(move |d| d.bg(hover))
            .child(lead)
            .child(label),
        on_activate,
    )
}

/// The header element for `c`.
pub fn conversation_header(
    source: &dyn TimelineSource,
    state: &WorkspaceState,
    c: &Conversation,
    theme: &Theme,
    actions: &HeaderActions,
    cx: &App,
) -> AnyElement {
    use crate::controls::{IconButtonSize, icon_button_styled};
    use crate::icon::{Icon, IconName};
    let l = i18n::gpui::localizer(cx);
    let info = header_info(state, c, &l);
    let colors = theme.colors();
    let tokens = parla_tokens(theme);
    let (kind_icon, name) = match c.kind {
        ConversationKind::Channel => (
            Some(IconName::Hash),
            info.title.strip_prefix("# ").unwrap_or(&info.title),
        ),
        ConversationKind::PrivateChannel => (
            Some(IconName::Lock),
            info.title.strip_prefix("\u{1F512} ").unwrap_or(&info.title),
        ),
        ConversationKind::Direct | ConversationKind::Group => (None, info.title.as_str()),
    };
    let button = |id: &'static str,
                  icon: IconName,
                  label: String,
                  on: bool,
                  color: Option<gpui::Hsla>,
                  f: ButtonFn| {
        icon_button_styled(id, icon, label, IconButtonSize::Large, color, on, colors, f)
            .debug_selector(move || id.to_string())
    };
    // A direct message shows its peer: an avatar with the presence dot, and a
    // line with presence, local time and status under the name.
    let peer = match c.kind {
        ConversationKind::Direct => c.peer.clone(),
        _ => None,
    };
    let subtitle = peer.as_ref().and_then(|_| {
        dm_subtitle(
            state,
            c,
            crate::now(cx),
            actions.clock.unwrap_or(settings::Clock::H24),
            &l,
        )
    });
    let mut title = div()
        .flex()
        .items_center()
        .gap(themes::metrics::SPACE_3)
        .min_w_0()
        .when_some(info.presence.filter(|_| peer.is_none()), |d, active| {
            d.child(presence_dot(
                "header-presence",
                active,
                tokens.presence_active,
                tokens.presence_away,
            ))
        })
        .when_some(kind_icon, |d, icon| {
            d.child(
                Icon::new(icon)
                    .size(themes::metrics::ICON_SIZE_LG)
                    .color(colors.text_muted),
            )
        })
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(themes::metrics::FONT_SIZE_XL)
                .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                .child(name.to_string()),
        );
    if let Some(toggle) = actions.star.clone() {
        let id = c.id.clone();
        let label = if actions.starred {
            t!(&l, "header-unstar")
        } else {
            t!(&l, "header-star")
        }
        .into_owned();
        title = title.child(button(
            "header-star",
            IconName::Star,
            label,
            false,
            actions.starred.then(|| theme.status().warning),
            Box::new(move |window, cx| toggle(&id, window, cx)),
        ));
    }
    let mut left = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(title)
        .when_some(subtitle, |d, line| {
            d.child(
                div().flex().child(
                    div()
                        .debug_selector(|| "header-dm-subtitle".into())
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(themes::metrics::FONT_SIZE_XS)
                        .text_color(tokens.text_subtle)
                        .child(line),
                ),
            )
        })
        .when(!info.topic.is_empty(), |d| {
            // A row of its own: the one-line ellipsis layout is cheap there and
            // allocates ten times more as a bare child of the column.
            d.child(
                div()
                    .flex()
                    .child(topic_element(source, &info.topic, theme, actions, cx)),
            )
        });
    left = left.debug_selector(|| "conversation-title".into());
    let mut row = div()
        .debug_selector(|| "conversation-header".into())
        .flex()
        .flex_none()
        .items_center()
        .gap(themes::metrics::SPACE_4)
        .w_full()
        .h(HEIGHT)
        .px(themes::metrics::SPACE_7)
        .bg(colors.surface_background)
        .border_b_1()
        .border_color(tokens.border_subtle)
        .when(peer.is_some(), |d| {
            d.child(
                div().debug_selector(|| "header-dm-avatar".into()).child(
                    crate::message_row::avatar_with_presence(
                        name,
                        crate::badge::initials(name).into(),
                        theme,
                        themes::metrics::LAYOUT_AVATAR_SM,
                        info.presence,
                        ("header-presence", colors.surface_background),
                        peer.as_ref()
                            .and_then(|u| state.members.get(u))
                            .and_then(|m| m.avatar_for(themes::metrics::LAYOUT_AVATAR_SM.as_f32())),
                    ),
                ),
            )
        })
        .child(left);
    if let Some(n) = info.member_count {
        let (people, recent) = people_for_stack(state, c);
        let count = t!(&l, "header-members", count = n).into_owned();
        let tip = if recent {
            t!(&l, "header-members-recent", members = count.clone()).into_owned()
        } else {
            count.clone()
        };
        let mut stack = div().flex().items_center();
        for (i, (name, picture)) in people.iter().enumerate() {
            stack = stack.child(
                div()
                    .when(i > 0, |d| d.ml(-themes::metrics::SPACE_3))
                    .rounded(themes::metrics::RADIUS_AVATAR)
                    .border_2()
                    .border_color(colors.surface_background)
                    .child(crate::message_row::avatar_sized(
                        name,
                        theme,
                        themes::metrics::LAYOUT_AVATAR_SM,
                        picture.as_ref(),
                    )),
            );
        }
        let lead = stack.into_any_element();
        let n_text = n.to_string();
        row = match actions.members.clone() {
            Some(open) => {
                let id = c.id.clone();
                row.child(
                    text_button("header-members", n_text, lead, colors, move |window, cx| {
                        open(&id, window, cx)
                    })
                    .tooltip(crate::reactions::tooltip_view(tip)),
                )
            }
            None => row.child(
                div()
                    .id("header-members")
                    .debug_selector(|| "header-members".into())
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(themes::metrics::SPACE_3)
                    .text_size(themes::metrics::FONT_SIZE_SM)
                    .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                    .text_color(colors.text_muted)
                    .tooltip(crate::reactions::tooltip_view(tip))
                    .child(lead)
                    .child(n_text),
            ),
        };
    }
    if let Some(join) = actions.huddle.clone() {
        let id = c.id.clone();
        row = row.child(text_button(
            "header-huddle",
            t!(&l, "header-huddle").into_owned(),
            Icon::new(IconName::Headphones)
                .size(themes::metrics::ICON_SIZE_MD)
                .color(colors.text)
                .into_any_element(),
            colors,
            move |window, cx| join(&id, window, cx),
        ));
    }
    if let (Some(user), Some(open)) = (peer.clone(), actions.user.clone()) {
        row = row.child(button(
            "header-profile",
            IconName::User,
            t!(&l, "header-profile").into_owned(),
            false,
            None,
            Box::new(move |window, cx| open(user.clone(), window, cx)),
        ));
    }
    if let Some(toggle) = actions.pins.clone() {
        let id = c.id.clone();
        let pinned = state.pins.get(&c.id).map_or(0, Vec::len);
        let label = if pinned > 0 {
            t!(&l, "header-pins-count", count = pinned).into_owned()
        } else {
            t!(&l, "header-pins").into_owned()
        };
        row = row.child(if pinned > 0 {
            text_button(
                "header-pins",
                pinned.to_string(),
                Icon::new(IconName::Pin)
                    .size(themes::metrics::ICON_SIZE_LG)
                    .color(colors.text_muted)
                    .into_any_element(),
                colors,
                move |window, cx| toggle(&id, window, cx),
            )
            .text_color(colors.text_muted)
            .tooltip(crate::reactions::tooltip_view(label))
        } else {
            button(
                "header-pins",
                IconName::Pin,
                label,
                false,
                None,
                Box::new(move |window, cx| toggle(&id, window, cx)),
            )
        });
    }
    if let Some(open) = actions.notify_menu.clone() {
        let level = state.prefs_for(&c.id).map(|p| p.level).unwrap_or_default();
        let quiet = matches!(level, domain::NotifyLevel::Nothing);
        let id = c.id.clone();
        let label = match crate::conversation_menu::level_mark(level) {
            Some(_) => format!(
                "{}: {}",
                t!(&l, "header-notify").into_owned(),
                crate::conversation_menu::level_label(level, &l)
            ),
            None => t!(&l, "header-notify").into_owned(),
        };
        let anchor: Anchor = Anchor::default();
        let at = anchor.clone();
        row = row.child(track_bounds(
            button(
                "header-notify",
                if quiet {
                    IconName::BellOff
                } else {
                    IconName::Bell
                },
                label,
                false,
                None,
                Box::new(move |window, cx| open(&id, menu_anchor(at.get()), window, cx)),
            ),
            anchor,
        ));
    }
    if let Some(toggle) = actions.details.clone() {
        let id = c.id.clone();
        row = row.child(button(
            "header-details",
            IconName::Info,
            t!(&l, "header-details").into_owned(),
            actions.details_open,
            None,
            Box::new(move |window, cx| toggle(&id, window, cx)),
        ));
    }
    if let Some(open) = actions.more_menu.clone() {
        let id = c.id.clone();
        let anchor: Anchor = Anchor::default();
        let at = anchor.clone();
        row = row.child(track_bounds(
            button(
                "header-more",
                IconName::MoreV,
                t!(&l, "header-more").into_owned(),
                false,
                None,
                Box::new(move |window, cx| open(&id, menu_anchor(at.get()), window, cx)),
            ),
            anchor,
        ));
    }
    let tab = |which: ConversationTab, id: &'static str, icon: IconName, label: String| {
        let on = actions.tab == which;
        let text = if on { colors.text } else { colors.text_muted };
        let el = div()
            .id(id)
            .debug_selector(move || id.to_string())
            .flex()
            .items_center()
            .gap(themes::metrics::SPACE_3)
            .h_full()
            .px(themes::metrics::SPACE_1)
            .border_b_2()
            .border_color(if on {
                tokens.signal
            } else {
                gpui::transparent_black()
            })
            .text_size(themes::metrics::FONT_SIZE_MD)
            .font_weight(if on {
                themes::metrics::FONT_WEIGHT_SEMIBOLD
            } else {
                themes::metrics::FONT_WEIGHT_MEDIUM
            })
            .text_color(text)
            .child(
                Icon::new(icon)
                    .size(themes::metrics::ICON_SIZE_BASE)
                    .color(text),
            )
            .child(label);
        match actions.switch_tab.clone() {
            Some(switch) => crate::controls::on_activate_app(
                el.cursor_pointer()
                    .tab_stop(true)
                    .focusable()
                    .focus_visible(crate::controls::focus_style(colors)),
                move |window, cx| switch(which, window, cx),
            ),
            None => el,
        }
    };
    let mut tabs = div()
        .debug_selector(|| "header-tabs".into())
        .flex()
        .items_center()
        // The board: 4 px between tabs plus a 16 px margin after each.
        .gap(themes::metrics::SPACE_7)
        .flex_none()
        .h(themes::metrics::LAYOUT_TAB_HEIGHT)
        .px(themes::metrics::SPACE_7)
        .bg(colors.surface_background)
        .border_b_1()
        .border_color(tokens.border_subtle)
        .child(tab(
            ConversationTab::Messages,
            "header-tab-messages",
            IconName::Message,
            t!(&l, "header-tab-messages").into_owned(),
        ));
    if actions.switch_tab.is_some() {
        tabs = tabs.child(tab(
            ConversationTab::Files,
            "header-tab-files",
            IconName::Layers,
            t!(&l, "header-tab-files").into_owned(),
        ));
    }
    div()
        .flex()
        .flex_col()
        .flex_none()
        .w_full()
        .child(row)
        .child(tabs)
        .into_any_element()
}

/// The names the header's avatar stack shows (at most [`STACK_MAX`]) and whether
/// they are recent authors rather than the member list.
fn people_for_stack(
    state: &WorkspaceState,
    c: &Conversation,
) -> (Vec<(String, Option<domain::ImageRef>)>, bool) {
    let (people, recent) = crate::channel_details::recent_people(state, c, STACK_MAX);
    let names = people
        .iter()
        .filter_map(|u| state.members.get(u))
        .filter_map(|m| {
            crate::sidebar::best_name(m).map(|n| {
                (
                    n.to_string(),
                    m.avatar_for(themes::metrics::LAYOUT_AVATAR_SM.as_f32())
                        .cloned(),
                )
            })
        })
        .collect();
    (names, recent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{ConversationId, ProviderExt};
    use settings::WorkspaceKey;
    use std::sync::Arc;

    fn conv(kind: ConversationKind, name: &str) -> Conversation {
        Conversation {
            id: ConversationId(Arc::from("C1")),
            kind,
            name: Arc::from(name),
            peer: None,
            is_member: true,
            is_archived: false,
            is_open: true,
            is_muted: false,
            last_read: None,
            latest: None,
            unread: false,
            mentions: 0,
            participants: Arc::from([]),
            member_ids: Arc::from([]),
            topic: Default::default(),
            member_count: None,
            purpose: Default::default(),
            ext: ProviderExt::none(),
        }
    }

    fn en() -> i18n::Localizer {
        let pref = i18n::LocalePref::Code("en".parse().unwrap());
        i18n::Localizer::new(
            i18n::chain(&pref, &[], i18n::LocaleSet::embedded()),
            &[crate::catalog()],
        )
        .unwrap()
    }

    fn state() -> WorkspaceState {
        WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 10)
    }

    #[test]
    fn direct_header_shows_the_peers_presence() {
        let mut d = conv(ConversationKind::Direct, "ana");
        d.peer = Some(domain::UserId(Arc::from("U1")));
        let mut st = state();
        assert_eq!(header_info(&st, &d, &en()).presence, None);
        st.presence.insert(domain::UserId(Arc::from("U1")), true);
        assert_eq!(header_info(&st, &d, &en()).presence, Some(true));
        let c = conv(ConversationKind::Channel, "general");
        assert_eq!(header_info(&st, &c, &en()).presence, None);
    }

    #[test]
    fn direct_header_line_has_presence_local_time_and_status() {
        let peer = domain::UserId(Arc::from("U1"));
        let mut d = conv(ConversationKind::Direct, "aiko");
        d.peer = Some(peer.clone());
        let mut st = state();
        // 2026-11-15 09:42 UTC is 18:42 in Tokyo.
        let now = jiff::Timestamp::from_second(1_794_735_720).unwrap();
        let sub = |st: &WorkspaceState| dm_subtitle(st, &d, now, settings::Clock::H24, &en());
        assert_eq!(sub(&st), None);
        st.presence.insert(peer.clone(), false);
        let status = domain::Status {
            emoji: Some(domain::EmojiRef {
                shortcode: Arc::from("headphones"),
                unicode: Some(Arc::from("\u{1f3a7}")),
                skin_tone: None,
            }),
            text: Arc::from("Reviewing baselines"),
            expires: None,
        };
        st.members.insert(
            peer.clone(),
            domain::Member {
                id: peer.clone(),
                display_name: Arc::from("Aiko"),
                real_name: Arc::from("Aiko Tanaka"),
                handle: Arc::from("aiko"),
                is_bot: false,
                deleted: false,
                status: Some(status),
                title: None,
                tz: None,
                avatar: None,
                ext: domain::ProviderExt::none(),
            },
        );
        assert_eq!(
            sub(&st).as_deref(),
            Some("Away \u{b7} \u{1f3a7} Reviewing baselines")
        );
        st.profiles.insert(
            peer.clone(),
            domain::Profile {
                user: peer.clone(),
                display_name: Arc::from("Aiko"),
                real_name: Arc::from("Aiko Tanaka"),
                title: None,
                pronouns: None,
                tz: Some(Arc::from("Asia/Tokyo")),
                status: None,
                email: None,
                phone: None,
                fields: Vec::new(),
                image: None,
                is_bot: false,
                deleted: false,
            },
        );
        assert_eq!(
            sub(&st).as_deref(),
            Some("Away \u{b7} 18:42 local time (Tokyo) \u{b7} \u{1f3a7} Reviewing baselines")
        );
        // A channel has no such line.
        let c = conv(ConversationKind::Channel, "general");
        assert_eq!(dm_subtitle(&st, &c, now, settings::Clock::H24, &en()), None);
    }

    #[test]
    fn channel_header_has_hash_topic_and_count() {
        let mut c = conv(ConversationKind::Channel, "general");
        c.topic = Arc::from("  Company news ");
        c.member_count = Some(42);
        let i = header_info(&state(), &c, &en());
        assert_eq!(i.title, "# general");
        assert_eq!(i.topic, "Company news");
        assert_eq!(i.member_count, Some(42));
    }

    #[test]
    fn private_channel_has_lock_and_dm_has_no_count() {
        let c = conv(ConversationKind::PrivateChannel, "secret");
        assert_eq!(header_info(&state(), &c, &en()).title, "\u{1F512} secret");
        let mut d = conv(ConversationKind::Direct, "ana");
        d.member_count = Some(2);
        let i = header_info(&state(), &d, &en());
        assert_eq!((i.title.as_str(), i.member_count), ("ana", None));
    }

    #[test]
    fn group_count_is_exact_when_the_members_are_listed() {
        let mut g = conv(ConversationKind::Group, "x");
        g.participants = Arc::from([Arc::<str>::from("a"), Arc::from("b"), Arc::from("me")]);
        assert_eq!(header_info(&state(), &g, &en()).member_count, Some(4));
        g.member_ids = Arc::from([
            domain::UserId(Arc::from("U1")),
            domain::UserId(Arc::from("U2")),
            domain::UserId(Arc::from("U3")),
        ]);
        assert_eq!(header_info(&state(), &g, &en()).member_count, Some(3));
    }

    #[test]
    fn group_count_counts_self_once_when_the_participants_list_self() {
        let mut g = conv(ConversationKind::Group, "x");
        g.participants = Arc::from([Arc::<str>::from("a"), Arc::from("b"), Arc::from("me")]);
        let mut st = state();
        let uid = domain::UserId(Arc::from("U9"));
        st.members.insert(
            uid.clone(),
            domain::Member {
                id: uid.clone(),
                display_name: Arc::from("Me"),
                real_name: Arc::from("Me"),
                handle: Arc::from("me"),
                is_bot: false,
                deleted: false,
                status: None,
                title: None,
                tz: None,
                avatar: None,
                ext: domain::ProviderExt::none(),
            },
        );
        st.self_user = Some(uid);
        assert_eq!(header_info(&st, &g, &en()).member_count, Some(3));
        g.participants = Arc::from([Arc::<str>::from("a"), Arc::from("b")]);
        assert_eq!(header_info(&st, &g, &en()).member_count, Some(3));
    }
}
