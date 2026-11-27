//! Global accent presets: recolours the accent tokens of a theme.
//!
//! The preset colours are the `accent-presets` of `docs/design/parla.tokens.json`.
//! `Brand` leaves the theme as it is; any other preset replaces the accent fill, its
//! text, soft wash and soft text, nudging each toward the theme's text colour until it
//! reads at 4.5:1 on every ground it sits on (WCAG AA).

use gpui::{Hsla, rgba};
use settings::AccentPreset;

use crate::color::{hsla_to_rgba, rgb_to_hsla};
use crate::model::{Appearance, Theme, ThemeColors};
use crate::theme_json::{contrast, readable_on};

/// Preset fill (light, dark) as `0xRRGGBB`; `None` keeps the theme's own accent.
fn fill(preset: AccentPreset, appearance: Appearance) -> Option<u32> {
    let (light, dark) = match preset {
        AccentPreset::Brand => return None,
        AccentPreset::Ember => (0xB83A0F, 0xFE6129),
        AccentPreset::Amber => (0xA15C00, 0xFFA611),
        AccentPreset::Coral => (0xB5163A, 0xFD3459),
        AccentPreset::Ink => (0x4F4C99, 0xADAADB),
    };
    Some(if appearance == Appearance::Light {
        light
    } else {
        dark
    })
}

fn opaque(rgb: u32) -> Hsla {
    rgb_to_hsla(rgba((rgb << 8) | 0xff))
}

/// `top` over an opaque `under`.
fn over(top: Hsla, under: Hsla) -> Hsla {
    let (t, u) = (hsla_to_rgba(top), hsla_to_rgba(under));
    let mix = |a: f32, b: f32| a * t.a + b * (1.0 - t.a);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    rgb_to_hsla(rgba(
        (byte(mix(t.r, u.r)) << 24)
            | (byte(mix(t.g, u.g)) << 16)
            | (byte(mix(t.b, u.b)) << 8)
            | 0xff,
    ))
}

/// `from` moved toward `toward` in small steps until it reads at `min` on every ground.
fn readable(from: Hsla, toward: Hsla, grounds: &[Hsla], min: f32) -> Hsla {
    let (f, t) = (hsla_to_rgba(from), hsla_to_rgba(toward));
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    for step in 0..=20 {
        let k = step as f32 / 20.0;
        let mix = |a: f32, b: f32| a + (b - a) * k;
        let c = rgb_to_hsla(rgba(
            (byte(mix(f.r, t.r)) << 24)
                | (byte(mix(f.g, t.g)) << 16)
                | (byte(mix(f.b, t.b)) << 8)
                | 0xff,
        ));
        if grounds.iter().all(|g| contrast(c, *g) >= min) {
            return c;
        }
    }
    toward
}

impl Theme {
    /// This theme with `preset` applied (a clone for `Brand`).
    pub fn with_accent(&self, preset: AccentPreset) -> Theme {
        let Some(rgb) = fill(preset, self.appearance) else {
            return self.clone();
        };
        let mut c: ThemeColors = *self.colors();
        let fill = opaque(rgb);
        let grounds = [c.background, c.panel_background, c.surface_background];
        let wash = if self.appearance == Appearance::Light {
            0.12
        } else {
            0.16
        };
        c.element_background = fill;
        c.text_on_accent = readable_on(fill, opaque(0xFFFFFF), opaque(0x16151F));
        c.text_accent = readable(fill, c.text, &grounds, 4.5);
        c.accent_soft = fill;
        c.accent_soft.a = wash;
        // Focus ring: 3:1 against every ground (WCAG non-text contrast).
        c.border_focused = readable(fill, c.text, &grounds, 3.0);
        // Selected row: an opaque tint of the fill, so body text stays readable.
        let tint = if self.appearance == Appearance::Light {
            0.22
        } else {
            0.30
        };
        let mut selected = fill;
        selected.a = tint;
        c.element_selected = over(selected, c.panel_background);
        let soft = grounds.map(|g| over(c.accent_soft, g));
        let mut parla = self.parla_overrides();
        parla.accent_soft_text = Some(readable(fill, c.text, &soft, 4.5));
        Theme::new(self.name.clone(), self.appearance, c, *self.status(), parla)
            .with_kind(self.kind())
    }
}

/// The colour of `preset`'s swatch on the Appearance page, for `appearance`.
/// `Brand` is the accent of the theme `settings` selects (before any preset),
/// so a custom theme shows its own accent.
pub fn accent_swatch(cx: &gpui::App, settings: &settings::Settings, preset: AccentPreset) -> Hsla {
    use crate::model::ActiveTheme as _;
    let appearance = cx.theme().appearance;
    match fill(preset, appearance) {
        Some(rgb) => opaque(rgb),
        None => {
            let registry = crate::ThemeRegistry::global(cx);
            let (base, _) = crate::resolve_with_contrast(
                &registry,
                &settings.theme,
                cx.window_appearance(),
                crate::increase_contrast(cx),
                crate::explicit_themes(cx),
            );
            base.colors().element_background
        }
    }
}
