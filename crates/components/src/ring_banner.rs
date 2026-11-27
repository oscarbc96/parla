//! The incoming-huddle banner: who is ringing, with Join and Decline. Pure
//! view; the shell owns the ring and decides when it is shown.

use gpui::{App, ClickEvent, IntoElement, SharedString, Window, div, prelude::*};
use themes::{ActiveTheme, metrics, parla_tokens};

/// What the banner shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingBannerData {
    /// Who is ringing.
    pub caller: SharedString,
}

/// Renders the banner. `on_join` and `on_decline` run on the buttons.
pub fn ring_banner<J, D>(
    data: &RingBannerData,
    on_join: J,
    on_decline: D,
    cx: &App,
) -> impl IntoElement + use<J, D>
where
    J: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    D: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let theme = cx.theme().clone();
    let colors = theme.colors();
    let tokens = parla_tokens(&theme);
    let l = i18n::Translate::localizer(cx);
    let initials = crate::badge::initials(&data.caller);
    // Raised card in a full-width strip, like the board's incoming-huddle ring.
    let card = div()
        .debug_selector(|| "ring-banner".to_string())
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .w_full()
        .max_w(metrics::LAYOUT_DIALOG_MD)
        .px(metrics::SPACE_5)
        .py(metrics::SPACE_4)
        .text_size(metrics::FONT_SIZE_MD)
        .text_color(colors.text)
        .bg(colors.elevated_surface_background)
        .border_1()
        .border_color(colors.border)
        .rounded(metrics::RADIUS_LG)
        .child(
            // Ringing mark: the caller's initials in a rounded avatar with a
            // presence-coloured outline (shape plus colour).
            div()
                .debug_selector(|| "ring-avatar".to_string())
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(metrics::SPACE_11)
                .rounded(metrics::RADIUS_AVATAR)
                .border_2()
                .border_color(tokens.presence_active)
                .bg(colors.surface_background)
                .text_size(metrics::FONT_SIZE_MD)
                .font_weight(metrics::FONT_WEIGHT_BOLD)
                .child(initials),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_weight(metrics::FONT_WEIGHT_BOLD)
                .text_size(metrics::FONT_SIZE_LG)
                .child(t!(l, "ring-banner-text", caller = data.caller.to_string()).into_owned()),
        )
        .child(
            div()
                .id("ring-decline")
                .debug_selector(|| "ring-decline".to_string())
                .flex()
                .flex_none()
                .items_center()
                .h(metrics::SPACE_10)
                .px(metrics::SPACE_6)
                .rounded(metrics::RADIUS_FULL)
                .bg(theme.status().error)
                .text_color(tokens.danger_text)
                .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                .cursor_pointer()
                .hover(|s| s.opacity(0.9))
                .child(t!(l, "ring-banner-decline").into_owned())
                .on_click(on_decline),
        )
        .child(
            div()
                .id("ring-join")
                .debug_selector(|| "ring-join".to_string())
                .flex()
                .flex_none()
                .items_center()
                .h(metrics::SPACE_10)
                .px(metrics::SPACE_6)
                .rounded(metrics::RADIUS_FULL)
                .bg(theme.status().success)
                .text_color(colors.text_on_accent)
                .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                .cursor_pointer()
                .hover(|s| s.opacity(0.9))
                .child(t!(l, "ring-banner-join").into_owned())
                .on_click(on_join),
        );
    div()
        .flex()
        .flex_none()
        .justify_center()
        .w_full()
        .px(metrics::SPACE_5)
        .py(metrics::SPACE_4)
        .border_b_1()
        .border_color(tokens.border_subtle)
        .child(card)
}
