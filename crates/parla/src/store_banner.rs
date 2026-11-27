//! The "cannot open cache" banner and the platform disk-encryption check.

use gpui::{App, ClickEvent, IntoElement, Window, div, prelude::*};
use themes::ActiveTheme as _;

/// Banner text for an unreadable local cache of workspace `display_name`.
pub fn text(l: &i18n::Localizer, display_name: &str) -> String {
    t!(l, "store-banner-unreadable", provider = display_name).into_owned()
}

/// Renders the banner with a "Rebuild cache" button.
pub fn store_banner(
    cx: &App,
    display_name: &str,
    on_rebuild: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let colors = cx.theme().colors();
    div()
        .debug_selector(|| "store-banner".into())
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .py_1()
        .bg(colors.element_background)
        .text_color(colors.text_on_accent)
        .border_b_1()
        .border_color(colors.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(text(&i18n::gpui::localizer(cx), display_name)),
        )
        .child(
            div()
                .id("rebuild-cache")
                .flex_none()
                .debug_selector(|| "rebuild-cache".into())
                .px_2()
                .rounded_md()
                .border_1()
                .border_color(colors.text_on_accent)
                .cursor_pointer()
                .on_click(on_rebuild)
                .child(t!(cx, "store-banner-rebuild").to_string()),
        )
}

/// The "Disk nearly full" strip with a Manage storage button
/// (`on_manage` opens the Settings storage page). Static text: no timer.
pub fn store_full_banner(
    cx: &App,
    on_manage: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    use themes::{metrics, parla_tokens};
    let colors = cx.theme().colors();
    let soft = parla_tokens(cx.theme()).warning_soft;
    div()
        .debug_selector(|| "store-full-banner".into())
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .px(metrics::SPACE_6)
        .py(metrics::SPACE_5)
        .bg(soft)
        .text_color(colors.text)
        .text_size(metrics::FONT_SIZE_MD)
        .child(
            components::icon::Icon::new(components::icon::IconName::Alert)
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
                        .child(t!(cx, "store-full-message").to_string()),
                )
                .child(
                    div()
                        .min_w_0()
                        .child(t!(cx, "store-full-detail").to_string()),
                ),
        )
        .child(components::controls::on_activate_app(
            components::controls::compact(components::controls::button(
                "store-full-manage",
                t!(cx, "store-full-manage").to_string(),
                components::controls::ButtonKind::Secondary,
                true,
                colors,
            ))
            .flex_none()
            .debug_selector(|| "store-full-manage".into()),
            on_manage,
        ))
}

/// Whether full-disk encryption is on. `None` when unknown (not implemented
/// outside macOS until M7).
pub fn disk_encrypted() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("fdesetup")
            .arg("isactive")
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim() == "true")
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Shown once per launch when `disk_encrypted()` is `Some(false)`.
pub fn unencrypted_disk_notice(l: &i18n::Localizer) -> String {
    t!(l, "store-notice-unencrypted").into_owned()
}

/// Runs `probe` (the disk-encryption check, which spawns a process on macOS)
/// on a background thread so it never sits on the path to the first paint,
/// and shows [`unencrypted_disk_notice`] when it reports `Some(false)`.
pub fn warn_if_unencrypted_later(
    cx: &mut gpui::App,
    probe: impl FnOnce() -> Option<bool> + Send + 'static,
) {
    let background = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        if background.spawn(async move { probe() }).await == Some(false) {
            cx.update(|cx| {
                let notice = unencrypted_disk_notice(&i18n::gpui::localizer(cx));
                cx.global_mut::<crate::toast::ConfigBanner>()
                    .set_warnings(crate::toast::WarningSource::Disk, vec![notice]);
                cx.refresh_windows();
            });
        }
    })
    .detach();
}
