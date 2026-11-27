//! Reaction pills: pure view-model helpers shared by the timeline and thread panel.

use domain::{EmojiRef, MessageId, Reaction};
use gpui::{
    AnyView, App, Context, Div, IntoElement, Render, SharedString, Window, div, prelude::*,
};
use i18n::Localizer;
use themes::ActiveTheme;
use themes::parla_tokens;

/// A tooltip lists at most this many names; the rest collapse into "and N others".
pub const MAX_TOOLTIP_NAMES: usize = 10;

/// What one pill shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Pill {
    pub emoji: EmojiRef,
    pub label: String,
    pub me: bool,
    pub tooltip: String,
    /// Image of a custom emoji, when known.
    pub image: Option<domain::ImageRef>,
    pub count: u32,
}

/// Glyph (or `:shortcode:` for custom emoji) of a reaction.
pub fn glyph(e: &EmojiRef) -> String {
    match &e.unicode {
        Some(u) => u.to_string(),
        None => format!(":{}:", e.shortcode),
    }
}

/// Image of `r` when it is a custom emoji present in `custom`.
pub fn pill_image(custom: &[domain::CustomEmoji], r: &Reaction) -> Option<domain::ImageRef> {
    if r.emoji.unicode.is_some() {
        return None;
    }
    custom
        .iter()
        .find(|c| c.shortcode == r.emoji.shortcode)
        .map(|c| c.image.clone())
}

/// Pills of `m` in provider order. `member` resolves reaction users for the
/// tooltip, `image` looks up the image of a custom emoji by shortcode.
pub(crate) fn pills_of<'a>(
    m: &domain::Message,
    member: impl Fn(&domain::UserId) -> Option<&'a domain::Member>,
    image: impl Fn(&str) -> Option<domain::ImageRef>,
    l: &Localizer,
) -> Vec<Pill> {
    m.reactions
        .iter()
        .map(|r| {
            let names: Vec<String> = r
                .users
                .iter()
                .map(|u| crate::message_row::user_name(member(u), l))
                .collect();
            Pill {
                emoji: r.emoji.clone(),
                label: pill_label(r),
                me: r.me,
                tooltip: tooltip_text(&names, l),
                image: if r.emoji.unicode.is_some() {
                    None
                } else {
                    image(&r.emoji.shortcode)
                },
                count: r.count,
            }
        })
        .collect()
}

/// `"<glyph> <count>"`.
pub fn pill_label(r: &Reaction) -> String {
    format!("{} {}", glyph(&r.emoji), r.count)
}

/// Up to [`MAX_TOOLTIP_NAMES`] names, then "and N others".
pub fn tooltip_text(names: &[String], l: &Localizer) -> String {
    if names.len() <= MAX_TOOLTIP_NAMES {
        return names.join(", ");
    }
    let rest = names.len() - MAX_TOOLTIP_NAMES;
    let shown = names[..MAX_TOOLTIP_NAMES].join(", ");
    t!(l, "reactions-and-others", names = shown, count = rest).into_owned()
}

pub(crate) struct TextTooltip(pub SharedString);

impl Render for TextTooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .text_xs()
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .child(self.0.clone())
    }
}

/// [`tooltip_view`] whose text is built only when the tooltip opens: for
/// chrome drawn every frame (the rail), where building the text costs
/// allocations on each redraw.
pub fn tooltip_lazy(
    text: impl Fn(&App) -> String + 'static,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| {
        let t = text(cx);
        gpui::AppContext::new(cx, |_| TextTooltip(t.into())).into()
    }
}

/// [`tooltip_view`] for a label already held as a [`SharedString`]: building
/// it copies nothing.
pub fn tooltip_shared(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| TextTooltip(text.clone())).into()
}

pub fn tooltip_view(text: String) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| TextTooltip(text.clone().into())).into()
}

/// Colours of a pill (background, edge, text): highlighted, with the soft
/// accent text, when the current user reacted; otherwise the inherited text.
pub(crate) fn pill_colors(cx: &App, me: bool) -> (gpui::Hsla, gpui::Hsla, Option<gpui::Hsla>) {
    let theme = cx.theme();
    let colors = theme.colors();
    if me {
        (
            colors.accent_soft,
            colors.text_accent,
            Some(parla_tokens(theme).accent_soft_text),
        )
    } else {
        (colors.background, colors.border, None)
    }
}

type Toggle<V> = fn(&mut V, MessageId, EmojiRef, &mut Context<V>);
type Add<V> = fn(&mut V, MessageId, &mut Window, &mut Context<V>);

/// A wrapped row of pills (plus an optional "+" button) for message `id`.
/// `key` makes element ids unique per row.
pub(crate) fn pills_row<V: 'static>(
    key: usize,
    id: &MessageId,
    pills: Vec<Pill>,
    cx: &mut Context<V>,
    toggle: Toggle<V>,
    add: Option<Add<V>>,
) -> Div {
    let border = cx.theme().colors().border;
    let mut row = div()
        .flex()
        .flex_wrap()
        .gap_1()
        .pl(crate::message_row::body_indent())
        .pr(themes::metrics::SPACE_7)
        .pb_1();
    for (j, p) in pills.into_iter().enumerate() {
        let (bg, edge, text) = pill_colors(cx, p.me);
        let (mid, emoji) = (id.clone(), p.emoji.clone());
        row = row.child(
            div()
                .id(("reaction-pill", key * 64 + j))
                .debug_selector(|| format!("reaction-{}", p.emoji.shortcode))
                .h(themes::metrics::SPACE_8)
                .px(themes::metrics::SPACE_4)
                .flex()
                .items_center()
                .cursor_pointer()
                .rounded_full()
                .border_1()
                .border_color(edge)
                .bg(bg)
                .when_some(text, |d, c| d.text_color(c))
                .text_size(themes::metrics::FONT_SIZE_XS)
                .child(
                    match (&p.image, cx.has_global::<crate::media::ImageCache>()) {
                        (Some(image), true) => div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(crate::media::emoji_hook(image, &p.emoji.shortcode))
                            .child(p.count.to_string())
                            .into_any_element(),
                        _ => div().child(p.label.clone()).into_any_element(),
                    },
                )
                .tooltip(tooltip_view(p.tooltip.clone()))
                .on_click(
                    cx.listener(move |this, _, _, cx| toggle(this, mid.clone(), emoji.clone(), cx)),
                ),
        );
    }
    if let Some(add) = add {
        let mid = id.clone();
        row = row.child(
            div()
                .id(("reaction-add", key))
                .h(themes::metrics::SPACE_8)
                .px(themes::metrics::SPACE_4)
                .flex()
                .items_center()
                .cursor_pointer()
                .rounded_full()
                .border_1()
                .border_color(border)
                .text_size(themes::metrics::FONT_SIZE_XS)
                .child("+")
                .on_click(
                    cx.listener(move |this, _, window, cx| add(this, mid.clone(), window, cx)),
                ),
        );
    }
    row
}
