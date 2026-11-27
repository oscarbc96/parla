//! Maps Parla's design tokens into the `gpui_base::Theme` global that the
//! adopted Base primitives read.
//!
//! Every colour comes from the active theme and every size from
//! [`crate::metrics`]; nothing here is a literal. [`install`] runs after every
//! [`crate::apply`], so a theme or accent change refreshes the Base global in
//! the same step.

use gpui::{App, Pixels, px};
use gpui_base::theme_tokens::{
    ColorTokens, RadiusTokens, SpacingTokens, TextStyleToken, TypographyTokens,
};
use gpui_base::{Theme, ThemeAppearance};
use settings::{Settings, SettingsStore};

use crate::model::Appearance;
use crate::{ActiveTheme as _, metrics, parla_tokens};

/// The design body line height (`line-height.body` in the tokens file).
const LINE_HEIGHT_BODY: f32 = 1.5;

/// The Base theme for `theme`, with typography from `settings`.
pub fn from_theme(theme: &crate::Theme, settings: &Settings) -> Theme {
    let colors = theme.colors();
    let tokens = parla_tokens(theme);
    let mut base = Theme::default();
    base.appearance = match theme.appearance {
        Appearance::Light => ThemeAppearance::Light,
        Appearance::Dark => ThemeAppearance::Dark,
    };

    let mut palette = match theme.appearance {
        Appearance::Light => ColorTokens::light(),
        Appearance::Dark => ColorTokens::dark(),
    };
    palette.background = colors.background;
    palette.foreground = colors.text;
    palette.surface = colors.surface_background;
    palette.surface_foreground = colors.text;
    palette.primary = colors.element_background;
    palette.primary_foreground = colors.text_on_accent;
    palette.secondary = colors.panel_background;
    palette.secondary_foreground = colors.text;
    palette.muted = colors.element_hover;
    palette.muted_foreground = colors.text_muted;
    palette.accent = colors.accent_soft;
    palette.accent_foreground = tokens.accent_soft_text;
    palette.destructive = theme.status().error;
    palette.destructive_foreground = tokens.danger_text;
    palette.border = colors.border;
    palette.input = tokens.border_control;
    palette.ring = colors.border_focused;
    palette.selection = colors.element_selected;
    base.tokens.colors = palette;

    let mut radius = RadiusTokens::default();
    radius.none = metrics::SPACE_0;
    radius.sm = metrics::RADIUS_SM;
    radius.md = metrics::RADIUS_MD;
    radius.lg = metrics::RADIUS_LG;
    radius.xl = metrics::RADIUS_XL;
    radius.full = metrics::RADIUS_FULL;
    base.tokens.radius = radius;

    let mut spacing = SpacingTokens::default();
    spacing.xxs = metrics::SPACE_1;
    spacing.xs = metrics::SPACE_2;
    spacing.sm = metrics::SPACE_4;
    spacing.md = metrics::SPACE_5;
    spacing.lg = metrics::SPACE_6;
    spacing.xl = metrics::SPACE_8;
    spacing.xxl = metrics::SPACE_9;
    base.tokens.spacing = spacing;

    let mut typography = TypographyTokens::default();
    typography.sans = settings.ui_font_family.clone().into();
    typography.mono = settings.buffer_font_family.clone().into();
    typography.xs = text_style(metrics::FONT_SIZE_XS);
    typography.sm = text_style(metrics::FONT_SIZE_SM);
    typography.md = text_style(px(settings.ui_font_size));
    typography.lg = text_style(metrics::FONT_SIZE_LG);
    typography.xl = text_style(metrics::FONT_SIZE_XL);
    typography.mono_md = text_style(metrics::FONT_SIZE_SM);
    base.tokens.typography = typography;

    base
}

/// Installs the Base theme for the active Parla theme.
pub fn install(cx: &mut App) {
    if !crate::model::has_active(cx) {
        return;
    }
    let theme = cx.theme().clone();
    let mapped = match cx.try_global::<SettingsStore>() {
        Some(store) => from_theme(&theme, store.current()),
        None => from_theme(&theme, &Settings::default()),
    };
    cx.set_global(mapped);
}

fn text_style(size: Pixels) -> TextStyleToken {
    let mut style = TextStyleToken::default();
    style.size = size;
    style.line_height = px(size.as_f32() * LINE_HEIGHT_BODY);
    style.weight = metrics::FONT_WEIGHT_REGULAR;
    style
}
