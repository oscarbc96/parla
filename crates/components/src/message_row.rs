//! One message row of the timeline: pure view-model plus its element.

use domain::{DeliveryState, ImageRef, Member, Message, MessageTs};
use gpui::{AnyElement, Hsla, IntoElement, Pixels, SharedString, div, prelude::*, relative};
use i18n::{HourCycle, Localizer};
use jiff::{Timestamp, tz::TimeZone};
use settings::{Clock, Density};
use themes::{Theme, parla_tokens};

use crate::rich_text::RichColors;

/// Consecutive messages from one author within this many seconds share a header.
pub const GROUP_WINDOW_SECS: i64 = 5 * 60;

/// Whole seconds of a message timestamp.
pub fn secs(ts: &MessageTs) -> Option<i64> {
    ts.0.split('.').next()?.parse().ok()
}

/// Whether `cur` continues the group started above it (same author, <= 5 min
/// apart). An unconfirmed row (pending or failed) sits below confirmed rows
/// the server stamped later than its local ts, so a negative gap joins too.
pub fn continues(prev: &Message, cur: &Message) -> bool {
    match (&prev.author, &cur.author) {
        (Some(a), Some(b)) if a == b => {}
        _ => return false,
    }
    let local = cur.state != DeliveryState::Delivered;
    match (secs(&prev.id.ts), secs(&cur.id.ts)) {
        (Some(p), Some(c)) => c - p <= GROUP_WINDOW_SECS && (local || c >= p),
        _ => false,
    }
}

/// For each message, whether it is grouped under the previous one.
pub fn group_flags(msgs: &[Message]) -> Vec<bool> {
    (0..msgs.len())
        .map(|i| i > 0 && continues(&msgs[i - 1], &msgs[i]))
        .collect()
}

/// The time of day in `tz` for the active language and `clock`; empty when `ts` is not a timestamp.
pub fn format_time(ts: &MessageTs, clock: Clock, tz: &TimeZone, l: &Localizer) -> String {
    let Some(t) = secs(ts).and_then(|s| Timestamp::from_second(s).ok()) else {
        return String::new();
    };
    l.time(t, tz, hour_cycle(clock))
}

/// The i18n clock choice for the `messages.clock` setting.
pub fn hour_cycle(clock: Clock) -> HourCycle {
    match clock {
        Clock::H24 => HourCycle::H24,
        Clock::H12 => HourCycle::H12,
    }
}

/// Date and time for lists that span days.
pub fn format_date_time(ts: &MessageTs, clock: Clock, tz: &TimeZone, l: &Localizer) -> String {
    let Some(t) = secs(ts).and_then(|s| Timestamp::from_second(s).ok()) else {
        return String::new();
    };
    l.date_time(t, tz, hour_cycle(clock))
}

/// Longest preview text, in characters, before it is cut with an ellipsis.
pub const PREVIEW_CHARS: usize = 160;

/// One short, read-only line for a message: an "All unreads" preview or a
/// search hit in context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewLine {
    pub id: domain::MessageId,
    pub author: SharedString,
    /// The author's picture (initials while it loads and when there is none).
    pub avatar: Option<ImageRef>,
    pub time: SharedString,
    pub text: SharedString,
}

/// The plain text of `m` on one line, at most [`PREVIEW_CHARS`] characters.
pub fn preview_text(
    parser: &dyn domain::ContentParser,
    m: &Message,
    groups: &domain::UserGroups,
) -> String {
    let plain = parser.parse_text(&m.text).plain_text();
    // A label-less group token reads as its id; show the handle.
    let plain = domain::resolve_group_text(&plain, &m.mentions, groups);
    let flat = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut text: String = flat.chars().take(PREVIEW_CHARS).collect();
    if flat.chars().count() > PREVIEW_CHARS {
        text.push('\u{2026}');
    }
    text
}

/// A [`PreviewLine`] for `m`: author from the directory, time in `tz`.
pub fn preview_line(
    m: &Message,
    author: Option<&Member>,
    parser: &dyn domain::ContentParser,
    clock: Clock,
    tz: &TimeZone,
    l: &Localizer,
    groups: &domain::UserGroups,
) -> PreviewLine {
    PreviewLine {
        id: m.id.clone(),
        author: author_name(author, l),
        avatar: author.and_then(|m| {
            m.avatar_for(themes::metrics::LAYOUT_MSG_AVATAR.as_f32())
                .cloned()
        }),
        time: format_time(&m.id.ts, clock, tz, l).into(),
        text: preview_text(parser, m, groups).into(),
    }
}

