//! The Base theme global follows the active Parla theme and its metrics.

use themes::ActiveTheme as _;

#[gpui::test]
fn the_base_theme_maps_the_active_theme(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        let ours = *cx.theme().colors();
        let base = gpui_base::Theme::global(cx);
        assert_eq!(base.tokens.colors.background, ours.background);
        assert_eq!(base.tokens.colors.foreground, ours.text);
        assert_eq!(base.tokens.colors.surface, ours.surface_background);
        assert_eq!(base.tokens.colors.border, ours.border);
        assert_eq!(base.tokens.colors.ring, ours.border_focused);
        assert_eq!(base.tokens.radius.lg, themes::metrics::RADIUS_LG);
        assert_eq!(base.tokens.spacing.md, themes::metrics::SPACE_5);
    });
}

#[gpui::test]
fn applying_another_theme_refreshes_the_base_theme(cx: &mut gpui::TestAppContext) {
    let mut settings = settings::Settings::default();
    settings.theme.mode = settings::ThemeMode::Dark;
    cx.update(|cx| {
        themes::init(cx, &settings);
        let base = gpui_base::Theme::global(cx);
        assert_eq!(base.appearance, gpui_base::ThemeAppearance::Dark);
        assert_eq!(
            base.tokens.colors.background,
            cx.theme().colors().background
        );
    });
}
