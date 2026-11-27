//! The "you are offline" strip with a Retry now button. The realtime
//! connection retries by itself with a backoff, so the text is static: no
//! countdown, no timer.

use gpui::{App, IntoElement, Window, div, prelude::*};
use themes::{ActiveTheme as _, metrics, parla_tokens};

/// Renders the banner. `on_retry` runs when the button is clicked, or
/// activated with Enter or Space.
pub fn offline_banner(
    cx: &App,
    on_retry: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let colors = cx.theme().colors();
    let soft = parla_tokens(cx.theme()).warning_soft;
    div()
        .debug_selector(|| "offline-banner".into())
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .px(metrics::SPACE_6)
        .py(metrics::SPACE_5)
        .bg(soft)
        .text_color(colors.text)
        .text_size(metrics::FONT_SIZE_MD)
        .child(
            components::icon::Icon::new(components::icon::IconName::WifiOff)
                .size(metrics::ICON_SIZE_LG)
                .color(cx.theme().status().warning),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .gap_x(metrics::SPACE_2)
                .child(
                    div()
                        .font_weight(metrics::FONT_WEIGHT_BOLD)
                        .child(t!(cx, "offline-message").to_string()),
                )
                .child(div().min_w_0().child(t!(cx, "offline-detail").to_string())),
        )
        .child(components::controls::on_activate_app(
            components::controls::compact(components::controls::button(
                "offline-retry-now",
                t!(cx, "offline-retry-now").to_string(),
                components::controls::ButtonKind::Secondary,
                true,
                colors,
            ))
            .flex_none()
            .debug_selector(|| "offline-retry-now".into()),
            on_retry,
        ))
}
