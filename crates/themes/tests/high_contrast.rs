//! Parla High Contrast Light and Dark: every text pair reads at 7:1 (WCAG AAA
//! 1.4.6), focus and cursor rings at 4.5:1 and every border, badge and
//! indicator at 3:1 (WCAG 1.4.11), on every ground they sit on: the app
//! grounds, the sidebar, its active and hovered rows, the rail, the tour dim
//! and the media viewer. They replace the built-in brand themes while the OS
//! asks for more contrast, unless the user chose a theme, and can be picked by
//! name.

use gpui::{Hsla, TestAppContext, WindowAppearance};
use settings::{AccentPreset, Settings, ThemeMode, ThemeSelection};
use themes::color::{hsla_to_rgba, rgb_to_hsla};
use themes::{
    ActiveTheme, DEFAULT_DARK, DEFAULT_LIGHT, ExplicitThemes, HIGH_CONTRAST_DARK,
    HIGH_CONTRAST_LIGHT, ThemeKind, ThemeRegistry, init, load_user_themes, parla_tokens,
    resolve_with_contrast,
};

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

fn setup(cx: &mut TestAppContext) -> std::sync::Arc<ThemeRegistry> {
    cx.update(|cx| {
        init(cx, &Settings::default());
        ThemeRegistry::global(cx)
    })
}

/// Collects every pair under its minimum so one run lists them all.
struct Checker {
    theme: String,
    failures: Vec<String>,
    checked: usize,
}

impl Checker {
    fn need(&mut self, fg: (&str, Hsla), bg: (&str, Hsla), min: f32) {
        self.checked += 1;
        let r = ratio(fg.1, bg.1);
        if r < min {
            self.failures.push(format!(
                "{}: {} on {} is {r:.2}:1 < {min}:1",
                self.theme, fg.0, bg.0
            ));
        }
    }

    fn each(&mut self, fgs: &[(&str, Hsla)], bgs: &[(&str, Hsla)], min: f32) {
        for fg in fgs {
            for bg in bgs {
                self.need(*fg, *bg, min);
            }
        }
    }
}