/// Best available display name for an author. An id the directory has not
/// resolved yet is never shown raw: it reads as the neutral placeholder.
pub fn author_name(member: Option<&Member>, l: &Localizer) -> SharedString {
    // Shares the member's name instead of copying it: rows render every frame.
    match member.and_then(crate::sidebar::best_name) {
        Some(n) => n.into(),
        None => t!(l, "author-unknown").into_owned().into(),
    }
}

/// Display name of a user (reply and reaction lists), or the neutral
/// placeholder while the member is unresolved. Uses [`best_name`].
pub fn user_name(member: Option<&Member>, l: &Localizer) -> String {
    member
        .and_then(crate::sidebar::best_name)
        .map(|n| n.to_string())
        .unwrap_or_else(|| t!(l, "author-unknown").into_owned())
}

/// What a click on the author's name does.
pub type AuthorClick = std::rc::Rc<dyn Fn(&mut gpui::Window, &mut gpui::App)>;

/// What a failed-send action does.
pub type FailedHandler = std::rc::Rc<dyn Fn(&mut gpui::Window, &mut gpui::App)>;

/// The actions under a message that could not be sent: Retry re-queues it,
/// Delete discards the outbox item, Copy text puts the text on the clipboard.
#[derive(Clone)]
pub struct FailedActions {
    /// Unique among the rows on screen (the message's timestamp).
    pub key: SharedString,
    pub retry: FailedHandler,
    pub delete: FailedHandler,
    pub copy: FailedHandler,
}

/// The Retry, Delete and Copy text actions of the failed message `m`: Retry
/// and Delete go through `retry` and `discard` with its client id, Copy puts
/// its text on the clipboard. `None` when `m` has not failed or has no client
/// id (it was never an outbox item).
pub fn failed_actions_for(
    m: &Message,
    retry: impl Fn(domain::ClientMsgId, &mut gpui::App) + 'static,
    discard: impl Fn(domain::ClientMsgId, &mut gpui::App) + 'static,
) -> Option<FailedActions> {
    if !matches!(m.state, DeliveryState::Failed(_)) {
        return None;
    }
    let id = m.client_msg_id.clone()?;
    let text = m.text.to_string();
    let retry_id = id.clone();
    Some(FailedActions {
        key: SharedString::from(m.id.ts.0.clone()),
        retry: std::rc::Rc::new(move |_, cx| retry(retry_id.clone(), cx)),
        delete: std::rc::Rc::new(move |_, cx| discard(id.clone(), cx)),
        copy: std::rc::Rc::new(move |_, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()))
        }),
    })
}

/// Makes the author's name a target: a click runs `on_click`.
#[derive(Clone)]
pub struct AuthorLink {
    /// Unique among the rows on screen (the message's timestamp).
    pub key: SharedString,
    pub on_click: AuthorClick,
}

pub struct MessageRow {
    pub author: SharedString,
    /// The author's status emoji (a glyph), shown after the name.
    pub status: Option<SharedString>,
    pub author_link: Option<AuthorLink>,
    pub time: SharedString,
    pub text: SharedString,
    /// Rendered rich body; replaces `text` when set.
    pub rich: Option<AnyElement>,
    pub edited: bool,
    pub state: DeliveryState,
    /// A pending message that waits for the connection: shows "Queued".
    pub queued: bool,
    /// Actions of a failed message; without them the row only says "Not sent".
    pub failed_actions: Option<FailedActions>,
    pub grouped: bool,
    pub selected: bool,
    pub density: Density,
    /// `messages.show_avatars`; off keeps the indent and leaves the avatar out.
    pub show_avatar: bool,
    /// The author's picture; initials until it loads and when it has none.
    pub avatar: Option<ImageRef>,
}

/// Left inset of everything under the author line: row padding, avatar, gap.
pub fn body_indent() -> Pixels {
    themes::metrics::SPACE_7 + themes::metrics::LAYOUT_MSG_AVATAR + themes::metrics::SPACE_5
}

/// The avatar: the picture once loaded, else initials on a colour derived from
/// the name, in a rounded square.
pub fn avatar(name: &str, theme: &Theme, image: Option<&ImageRef>) -> AnyElement {
    avatar_box(name, theme, themes::metrics::LAYOUT_MSG_AVATAR, image)
        .debug_selector(|| "message-avatar".into())
        .into_any_element()
}

