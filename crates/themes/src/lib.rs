//! Themes: Parla's own theme model, a registry and the active theme as GPUI globals.
//!
//! Theme files are theme-family JSON in the shape Zed's public theme files use (see
//! [`theme_json`]). Chat-specific tokens live in an optional
//! `parla` object inside each theme's `style` (see [`parla_tokens`]).

mod accent;
pub mod base_theme;
mod builtin;
pub mod color;
pub mod fonts;
mod load;
pub mod metrics;
mod model;
mod parla_tokens;
mod theme_json;

use std::sync::Arc;

use gpui::{App, Global, WindowAppearance};
use settings::{Settings, ThemeMode, ThemeSelection};

pub use accent::accent_swatch;
pub use load::{ThemeLoadError, load_user_themes};
pub use model::{
    ActiveTheme, Appearance, StatusColors, Theme, ThemeColors, ThemeKind, ThemeNotFound,
    ThemeRegistry,
};
pub use parla_tokens::{LogoTokens, ParlaTokens, parla_tokens};

/// Name of the built-in light theme.
pub const DEFAULT_LIGHT: &str = settings::DEFAULT_LIGHT_THEME;
/// Name of the built-in dark theme.
pub const DEFAULT_DARK: &str = settings::DEFAULT_DARK_THEME;

/// Name of the built-in high contrast light theme (WCAG AAA text).
pub const HIGH_CONTRAST_LIGHT: &str = "Parla High Contrast Light";
/// Name of the built-in high contrast dark theme (WCAG AAA text).
pub const HIGH_CONTRAST_DARK: &str = "Parla High Contrast Dark";

/// Whether the OS asks for more contrast (macOS "Increase contrast"). Set by
/// the app from the platform and its change notification; absent means off.
struct IncreaseContrast(bool);

impl Global for IncreaseContrast {}

/// Records the OS "Increase contrast" switch. The next [`apply`] follows it.
pub fn set_increase_contrast(cx: &mut App, on: bool) {
    cx.set_global(IncreaseContrast(on));
}

/// The last value given to [`set_increase_contrast`] (`false` when never set).
pub fn increase_contrast(cx: &App) -> bool {
    cx.try_global::<IncreaseContrast>().is_some_and(|c| c.0)
}

/// Which theme slots the user set in `settings.json` (`theme.light`,
/// `theme.dark`). A set slot is the user's choice and is never swapped for
/// high contrast, even when it names Parla Light or Parla Dark.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExplicitThemes {
    pub light: bool,
    pub dark: bool,
}

impl Global for ExplicitThemes {}

/// Records which theme slots the user set. The next [`apply`] follows it.
pub fn set_explicit_themes(cx: &mut App, explicit: ExplicitThemes) {
    cx.set_global(explicit);
}

/// The last value given to [`set_explicit_themes`] (none set when never given).
pub fn explicit_themes(cx: &App) -> ExplicitThemes {
    cx.try_global::<ExplicitThemes>()
        .copied()
        .unwrap_or_default()
}

/// Whether accent presets recolour the active theme: not on the built-in high
/// contrast themes, whose 7:1 pairs a preset (4.5:1) would break.
pub fn accent_presets_apply(cx: &App) -> bool {
    cx.theme().kind() != ThemeKind::HighContrast
}

/// The names of the themes that render for the light and the dark appearance
/// under `sel`, after the high contrast swap. The Appearance page marks these.
pub fn rendered_names(cx: &App, sel: &ThemeSelection) -> (String, String) {
    let registry = ThemeRegistry::global(cx);
    let (on, explicit) = (increase_contrast(cx), explicit_themes(cx));
    let name = |mode| {
        let pinned = ThemeSelection {
            mode,
            ..sel.clone()
        };
        let appearance = WindowAppearance::Light;
        resolve_with_contrast(&registry, &pinned, appearance, on, explicit)
            .0
            .name
            .to_string()
    };
    (name(ThemeMode::Light), name(ThemeMode::Dark))
}

