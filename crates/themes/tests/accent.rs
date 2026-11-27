//! The global accent preset recolours the accent tokens of both built-in themes live
//! and keeps every text/fill pair readable (WCAG AA text, 3:1 for the fill).

use gpui::{Hsla, TestAppContext};
use settings::{AccentPreset, Settings, ThemeMode};
use themes::color::{hsla_to_rgba, rgb_to_hsla};
use themes::{ActiveTheme, Appearance, ThemeColors, init, parla_tokens};

fn lin(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn lum(c: Hsla) -> f32 {
    let c = hsla_to_rgba(c);
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

fn ratio(a: Hsla, b: Hsla) -> f32 {
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `top` laid over opaque `under`.
fn over(top: Hsla, under: Hsla) -> Hsla {
    let (t, u) = (hsla_to_rgba(top), hsla_to_rgba(under));
    let mix = |a: f32, b: f32| a * t.a + b * (1.0 - t.a);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    let packed = (byte(mix(t.r, u.r)) << 24)
        | (byte(mix(t.g, u.g)) << 16)
        | (byte(mix(t.b, u.b)) << 8)
        | 0xff;
    rgb_to_hsla(gpui::rgba(packed))
}

fn settings(mode: ThemeMode, preset: AccentPreset) -> Settings {
    let mut s = Settings::default();
    s.theme.mode = mode;
    s.accent_preset = preset;
    s
}

fn active(cx: &mut TestAppContext, mode: ThemeMode, preset: AccentPreset) -> (ThemeColors, Hsla) {
    cx.update(|cx| {
        init(cx, &Settings::default());
        themes::apply(cx, &settings(mode, preset));
        let soft_text = parla_tokens(cx.theme()).accent_soft_text;
        (*cx.theme().colors(), soft_text)
    })
}

#[gpui::test]
fn brand_keeps_the_theme_accent_and_other_presets_change_it(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        let (brand, _) = active(cx, mode, AccentPreset::Brand);
        for p in [
            AccentPreset::Ember,
            AccentPreset::Amber,
            AccentPreset::Coral,
            AccentPreset::Ink,
        ] {
            let (c, _) = active(cx, mode, p);
            let same_as_brand = c.element_background == brand.element_background
                && c.text_accent == brand.text_accent
                && c.accent_soft == brand.accent_soft
                && c.element_selected == brand.element_selected
                && c.border_focused == brand.border_focused;
            // Dark ember and brand share the brand orange fill; the rest must differ.
            if !(mode == ThemeMode::Dark && p == AccentPreset::Ember) {
                assert!(!same_as_brand, "{p:?} {mode:?} equals brand");
            }
        }
    }
}

#[gpui::test]
fn switching_back_to_brand_restores_the_theme(cx: &mut TestAppContext) {
    let (before, _) = active(cx, ThemeMode::Light, AccentPreset::Brand);
    let after = cx.update(|cx| {
        themes::apply(cx, &settings(ThemeMode::Light, AccentPreset::Coral));
        themes::apply(cx, &settings(ThemeMode::Light, AccentPreset::Brand));
        *cx.theme().colors()
    });
    assert_eq!(before, after);
}

#[gpui::test]
fn every_preset_passes_contrast_in_both_themes(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for p in AccentPreset::ALL {
            let (c, soft_text) = active(cx, mode, p);
            let tag = format!("{p:?} {mode:?}");
            assert!(
                ratio(c.text_on_accent, c.element_background) >= 4.5,
                "{tag}: text on accent fill"
            );
            assert!(
                ratio(c.element_background, c.background) >= 3.0,
                "{tag}: accent fill against background"
            );
            for (name, ground) in [
                ("background", c.background),
                ("panel", c.panel_background),
                ("surface", c.surface_background),
            ] {
                assert!(
                    ratio(c.text_accent, ground) >= 4.5,
                    "{tag}: accent text on {name}"
                );
            }
            for (name, ground) in [
                ("background", c.background),
                ("panel", c.panel_background),
                ("surface", c.surface_background),
            ] {
                let soft = over(c.accent_soft, ground);
                assert!(
                    ratio(soft_text, soft) >= 4.5,
                    "{tag}: soft text on the soft wash over {name}"
                );
                assert!(
                    ratio(c.text, soft) >= 4.5,
                    "{tag}: body text on the soft wash over {name}"
                );
            }
            for (name, ground) in [
                ("background", c.background),
                ("panel", c.panel_background),
                ("surface", c.surface_background),
            ] {
                assert!(
                    ratio(c.border_focused, ground) >= 3.0,
                    "{tag}: focus ring on {name}"
                );
            }
            assert!(
                ratio(c.text, c.element_selected) >= 4.5,
                "{tag}: body text on a selected row"
            );
        }
    }
    let _ = Appearance::Light;
}

#[gpui::test]
fn presets_recolour_focus_and_selection(cx: &mut TestAppContext) {
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        let (brand, _) = active(cx, mode, AccentPreset::Brand);
        for p in [AccentPreset::Amber, AccentPreset::Coral, AccentPreset::Ink] {
            let (c, _) = active(cx, mode, p);
            assert_ne!(
                c.border_focused, brand.border_focused,
                "{p:?} {mode:?} focus"
            );
            assert_ne!(
                c.element_selected, brand.element_selected,
                "{p:?} {mode:?} selection"
            );
        }
    }
}