/// [`avatar`] at `size` (a `LAYOUT_*` avatar size); small ones use the XS type.
pub fn avatar_sized(
    name: &str,
    theme: &Theme,
    size: Pixels,
    image: Option<&ImageRef>,
) -> AnyElement {
    avatar_box(name, theme, size, image).into_any_element()
}

/// [`avatar_sized`] with a presence dot on its bottom right corner, ringed in
/// `ring` (the ground it sits on). No dot when `presence` is unknown.
pub fn avatar_with_presence(
    name: &str,
    initials: SharedString,
    theme: &Theme,
    size: Pixels,
    presence: Option<bool>,
    (dot_id, ring): (&'static str, gpui::Hsla),
    image: Option<&ImageRef>,
) -> gpui::Div {
    let tokens = parla_tokens(theme);
    avatar_box_with(
        name,
        initials,
        theme,
        size,
        image,
        1.0,
        theme.colors().editor_background,
    )
    .relative()
    .when_some(presence, |d, active| {
        d.child(
            // One element: the ring is a shadow spread around the dot.
            crate::sidebar::presence_dot(
                dot_id,
                active,
                tokens.presence_active,
                tokens.presence_away,
            )
            .absolute()
            .right(gpui::px(0.0))
            .bottom(gpui::px(0.0))
            .shadow(vec![gpui::BoxShadow {
                color: ring,
                offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
                blur_radius: gpui::px(0.0),
                spread_radius: themes::metrics::SPACE_1,
                inset: false,
            }]),
        )
    })
}

fn avatar_box(name: &str, theme: &Theme, size: Pixels, image: Option<&ImageRef>) -> gpui::Div {
    avatar_box_with(
        name,
        crate::badge::initials(name).into(),
        theme,
        size,
        image,
        1.0,
        theme.colors().editor_background,
    )
}

/// [`avatar`] dimmed to `strength`: the fill, the picture, the edge and the
/// initials are composited over `ground` at that strength. Main faded the
/// whole avatar as one image, which left its initials white on the pale fill
/// in light and grey in dark; element opacity would fade them over the
/// already-faded fill instead.
fn avatar_dimmed(
    name: &str,
    theme: &Theme,
    image: Option<&ImageRef>,
    strength: f32,
    ground: gpui::Hsla,
) -> AnyElement {
    avatar_box_with(
        name,
        crate::badge::initials(name).into(),
        theme,
        themes::metrics::LAYOUT_MSG_AVATAR,
        image,
        strength,
        ground,
    )
    .debug_selector(|| "message-avatar".into())
    .into_any_element()
}

/// The corner radius of an avatar of side `size`.
fn avatar_radius(size: Pixels) -> Pixels {
    if size >= themes::metrics::LAYOUT_AVATAR_LG {
        themes::metrics::RADIUS_XL
    } else if size >= themes::metrics::LAYOUT_AVATAR_CARD {
        themes::metrics::RADIUS_LG
    } else {
        themes::metrics::RADIUS_AVATAR
    }
}

/// [`avatar_box`] with its letters given (a caller that keeps them). The box
/// has the avatar's size; its face is the picture or the initials.
fn avatar_box_with(
    name: &str,
    initials: SharedString,
    theme: &Theme,
    size: Pixels,
    image: Option<&ImageRef>,
    strength: f32,
    ground: gpui::Hsla,
) -> gpui::Div {
    div().flex_none().size(size).child(AvatarFace {
        name: SharedString::from(name.to_string()),
        initials,
        size,
        image: image.cloned(),
        fill: themes::avatar_fill(crate::badge::hue(name), theme),
        text: parla_tokens(theme).avatar_text,
        edge: parla_tokens(theme).border_subtle,
        strength,
        ground,
    })
}

/// What an avatar shows. Initials on the name's colour always draw first and
/// stay while the picture loads, when it fails and when there is none; the
/// picture comes from the shared [`crate::media::ImageCache`] (decoded at the
/// display size, fetched once, only when the avatar is rendered).
#[derive(IntoElement)]
struct AvatarFace {
    /// The person's name, the picture's accessible name.
    name: SharedString,
    initials: SharedString,
    size: Pixels,
    image: Option<ImageRef>,
    fill: gpui::Hsla,
    text: gpui::Hsla,
    /// The 1 px edge drawn over a picture, so a light photo keeps its shape.
    edge: gpui::Hsla,
    /// Strength of the fill, the picture and the edge; the initials are
    /// composited over `ground` at the same strength, as main's group opacity
    /// drew them (white on the pale fill in light, grey in dark).
    strength: f32,
    /// What the avatar is drawn over (the row's ground).
    ground: gpui::Hsla,
}

impl RenderOnce for AvatarFace {
    fn render(self, window: &mut gpui::Window, cx: &mut gpui::App) -> impl IntoElement {
        let radius = avatar_radius(self.size);
        let fade = |colour: gpui::Hsla| {
            if self.strength < 1.0 {
                themes::color::faded(colour, self.ground, self.strength)
            } else {
                colour
            }
        };
        let loaded = match (&self.image, cx.has_global::<crate::media::ImageCache>()) {
            (Some(src), true) => {
                let scale = window.scale_factor();
                let boxed = gpui::size(self.size, self.size);
                match cx.update_global::<crate::media::ImageCache, _>(|c, cx| {
                    c.get(src, boxed, scale, cx)
                }) {
                    crate::media::ImageState::Ready(image) => Some(image),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(image) = loaded {
            return div()
                .id("avatar")
                .role(gpui::Role::Image)
                .aria_label(self.name)
                .relative()
                .size_full()
                .rounded(radius)
                .overflow_hidden()
                .child(
                    // overflow_hidden clips to a rectangle: round the picture
                    // itself so its corners stay inside the rounded edge.
                    gpui::img(image)
                        .debug_selector(|| "avatar-image".into())
                        .rounded(radius)
                        .object_fit(gpui::ObjectFit::Cover)
                        .size_full()
                        .when(self.strength < 1.0, |img| img.opacity(self.strength)),
                )
                .child(
                    div()
                        .absolute()
                        .top(themes::metrics::SPACE_0)
                        .left(themes::metrics::SPACE_0)
                        .size_full()
                        .rounded(radius)
                        .border_1()
                        .border_color(fade(self.edge)),
                )
                .into_any_element();
        }
        let font = if self.size < themes::metrics::LAYOUT_MSG_AVATAR {
            themes::metrics::FONT_SIZE_XS
        } else if self.size >= themes::metrics::LAYOUT_AVATAR_LG {
            themes::metrics::FONT_SIZE_3XL
        } else if self.size >= themes::metrics::LAYOUT_AVATAR_CARD {
            themes::metrics::FONT_SIZE_2XL
        } else {
            themes::metrics::FONT_SIZE_SM
        };
        div()
            .id("avatar")
            .role(gpui::Role::Image)
            .aria_label(self.name)
            .debug_selector(|| "avatar-initials".into())
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .rounded(radius)
            .bg(fade(self.fill))
            .text_color(fade(self.text))
            .text_size(font)
            .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
            .child(self.initials)
            .into_any_element()
    }
}

/// Strength of a row's colours: only a plain pending row is dimmed. A queued
/// row stays at full strength so its "Queued" label keeps AA contrast.
pub fn row_opacity(state: &DeliveryState, queued: bool) -> f32 {
    if *state == DeliveryState::Pending && !queued {
        0.5
    } else {
        1.0
    }
}

/// The rich text colours for a row: a pending row's own colours (links,
/// mentions, code chips, card grounds) composited over its `ground`, since the
/// inherited text colour cannot dim what the rich text paints itself. Queued
/// and delivered rows keep them at full strength.
pub fn rich_colors(
    colors: RichColors,
    state: &DeliveryState,
    queued: bool,
    ground: Hsla,
) -> RichColors {
    let strength = row_opacity(state, queued);
    if strength < 1.0 {
        colors.faded(strength, ground)
    } else {
        colors
    }
}

/// Colour of the "Queued" label and its clock: the theme's warning, which
/// meets 4.5:1 on the app, surface and editor grounds in both built-in themes.
pub fn queued_color(theme: &Theme) -> gpui::Hsla {
    theme.status().warning
}

fn queued_label(theme: &Theme, l: &Localizer) -> impl IntoElement {
    let color = queued_color(theme);
    div()
        .debug_selector(|| "message-queued".into())
        .flex()
        .items_center()
        .gap(themes::metrics::SPACE_2)
        .text_size(themes::metrics::FONT_SIZE_XS)
        .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(color)
        .child(
            div().debug_selector(|| "message-queued-icon".into()).child(
                crate::icon::Icon::new(crate::icon::IconName::Clock)
                    .size(themes::metrics::ICON_SIZE_SM)
                    .color(color),
            ),
        )
        .child(t!(l, "message-queued").into_owned())
}

/// The author's status glyph beside the name. A monochrome status mark (✔ ☎
/// ♥) takes the row's full-strength text colour, so the element opacity dims
/// each glyph once: the inherited colour is already faded for a pending row.
/// Colour emoji ignore the text colour and dim through the opacity alone.
fn status_glyph(glyph: SharedString, alpha: f32, colors: &themes::ThemeColors) -> gpui::Div {
    div()
        .debug_selector(|| "message-row-status".into())
        .text_sm()
        .text_color(colors.text)
        .when(alpha < 1.0, |d| d.opacity(alpha))
        .child(glyph)
}

impl MessageRow {
    pub fn render(mut self, theme: &Theme, l: &Localizer) -> AnyElement {
        let colors = theme.colors();
        let tokens = parla_tokens(theme);
        let dimmed = self.state == DeliveryState::Pending;
        let queued = dimmed && self.queued;
        // Main faded a pending row as one image. GPUI Fast multiplies opacity
        // into every paint, so a dimmed row went pale on pale where a child
        // draws over another (the avatar's initials over its fill); dim the
        // colours instead, which paints each leaf once. The avatar composites
        // its fill and initials over the row's ground (see `avatar_dimmed`).
        let alpha = row_opacity(&self.state, self.queued);
        let mut body = div().flex().flex_col().min_w_0();
        if !self.grouped {
            body = body.child(
                div()
                    .flex()
                    .gap(themes::metrics::SPACE_4)
                    .items_baseline()
                    .child(match self.author_link.take() {
                        Some(link) => {
                            let key = link.key.clone();
                            let on = link.on_click;
                            div()
                                // The timestamp key alone is unique among a row's siblings
                                // and costs no allocation per frame.
                                .id(key.clone())
                                .debug_selector(move || format!("message-author-{key}"))
                                .cursor_pointer()
                                .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                                .child(self.author.clone())
                                .on_click(move |_, w, cx| on(w, cx))
                                .into_any_element()
                        }
                        None => div()
                            .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                            .child(self.author.clone())
                            .into_any_element(),
                    })
                    .children(self.status.take().map(|e| status_glyph(e, alpha, colors)))
                    .child(if queued {
                        queued_label(theme, l).into_any_element()
                    } else {
                        div()
                            .text_size(themes::metrics::FONT_SIZE_XS)
                            .text_color(tokens.text_subtle.opacity(alpha))
                            .child(self.time.clone())
                            .into_any_element()
                    }),
            );
        }
        let body_el = match self.rich.take() {
            Some(el) => el,
            None => div().child(self.text.clone()).into_any_element(),
        };
        // A grouped row has no header, so its "Queued" label sits above the text.
        if queued && self.grouped {
            body = body.child(queued_label(theme, l));
        }
        // Only an edited row needs the text row with its "(edited)" mark; any
        // other row puts the body straight in the column, which stretches it to
        // the same width (two fewer elements per row, allocation budget).
        if self.edited {
            body = body.child(
                div()
                    .flex()
                    .gap_1()
                    .child(div().flex_1().min_w_0().child(body_el))
                    .child(
                        div()
                            .text_size(themes::metrics::FONT_SIZE_XS)
                            .text_color(tokens.text_subtle.opacity(alpha))
                            .child(t!(l, "message-edited").into_owned()),
                    ),
            );
        } else {
            body = body.child(body_el);
        }
        if let DeliveryState::Failed(_) = &self.state {
            body = body.child(failed_strip(
                &self.state,
                self.failed_actions.take(),
                theme,
                l,
            ));
        }
        let pad = if self.grouped {
            crate::density::message_gap(self.density)
        } else {
            crate::density::message_pad(self.density)
        };
        // Block layout, avatar placed absolutely: a flex row costs more layout
        // work per row (allocation budget).
        div()
            .w_full()
            .relative()
            .pl(body_indent())
            .pr(themes::metrics::SPACE_7)
            .py(pad)
            .text_size(themes::metrics::FONT_SIZE_MD)
            .line_height(relative(1.55))
            .text_color(colors.text.opacity(alpha))
            // The subtle selected surface, not the saturated sidebar accent:
            // the text and metadata keep their contrast in light and dark.
            .when(self.selected, |d| {
                d.bg(colors.element_selected.opacity(alpha))
            })
            .when(!self.grouped && self.show_avatar, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(themes::metrics::SPACE_7)
                        .top(pad)
                        .child(if alpha < 1.0 {
                            avatar_dimmed(
                                &self.author,
                                theme,
                                self.avatar.as_ref(),
                                alpha,
                                colors.editor_background,
                            )
                        } else {
                            avatar(&self.author, theme, self.avatar.as_ref())
                        }),
                )
            })
            .child(body)
            .into_any_element()
    }
}

/// One clickable action of the failed strip: a tab stop with the focus ring,
/// run by a click or Enter or Space.
fn failed_action(
    name: &str,
    key: &SharedString,
    label: String,
    handler: FailedHandler,
    theme: &Theme,
) -> AnyElement {
    let colors = theme.colors();
    let selector = format!("message-failed-{name}-{key}");
    let el = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .px(themes::metrics::SPACE_1)
        .rounded(themes::metrics::RADIUS_SM)
        .border_1()
        .border_color(gpui::transparent_black())
        .text_size(themes::metrics::FONT_SIZE_XS)
        .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(colors.text_accent)
        .cursor_pointer()
        .tab_stop(true)
        .focusable()
        .focus_visible(crate::controls::focus_style(colors))
        .child(label);
    crate::controls::on_activate_app(el, move |w, cx| {
        cx.stop_propagation();
        handler(w, cx)
    })
    .into_any_element()
}

/// The strip under a message that could not be sent: "Not sent." and, when the
/// host wires them, Retry, Delete and Copy text.
fn failed_strip(
    state: &DeliveryState,
    actions: Option<FailedActions>,
    theme: &Theme,
    l: &Localizer,
) -> AnyElement {
    let tokens = parla_tokens(theme);
    let mut strip = div()
        .flex()
        .flex_wrap()
        .items_center()
        .self_start()
        .gap(themes::metrics::SPACE_4)
        .mt(themes::metrics::SPACE_3)
        .px(themes::metrics::SPACE_5)
        .py(themes::metrics::SPACE_3)
        .rounded(themes::metrics::RADIUS_MD)
        .bg(tokens.danger_soft)
        .child(
            div()
                .text_size(themes::metrics::FONT_SIZE_XS)
                .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                .text_color(theme.status().error)
                .child(t!(l, "message-not-sent").into_owned()),
        );
    // A send cut short by quitting may have gone through: warn before a retry.
    if matches!(state, DeliveryState::Failed(r) if r == domain::RESTART_REASON) {
        strip = strip.child(
            div()
                .debug_selector(|| "message-failed-note".to_string())
                .text_size(themes::metrics::FONT_SIZE_XS)
                .text_color(tokens.text_subtle)
                .child(t!(l, "message-failed-maybe-sent").into_owned()),
        );
    }
    if let Some(a) = actions {
        let dot = || {
            div()
                .text_size(themes::metrics::FONT_SIZE_XS)
                .text_color(tokens.text_subtle)
                .child("\u{b7}")
        };
        strip = strip
            .child(failed_action(
                "retry",
                &a.key,
                t!(l, "message-retry").into_owned(),
                a.retry,
                theme,
            ))
            .child(dot())
            .child(failed_action(
                "delete",
                &a.key,
                t!(l, "message-delete").into_owned(),
                a.delete,
                theme,
            ))
            .child(dot())
            .child(failed_action(
                "copy",
                &a.key,
                t!(l, "message-copy-text").into_owned(),
                a.copy,
                theme,
            ));
    }
    strip.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    /// The row paints one element opacity over a status glyph; the glyph's
    /// colour must be the full-strength text colour, not the row's inherited
    /// (already faded) one, or monochrome glyphs fade twice.
    #[gpui::test]
    fn a_pending_rows_status_glyph_is_dimmed_once(cx: &mut TestAppContext) {
        cx.update(|cx| {
            themes::init_default(cx);
            let registry = themes::ThemeRegistry::global(cx);
            for name in [themes::DEFAULT_LIGHT, themes::DEFAULT_DARK] {
                let theme = registry.get(name).unwrap();
                let colors = theme.colors();
                let mut glyph = status_glyph("ok".into(), 0.5, colors);
                assert_eq!(glyph.style().opacity, Some(0.5), "{name}: one multiply");
                assert_eq!(
                    glyph.style().text.color,
                    Some(colors.text),
                    "{name}: full-strength glyph colour"
                );
                assert_ne!(
                    glyph.style().text.color,
                    Some(colors.text.opacity(0.5)),
                    "{name}: not the row's faded colour"
                );
            }
        });
    }
}