#[gpui::test]
fn high_contrast_themes_meet_their_ratios_on_every_ground(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let mut failures = Vec::new();
    for name in [HIGH_CONTRAST_LIGHT, HIGH_CONTRAST_DARK] {
        let theme = registry
            .get(name)
            .unwrap_or_else(|_| panic!("{name} not registered"));
        let c = *theme.colors();
        let s = *theme.status();
        let t = parla_tokens(&theme);
        let mut k = Checker {
            theme: name.into(),
            failures: Vec::new(),
            checked: 0,
        };
        let app = [
            ("background", c.background),
            ("surface", c.surface_background),
            ("raised", c.elevated_surface_background),
            ("panel", c.panel_background),
            ("title bar", c.title_bar_background),
            ("editor", c.editor_background),
            ("hover", c.element_hover),
            ("selected", c.element_selected),
            ("active", c.element_active),
            ("sunken", t.bg_sunken),
            ("mention row", t.message_mention_background),
            ("accent soft", over(c.accent_soft, c.surface_background)),
            (
                "read highlight",
                over(
                    c.editor_document_highlight_read_background,
                    c.surface_background,
                ),
            ),
        ];
        let sidebar = [
            ("sidebar", t.sidebar_background),
            ("sidebar active row", t.sidebar_active),
            (
                "sidebar hover row",
                over(t.sidebar_hover, t.sidebar_background),
            ),
            ("rail", t.sidebar_rail),
            ("rail hover", over(t.sidebar_hover, t.sidebar_rail)),
        ];
        let dim = [
            ("tour dim on background", over(t.bg_overlay, c.background)),
            (
                "tour dim on surface",
                over(t.bg_overlay, c.surface_background),
            ),
        ];
        let viewer = [
            ("viewer", t.viewer_bg),
            ("viewer stage", t.viewer_stage),
            (
                "viewer control hover",
                over(t.viewer_control_hover, t.viewer_bg),
            ),
            ("viewer nav", over(t.viewer_nav_bg, t.viewer_stage)),
        ];
        let every: Vec<(&str, Hsla)> = app.iter().chain(sidebar.iter()).copied().collect();

        // Text: 7:1.
        k.each(
            &[
                ("text", c.text),
                ("text muted", c.text_muted),
                ("text subtle", t.text_subtle),
                ("link", c.text_accent),
            ],
            &app,
            7.0,
        );
        k.each(
            &[
                ("sidebar text", t.sidebar_text),
                ("sidebar muted", t.sidebar_muted),
                ("sidebar strong", t.sidebar_text_strong),
            ],
            &sidebar,
            7.0,
        );
        k.each(
            &[
                ("signal text", t.signal_text),
                ("mention text", t.mention_text),
            ],
            &[
                ("background", c.background),
                ("surface", c.surface_background),
                ("raised", c.elevated_surface_background),
                ("signal soft", over(t.signal_soft, c.surface_background)),
                ("mention row", t.message_mention_background),
            ],
            7.0,
        );
        k.need(
            ("accent soft text", t.accent_soft_text),
            ("accent soft", over(c.accent_soft, c.surface_background)),
            7.0,
        );
        for (fg, soft) in [
            (("success", s.success), ("success soft", t.success_soft)),
            (("warning", s.warning), ("warning soft", t.warning_soft)),
            (("danger", s.error), ("danger soft", t.danger_soft)),
        ] {
            k.each(
                &[fg],
                &[
                    ("background", c.background),
                    ("surface", c.surface_background),
                    ("raised", c.elevated_surface_background),
                    (soft.0, over(soft.1, c.surface_background)),
                ],
                7.0,
            );
        }
        k.need(
            ("text on accent", c.text_on_accent),
            ("accent fill", c.element_background),
            7.0,
        );
        k.need(("badge text", t.danger_text), ("badge", s.error), 7.0);
        k.need(
            ("warning text", t.warning_text),
            ("warning", s.warning),
            7.0,
        );
        k.each(
            &[
                ("keyword", t.code_keyword),
                ("string", t.code_string),
                ("function", t.code_function),
                ("comment", t.code_comment),
                ("number", t.code_number),
                ("type", t.code_type),
            ],
            &[("editor", c.editor_background), ("sunken", t.bg_sunken)],
            7.0,
        );
        k.each(
            &[
                ("viewer text", t.viewer_text),
                ("viewer muted", t.viewer_muted),
            ],
            &viewer,
            7.0,
        );
        // Initials on every avatar hue.
        for deg in 0..360 {
            let hue = deg as f32 / 360.0;
            k.need(
                ("avatar text", t.avatar_text),
                ("avatar", themes::avatar_fill(hue, &theme)),
                7.0,
            );
        }

        // Focus and cursor rings: 4.5:1 on every ground, 3:1 on the fill.
        k.each(
            &[
                ("focus ring", c.border_focused),
                ("control border", t.border_control),
            ],
            &every,
            4.5,
        );
        k.need(
            ("focus ring", c.border_focused),
            ("accent fill", c.element_background),
            3.0,
        );
        k.each(&[("viewer focus", t.viewer_focus)], &viewer, 4.5);

        // Borders, badges and indicators: 3:1 on every ground.
        k.each(
            &[
                ("border", c.border),
                ("strong border", c.border_strong),
                ("subtle border", t.border_subtle),
                ("code border", t.code_border),
                ("presence active", t.presence_active),
                ("presence away", t.presence_away),
                ("signal", t.signal),
                ("mention badge", s.error),
            ],
            &every,
            3.0,
        );
        k.each(
            &[(
                "sidebar border",
                over(t.sidebar_border, t.sidebar_background),
            )],
            &sidebar,
            3.0,
        );
        k.each(
            &[("viewer border", over(t.viewer_border, t.viewer_bg))],
            &viewer,
            3.0,
        );
        // The tour spotlight ring stands out on the dim and on the target.
        k.each(&[("tour ring", t.signal)], &dim, 3.0);

        // Logo: the bubble (or its rim) on the background, drops on the bubble.
        let logo = t.logo;
        let edge = ratio(logo.bubble, logo.background)
            .max(ratio(over(logo.rim, logo.background), logo.background));
        k.checked += 1;
        if edge < 3.0 {
            k.failures.push(format!(
                "{name}: logo bubble and rim on background {edge:.2}:1 < 3:1"
            ));
        }
        k.each(
            &[
                ("logo accent 1", logo.accent_1),
                ("logo accent 2", logo.accent_2),
                ("logo accent 3", logo.accent_3),
            ],
            &[("logo bubble", logo.bubble)],
            3.0,
        );

        assert!(k.checked > 600, "{name}: only {} pairs checked", k.checked);
        failures.extend(k.failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[gpui::test]
fn brand_avatars_keep_their_colour(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let light = registry.get(DEFAULT_LIGHT).unwrap();
    assert_eq!(
        themes::avatar_fill(0.3, &light),
        gpui::hsla(0.3, 0.45, 0.38, 1.0)
    );
}

fn system() -> ThemeSelection {
    ThemeSelection {
        mode: ThemeMode::System,
        light: DEFAULT_LIGHT.into(),
        dark: DEFAULT_DARK.into(),
    }
}

#[gpui::test]
fn increase_contrast_swaps_the_brand_themes_for_high_contrast(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let none = ExplicitThemes::default();
    let pick = |system_appearance, on, explicit| {
        resolve_with_contrast(&registry, &system(), system_appearance, on, explicit)
            .0
            .name
            .to_string()
    };
    assert_eq!(
        pick(WindowAppearance::Light, true, none),
        HIGH_CONTRAST_LIGHT
    );
    assert_eq!(pick(WindowAppearance::Dark, true, none), HIGH_CONTRAST_DARK);
    assert_eq!(pick(WindowAppearance::Light, false, none), DEFAULT_LIGHT);
}

#[gpui::test]
fn an_explicit_brand_theme_wins_over_increase_contrast(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let light_chosen = ExplicitThemes {
        light: true,
        dark: false,
    };
    let (light, _) = resolve_with_contrast(
        &registry,
        &system(),
        WindowAppearance::Light,
        true,
        light_chosen,
    );
    assert_eq!(light.name.as_ref(), DEFAULT_LIGHT);
    let (dark, _) = resolve_with_contrast(
        &registry,
        &system(),
        WindowAppearance::Dark,
        true,
        light_chosen,
    );
    assert_eq!(dark.name.as_ref(), HIGH_CONTRAST_DARK);
}

#[gpui::test]
fn increase_contrast_keeps_a_theme_the_user_picked(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let sel = ThemeSelection {
        mode: ThemeMode::Dark,
        light: DEFAULT_LIGHT.into(),
        dark: "My Theme".into(),
    };
    // An unknown name falls back to the built-in default, which follows the OS.
    let (dark, warning) = resolve_with_contrast(
        &registry,
        &sel,
        WindowAppearance::Dark,
        true,
        ExplicitThemes::default(),
    );
    assert_eq!(dark.name.as_ref(), HIGH_CONTRAST_DARK);
    assert!(warning.is_some());
}

/// A user theme that reuses a built-in name is not swapped, and a user theme
/// named like a high contrast one still takes accent presets.
#[gpui::test]
fn only_the_builtin_themes_swap_and_resist_presets(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("mine.json"),
        r##"{ "name": "Mine", "themes": [
            { "name": "Parla Light", "appearance": "light", "style": { "background": "#fafafaff" } },
            { "name": "Parla High Contrast Dark", "appearance": "dark", "style": { "background": "#101010ff" } }
        ] }"##,
    )
    .unwrap();
    assert!(load_user_themes(&registry, dir.path()).is_empty());
    assert_eq!(registry.get(DEFAULT_LIGHT).unwrap().kind(), ThemeKind::User);
    let (light, _) = resolve_with_contrast(
        &registry,
        &system(),
        WindowAppearance::Light,
        true,
        ExplicitThemes::default(),
    );
    assert_eq!(light.name.as_ref(), DEFAULT_LIGHT);
    assert_eq!(light.kind(), ThemeKind::User);

    cx.update(|cx| {
        let mut s = Settings::default();
        s.theme.mode = ThemeMode::Dark;
        s.theme.dark = HIGH_CONTRAST_DARK.into();
        s.accent_preset = AccentPreset::Coral;
        themes::apply(cx, &s);
        let user = ThemeRegistry::global(cx).get(HIGH_CONTRAST_DARK).unwrap();
        assert_ne!(
            cx.theme().colors().element_background,
            user.colors().element_background,
            "a user theme takes the preset whatever its name"
        );
    });
}

