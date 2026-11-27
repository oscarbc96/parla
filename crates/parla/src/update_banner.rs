//! The "update available" strip: **Update**, progress, **Restart to update**
//! and the failure states, each with the download link as the fallback.

use gpui::{App, IntoElement, Window, div, prelude::*};
use themes::{ActiveTheme as _, metrics, parla_tokens};

use crate::toast::ConfigBanner;
use crate::updates::{UpdateNotice, UpdatePhase};

/// Renders the strip for `notice`. The buttons run the update flow directly
/// (`crate::updates`), so the strip works the same in the app and in tests.
pub fn update_banner(cx: &App, notice: &UpdateNotice) -> impl IntoElement {
    let colors = cx.theme().colors();
    let tokens = parla_tokens(cx.theme());
    let failed = matches!(notice.phase, UpdatePhase::Failed(_));
    let (ground, text) = if failed {
        (tokens.danger_soft, colors.text)
    } else {
        (colors.accent_soft, tokens.accent_soft_text)
    };
    let offered = matches!(notice.phase, UpdatePhase::Offer);
    let dismissable = offered || failed;
    let url = notice.url.clone();
    let tag = notice.tag.clone();

    let ring = components::controls::ring(colors);
    let dismiss_label: gpui::SharedString = t!(cx, "update-dismiss").to_string().into();
    let message = div()
        .id("update-text")
        .debug_selector(|| "update-text".into())
        .flex_1()
        .min_w_0()
        .font_weight(metrics::FONT_WEIGHT_BOLD)
        .child(notice.text.clone());
    // Clicking the text dismisses an offer or a failure; the keyboard reaches
    // it too (Tab, then Enter or Space) with the focus ring.
    let message = if dismissable {
        components::controls::on_activate_app(
            message
                .role(gpui::Role::Button)
                .aria_label(dismiss_label)
                .cursor_pointer()
                .tab_stop(true)
                .focusable()
                .focus_visible(move |s| s.shadow(ring.clone())),
            |window: &mut Window, cx: &mut App| {
                cx.global_mut::<ConfigBanner>().clear_update_notice();
                window.refresh();
            },
        )
    } else {
        message
    };

    let button = |id: &'static str, label: &str, primary: bool| {
        components::controls::compact(components::controls::button(
            id,
            label.to_string(),
            if primary {
                components::controls::ButtonKind::Primary
            } else {
                components::controls::ButtonKind::Secondary
            },
            true,
            colors,
        ))
        .flex_none()
        .debug_selector(move || id.into())
    };

    div()
        .id("update-banner")
        .debug_selector(|| "update-banner".into())
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .px(metrics::SPACE_6)
        .py(metrics::SPACE_5)
        .bg(ground)
        .text_color(text)
        .text_size(metrics::FONT_SIZE_MD)
        .child(message)
        .children(notice.install_label.as_ref().map(|label| {
            components::controls::on_activate_app(
                button("update-install", label, true),
                |window: &mut Window, cx: &mut App| {
                    crate::updates::start_install(cx);
                    window.refresh();
                },
            )
        }))
        .children(notice.restart_label.as_ref().map(|label| {
            components::controls::on_activate_app(
                button("update-restart", label, true),
                |_: &mut Window, cx: &mut App| crate::updates::restart_to_update(cx),
            )
        }))
        .children((offered || failed).then(|| {
            components::controls::on_activate_app(
                button("update-download", &notice.download_label, false),
                move |_: &mut Window, cx: &mut App| cx.open_url(&url),
            )
        }))
        .children(offered.then(|| {
            components::controls::on_activate_app(
                button("update-skip", &notice.skip_label, false),
                move |window: &mut Window, cx: &mut App| {
                    crate::updates::skip_version(&tag, cx);
                    window.refresh();
                },
            )
        }))
}
