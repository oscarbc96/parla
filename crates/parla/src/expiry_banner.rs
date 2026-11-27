//! The "session expired" strip with a sign-in-again button.

use gpui::{App, IntoElement, Window, div, prelude::*};
use themes::{ActiveTheme as _, metrics, parla_tokens};

/// Renders the banner, naming `workspace`. `on_sign_in` runs when the button is clicked, or
/// activated with Enter or Space.
pub fn expiry_banner(
    cx: &App,
    workspace: &str,
    on_sign_in: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let colors = cx.theme().colors();
    let soft = parla_tokens(cx.theme()).danger_soft;
    // Danger-soft ground, a bold title then plain text, and the one action as
    // a compact primary button (the States board).
    div()
        .debug_selector(|| "expiry-banner".into())
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .px(metrics::SPACE_6)
        .py(metrics::SPACE_5)
        .bg(soft)
        .text_color(colors.text)
        .text_size(metrics::FONT_SIZE_MD)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .gap_x(metrics::SPACE_2)
                .child(
                    div().font_weight(metrics::FONT_WEIGHT_BOLD).child(
                        t!(cx, "expiry-message", workspace = workspace.to_string()).to_string(),
                    ),
                )
                .child(t!(cx, "expiry-detail").to_string()),
        )
        .child(components::controls::on_activate_app(
            components::controls::compact(components::controls::button(
                "sign-in-again",
                t!(cx, "expiry-sign-in-again").to_string(),
                components::controls::ButtonKind::Primary,
                true,
                colors,
            ))
            .flex_none()
            .debug_selector(|| "sign-in-again".into()),
            on_sign_in,
        ))
}