/// The background of an avatar of `hue` (0 to 1) in `theme`. The built-in
/// high contrast themes darken it so `avatar_text` reads at 7:1 on every hue.
pub fn avatar_fill(hue: f32, theme: &Theme) -> gpui::Hsla {
    let lightness = if theme.kind() == ThemeKind::HighContrast {
        0.22
    } else {
        0.38
    };
    gpui::hsla(hue, 0.45, lightness, 1.0)
}

/// Why a theme selection could not be honoured exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeWarning {
    /// No theme with this name is registered; a built-in default was used.
    NotFound(String),
}

impl std::fmt::Display for ThemeWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(name) => write!(f, "theme not found: {name}"),
        }
    }
}

/// Registers the built-in themes in the [`ThemeRegistry`] global and activates the
/// theme selected for the current system appearance.
pub fn init(cx: &mut App, settings: &Settings) {
    ThemeRegistry::set_global(cx);
    builtin::load(&ThemeRegistry::global(cx));
    apply(cx, settings);
}

/// [`init`] with default settings, for tests that only need an active theme.
pub fn init_default(cx: &mut App) {
    init(cx, &Settings::default());
}

/// Re-applies `settings` (theme selection) after a reload.
/// Returns a warning when the selected theme name is not registered.
pub fn apply(cx: &mut App, settings: &Settings) -> Option<ThemeWarning> {
    let registry = ThemeRegistry::global(cx);
    let (active, warning) = resolve_with_contrast(
        &registry,
        &settings.theme,
        cx.window_appearance(),
        increase_contrast(cx),
        explicit_themes(cx),
    );
    // A preset only promises 4.5:1; the built-in high contrast themes keep 7:1.
    let active = if settings.accent_preset == settings::AccentPreset::Brand
        || active.kind() == ThemeKind::HighContrast
    {
        active
    } else {
        Arc::new(active.with_accent(settings.accent_preset))
    };
    model::set_active(cx, active);
    // gpui-base reads one global theme; keep it in step with every apply.
    base_theme::install(cx);
    warning
}

/// Picks the theme for `sel`, following `system` when the mode is `System`.
/// An unknown name falls back to [`DEFAULT_LIGHT`] / [`DEFAULT_DARK`] with a warning.
pub fn resolve(
    registry: &ThemeRegistry,
    sel: &ThemeSelection,
    system: WindowAppearance,
) -> (Arc<Theme>, Option<ThemeWarning>) {
    let dark = slot_is_dark(sel.mode, system);
    let (wanted, default) = if dark {
        (&sel.dark, DEFAULT_DARK)
    } else {
        (&sel.light, DEFAULT_LIGHT)
    };
    if let Ok(theme) = registry.get(wanted) {
        return (theme, None);
    }
    let warning = ThemeWarning::NotFound(wanted.clone());
    let theme = registry
        .get(default)
        .expect("built-in themes are registered by init");
    (theme, Some(warning))
}

/// [`resolve`], then, when the OS asks for more contrast, the built-in brand
/// themes become their high contrast pair, unless the user set that slot in
/// `settings.json` (`explicit`). A user theme, whatever its name, stays.
pub fn resolve_with_contrast(
    registry: &ThemeRegistry,
    sel: &ThemeSelection,
    system: WindowAppearance,
    increase_contrast: bool,
    explicit: ExplicitThemes,
) -> (Arc<Theme>, Option<ThemeWarning>) {
    let (theme, warning) = resolve(registry, sel, system);
    if !increase_contrast || theme.kind() != ThemeKind::Brand {
        return (theme, warning);
    }
    let chosen = if slot_is_dark(sel.mode, system) {
        explicit.dark
    } else {
        explicit.light
    };
    let swap = match theme.appearance {
        Appearance::Light => HIGH_CONTRAST_LIGHT,
        Appearance::Dark => HIGH_CONTRAST_DARK,
    };
    // A missing name fell back to the default: that is not the user's choice.
    if chosen && warning.is_none() {
        return (theme, warning);
    }
    match registry.get(swap) {
        Ok(hc) if hc.kind() == ThemeKind::HighContrast => (hc, warning),
        _ => (theme, warning),
    }
}

/// Whether `mode` under the OS `system` appearance uses the dark slot.
fn slot_is_dark(mode: ThemeMode, system: WindowAppearance) -> bool {
    match mode {
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
        ThemeMode::System => matches!(
            system,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    }
}