#[gpui::test]
fn apply_follows_the_switch_and_the_builtin_high_contrast_ignores_presets(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut s = Settings::default();
        s.theme.mode = ThemeMode::Light;
        init(cx, &s);
        assert_eq!(cx.theme().name.as_ref(), DEFAULT_LIGHT);
        assert!(!themes::increase_contrast(cx));
        themes::set_increase_contrast(cx, true);
        assert!(themes::increase_contrast(cx));
        s.accent_preset = AccentPreset::Coral;
        themes::apply(cx, &s);
        assert_eq!(cx.theme().name.as_ref(), HIGH_CONTRAST_LIGHT);
        assert_eq!(cx.theme().kind(), ThemeKind::HighContrast);
        assert!(!themes::accent_presets_apply(cx));
        let hc = ThemeRegistry::global(cx).get(HIGH_CONTRAST_LIGHT).unwrap();
        assert_eq!(
            cx.theme().colors().element_background,
            hc.colors().element_background,
            "an accent preset would drop the AAA pairs"
        );
        // The user chose Parla Light in settings.json: it wins.
        themes::set_explicit_themes(
            cx,
            ExplicitThemes {
                light: true,
                dark: false,
            },
        );
        themes::apply(cx, &s);
        assert_eq!(cx.theme().name.as_ref(), DEFAULT_LIGHT);
        assert!(themes::accent_presets_apply(cx));
        themes::set_explicit_themes(cx, ExplicitThemes::default());
        themes::set_increase_contrast(cx, false);
        themes::apply(cx, &s);
        assert_eq!(cx.theme().name.as_ref(), DEFAULT_LIGHT);
    });
}

/// What the Appearance page marks: the themes that render for each mode.
#[gpui::test]
fn rendered_names_follow_increase_contrast(cx: &mut TestAppContext) {
    cx.update(|cx| {
        init(cx, &Settings::default());
        let sel = system();
        assert_eq!(
            themes::rendered_names(cx, &sel),
            (DEFAULT_LIGHT.to_string(), DEFAULT_DARK.to_string())
        );
        themes::set_increase_contrast(cx, true);
        assert_eq!(
            themes::rendered_names(cx, &sel),
            (
                HIGH_CONTRAST_LIGHT.to_string(),
                HIGH_CONTRAST_DARK.to_string()
            )
        );
        themes::set_explicit_themes(
            cx,
            ExplicitThemes {
                light: false,
                dark: true,
            },
        );
        assert_eq!(
            themes::rendered_names(cx, &sel),
            (HIGH_CONTRAST_LIGHT.to_string(), DEFAULT_DARK.to_string())
        );
    });
}

/// The code viewer marks find matches with `signal.soft` over the code ground
/// (and over the hover ground on the current match's line). Some syntax colours
/// fall under 4.5:1 there (Parla Light string and number, Parla Dark comment), so
/// a match is drawn in `text`, which keeps 4.5:1 on both grounds in all four
/// built-in themes.
#[gpui::test]
fn match_text_stays_legible_on_the_find_match_ground(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let mut c = Checker {
        theme: String::new(),
        failures: Vec::new(),
        checked: 0,
    };
    for name in [
        DEFAULT_LIGHT,
        DEFAULT_DARK,
        HIGH_CONTRAST_LIGHT,
        HIGH_CONTRAST_DARK,
    ] {
        let theme = registry.get(name).unwrap();
        let t = parla_tokens(&theme);
        let colors = theme.colors();
        c.theme = name.to_string();
        let plain = over(t.signal_soft, colors.surface_background);
        let current = over(
            t.signal_soft,
            over(colors.element_hover, colors.surface_background),
        );
        c.each(
            &[("text", colors.text)],
            &[
                ("signal.soft over bg.surface", plain),
                ("signal.soft over bg.hover", current),
            ],
            4.5,
        );
    }
    assert!(c.failures.is_empty(), "{}", c.failures.join("\n"));
    assert_eq!(c.checked, 8);
}
