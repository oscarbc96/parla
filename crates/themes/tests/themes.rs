use gpui::{TestAppContext, WindowAppearance};
use settings::{Settings, ThemeMode, ThemeSelection};
use themes::color::rgb_to_hsla;
use themes::{ActiveTheme, Appearance, ThemeRegistry};
use themes::{
    DEFAULT_DARK, DEFAULT_LIGHT, ThemeWarning, init, load_user_themes, parla_tokens, resolve,
};

const SAMPLE: &str = include_str!("fixtures/zed-format-sample.json");
const BUILTIN: &str = include_str!("../../../assets/themes/parla.json");
const HIGH_CONTRAST: &str = include_str!("../../../assets/themes/parla-high-contrast.json");

fn selection(mode: ThemeMode) -> ThemeSelection {
    ThemeSelection {
        mode,
        light: DEFAULT_LIGHT.into(),
        dark: DEFAULT_DARK.into(),
    }
}

fn setup(cx: &mut TestAppContext) -> std::sync::Arc<ThemeRegistry> {
    cx.update(|cx| {
        init(cx, &Settings::default());
        ThemeRegistry::global(cx)
    })
}

#[gpui::test]
fn builtins_registered(cx: &mut TestAppContext) {
    let registry = setup(cx);
    for name in [
        "Parla Light",
        "Parla Dark",
        "Parla High Contrast Light",
        "Parla High Contrast Dark",
    ] {
        assert!(registry.get(name).is_ok(), "{name} not registered");
    }
    // The old built-ins were removed.
    for name in ["Paper", "Ember"] {
        assert!(registry.get(name).is_err(), "{name} must not be registered");
    }
}

#[gpui::test]
fn init_activates_a_parla_theme(cx: &mut TestAppContext) {
    setup(cx);
    let name = cx.update(|cx| cx.theme().name.to_string());
    assert!(
        name == "Parla Light" || name == "Parla Dark",
        "active theme was {name}"
    );
}

#[gpui::test]
fn system_mode_follows_appearance(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let sel = selection(ThemeMode::System);
    let (dark, w1) = resolve(&registry, &sel, WindowAppearance::Dark);
    let (light, w2) = resolve(&registry, &sel, WindowAppearance::Light);
    assert_eq!(dark.name.as_ref(), "Parla Dark");
    assert_eq!(light.name.as_ref(), "Parla Light");
    assert!(w1.is_none() && w2.is_none());
}

#[gpui::test]
fn explicit_mode_ignores_system(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let (t, _) = resolve(
        &registry,
        &selection(ThemeMode::Light),
        WindowAppearance::Dark,
    );
    assert_eq!(t.name.as_ref(), "Parla Light");
    let (t, _) = resolve(
        &registry,
        &selection(ThemeMode::Dark),
        WindowAppearance::Light,
    );
    assert_eq!(t.name.as_ref(), "Parla Dark");
}

#[gpui::test]
fn zed_theme_file_loads(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.json"), SAMPLE).unwrap();
    let errors = load_user_themes(&registry, dir.path());
    assert!(errors.is_empty(), "{errors:?}");
    assert!(registry.get("Sample Dark").is_ok());
    assert!(registry.get("Sample Light").is_ok());
}

#[gpui::test]
fn parla_tokens_fall_back(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.json"), SAMPLE).unwrap();
    load_user_themes(&registry, dir.path());
    let theme = registry.get("Sample Light").unwrap();
    let tokens = parla_tokens(&theme);
    assert_eq!(tokens.sidebar_background, theme.colors().panel_background);
    assert_eq!(tokens.sidebar_text, theme.colors().text);
    assert_eq!(tokens.sidebar_active, theme.colors().element_selected);
    assert_eq!(
        tokens.message_mention_background,
        theme.colors().editor_document_highlight_read_background
    );
    assert_eq!(tokens.presence_active, theme.status().success);
    assert_eq!(tokens.presence_away, theme.status().warning);
    assert_eq!(tokens.avatar_text, gpui::white());
    assert_eq!(tokens.danger_text, gpui::white());
    assert_eq!(tokens.warning_text, gpui::black());
    // The chrome tokens added with the design pass fall back to the sidebar
    // and interface colours.
    let c = theme.colors();
    assert_eq!(tokens.sidebar_rail, tokens.sidebar_background);
    assert_eq!(tokens.sidebar_border, tokens.sidebar_text.opacity(0.1));
    assert_eq!(tokens.sidebar_hover, tokens.sidebar_text.opacity(0.07));
    assert_eq!(tokens.sidebar_muted, tokens.sidebar_text.opacity(0.65));
    assert_eq!(tokens.sidebar_text_strong, tokens.sidebar_text);
    assert_eq!(tokens.signal, c.text_accent);
    assert_eq!(tokens.signal_text, c.text_accent);
    assert_eq!(tokens.signal_soft, c.text_accent.opacity(0.14));
    assert_eq!(tokens.text_subtle, c.text_muted);
    assert_eq!(tokens.mention_text, c.text);
    assert_eq!(tokens.accent_soft_text, c.text);
    assert_eq!(tokens.border_subtle, c.border);
    assert_eq!(tokens.success_soft, theme.status().success.opacity(0.14));
    assert_eq!(tokens.warning_soft, theme.status().warning.opacity(0.14));
    assert_eq!(tokens.danger_soft, theme.status().error.opacity(0.14));
    assert_eq!(tokens.border_control, c.border_strong);
}

#[gpui::test]
fn parla_object_overrides_tokens(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let light = registry.get("Parla Light").unwrap();
    let tokens = parla_tokens(&light);
    assert_ne!(tokens.sidebar_background, light.colors().panel_background);
    for name in ["Parla Light", "Parla Dark"] {
        let tokens = parla_tokens(&registry.get(name).unwrap());
        assert_eq!(tokens.avatar_text, gpui::white(), "{name}");
        // Parla Dark's count badges carry dark text for contrast on the coral.
        let danger = if name == "Parla Dark" {
            rgb_to_hsla(gpui::rgb(0x16151f))
        } else {
            gpui::white()
        };
        assert_eq!(tokens.danger_text, danger, "{name}");
        assert_eq!(tokens.warning_text, gpui::black(), "{name}");
    }
}

#[gpui::test]
fn unknown_theme_falls_back(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let mut sel = selection(ThemeMode::Light);
    sel.light = "Nope".into();
    let (t, warning) = resolve(&registry, &sel, WindowAppearance::Light);
    assert_eq!(t.name.as_ref(), "Parla Light");
    assert_eq!(warning, Some(ThemeWarning::NotFound("Nope".into())));
}

#[gpui::test]
fn removed_builtin_names_fall_back_to_defaults(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let mut sel = selection(ThemeMode::System);
    sel.light = "Paper".into();
    sel.dark = "Ember".into();
    let (light, w) = resolve(&registry, &sel, WindowAppearance::Light);
    assert_eq!(light.name.as_ref(), "Parla Light");
    assert_eq!(w, Some(ThemeWarning::NotFound("Paper".into())));
    let (dark, w) = resolve(&registry, &sel, WindowAppearance::Dark);
    assert_eq!(dark.name.as_ref(), "Parla Dark");
    assert_eq!(w, Some(ThemeWarning::NotFound("Ember".into())));
}

#[gpui::test]
fn bad_theme_file_reported_not_loaded(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bad.json"), "{").unwrap();
    std::fs::write(dir.path().join("one.json"), SAMPLE).unwrap();
    let errors = load_user_themes(&registry, dir.path());
    assert_eq!(errors.len(), 1);
    assert!(errors[0].path.ends_with("bad.json"));
    assert!(registry.get("Sample Dark").is_ok());
}

#[gpui::test]
fn user_theme_overrides_builtin_name(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let before = registry.get("Parla Light").unwrap();
    let before_sidebar = parla_tokens(&before).sidebar_background;
    let dir = tempfile::tempdir().unwrap();
    let custom = r##"{"name":"Mine","author":"me","themes":[{"name":"Parla Light","appearance":"light",
        "style":{"background":"#112233ff","parla":{"sidebar_background":"#445566ff"}}}]}"##;
    std::fs::write(dir.path().join("mine.json"), custom).unwrap();
    let errors = load_user_themes(&registry, dir.path());
    assert!(errors.is_empty(), "{errors:?}");
    let after = registry.get("Parla Light").unwrap();
    assert_ne!(before.colors().background, after.colors().background);
    assert_ne!(before_sidebar, parla_tokens(&after).sidebar_background);
}

#[test]
fn defaults_name_builtin_themes() {
    let sel = Settings::default().theme;
    assert_eq!(sel.light, DEFAULT_LIGHT);
    assert_eq!(sel.dark, DEFAULT_DARK);
}

#[test]
fn builtins_avoid_brand_palette() {
    let forbidden = ["3f0e40", "350d36", "4a154b"];
    let lower = format!("{BUILTIN}{HIGH_CONTRAST}").to_lowercase();
    for f in forbidden {
        assert!(
            !lower.contains(f),
            "built-in theme contains forbidden colour #{f}"
        );
    }
}

#[gpui::test]
fn zed_format_file_maps_keys_and_ignores_the_rest(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sample.json"), SAMPLE).unwrap();
    assert!(load_user_themes(&registry, dir.path()).is_empty());
    let dark = registry.get("Sample Dark").unwrap();
    assert_eq!(dark.appearance, Appearance::Dark);
    let rgb = |hex: u32| rgb_to_hsla(gpui::rgb(hex));
    assert_eq!(dark.colors().background, rgb(0x3b414d));
    assert_eq!(dark.colors().text_accent, rgb(0x74ade8));
    assert_eq!(dark.colors().element_selected, rgb(0x454a56));
    assert_eq!(dark.status().success, rgb(0xa1c181));
    // An unparsable colour is dropped: the warning colour keeps the dark default.
    assert_eq!(dark.status().warning, rgb(0xe0b04a));
    // The `parla` object is kept; a bad entry in it falls back.
    let tokens = parla_tokens(&dark);
    assert_eq!(tokens.sidebar_background, rgb(0x1b1e24));
    assert_eq!(tokens.presence_away, dark.status().warning);
    let light = registry.get("Sample Light").unwrap();
    assert_eq!(light.appearance, Appearance::Light);
    assert_eq!(light.colors().background, rgb(0xdcdcdd));
}

#[gpui::test]
fn logo_tokens_default_per_appearance(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let rgb = |hex: u32| rgb_to_hsla(gpui::rgb(hex));
    // The sample themes set no logo keys: the defaults of their appearance apply.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.json"), SAMPLE).unwrap();
    load_user_themes(&registry, dir.path());
    let light = parla_tokens(&registry.get("Sample Light").unwrap()).logo;
    assert_eq!(light.bubble, rgb(0x211e4c));
    assert_eq!(light.rim.a, 0.0);
    assert_eq!(light.accent_1, rgb(0xffa611));
    assert_eq!(light.accent_2, rgb(0xfe6129));
    assert_eq!(light.accent_3, rgb(0xfd3459));
    assert_eq!(light.background, rgb(0xf8f7f5));
    let dark = parla_tokens(&registry.get("Sample Dark").unwrap()).logo;
    assert_eq!(dark.bubble, rgb(0x211f47));
    assert_eq!(dark.rim, rgb(0x3a3a78));
    assert_eq!(dark.accent_1, rgb(0xfea604));
    assert_eq!(dark.background, rgb(0x121215));
}

#[gpui::test]
fn logo_tokens_read_the_parla_object(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let dir = tempfile::tempdir().unwrap();
    let json = r##"{ "name": "L", "author": "t", "themes": [ { "name": "Logo Theme",
        "appearance": "dark", "style": { "parla": {
        "logo.background": "#010203", "logo.bubble": "#040506", "logo.rim": "#070809ff",
        "logo.accent_1": "#0a0b0c", "logo.accent_2": "nope" } } } ] }"##;
    std::fs::write(dir.path().join("l.json"), json).unwrap();
    assert!(load_user_themes(&registry, dir.path()).is_empty());
    let logo = parla_tokens(&registry.get("Logo Theme").unwrap()).logo;
    let rgb = |hex: u32| rgb_to_hsla(gpui::rgb(hex));
    assert_eq!(logo.background, rgb(0x010203));
    assert_eq!(logo.bubble, rgb(0x040506));
    assert_eq!(logo.rim, rgb(0x070809));
    assert_eq!(logo.accent_1, rgb(0x0a0b0c));
    // Unparsable: falls back to the dark default.
    assert_eq!(logo.accent_2, rgb(0xf85e22));
    assert_eq!(logo.accent_3, rgb(0xfd2e57));
}

#[gpui::test]
fn builtin_themes_set_logo_tokens(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let rgb = |hex: u32| rgb_to_hsla(gpui::rgb(hex));
    let light = parla_tokens(&registry.get("Parla Light").unwrap()).logo;
    assert_eq!(light.accent_1, rgb(0xffa611));
    let dark = parla_tokens(&registry.get("Parla Dark").unwrap()).logo;
    assert_eq!(dark.rim, rgb(0x3a3a78));
}

const TOKENS: &str = include_str!("../../../docs/design/parla.tokens.json");

/// Resolves `{a.b.c}` references in the W3C tokens file down to a hex colour.
fn token_hex(tokens: &serde_json::Value, path: &str) -> u32 {
    let mut value = path.to_string();
    for _ in 0..8 {
        let mut node = tokens;
        for key in value.split('.') {
            node = &node[key];
        }
        let raw = node["$value"].as_str().unwrap_or_else(|| panic!("{value}"));
        if let Some(r) = raw.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            value = r.to_string();
            continue;
        }
        let hex = raw.trim_start_matches('#');
        assert_eq!(hex.len(), 6, "{raw} must be opaque");
        return u32::from_str_radix(hex, 16).unwrap();
    }
    panic!("reference loop at {path}");
}

/// Like `token_hex` but keeps an alpha channel: returns `0xRRGGBBAA`.
fn token_rgba(tokens: &serde_json::Value, path: &str) -> u32 {
    let mut value = path.to_string();
    for _ in 0..8 {
        let mut node = tokens;
        for key in value.split('.') {
            node = &node[key];
        }
        let raw = node["$value"].as_str().unwrap_or_else(|| panic!("{value}"));
        if let Some(r) = raw.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            value = r.to_string();
            continue;
        }
        let hex = raw.trim_start_matches('#');
        let v = u32::from_str_radix(hex, 16).unwrap();
        return if hex.len() == 6 { (v << 8) | 0xff } else { v };
    }
    panic!("reference loop at {path}");
}

#[gpui::test]
fn brand_themes_match_design_tokens(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let tokens: serde_json::Value = serde_json::from_str(TOKENS).unwrap();
    let rgb = |hex: u32| rgb_to_hsla(gpui::rgb(hex));
    for (name, mode) in [
        ("Parla Light", "light"),
        ("Parla Dark", "dark"),
        ("Parla High Contrast Light", "high-contrast-light"),
        ("Parla High Contrast Dark", "high-contrast-dark"),
    ] {
        let theme = registry.get(name).unwrap();
        let t = |p: &str| rgb(token_hex(&tokens, &format!("theme.{mode}.{p}")));
        assert_eq!(
            parla_tokens(&theme).sidebar_background,
            t("sidebar.bg"),
            "{name} sidebar"
        );
        let tt = parla_tokens(&theme);
        let t = |p: &str| {
            rgb_to_hsla(gpui::rgba(token_rgba(
                &tokens,
                &format!("theme.{mode}.{p}"),
            )))
        };
        for (got, path) in [
            (tt.sidebar_rail, "sidebar.rail"),
            (tt.sidebar_border, "sidebar.border"),
            (tt.sidebar_hover, "sidebar.hover"),
            (tt.sidebar_muted, "sidebar.muted"),
            (tt.sidebar_text_strong, "sidebar.text-strong"),
            (tt.signal, "signal.default"),
            (tt.signal_text, "signal.text"),
            (tt.signal_soft, "signal.soft"),
            (tt.text_subtle, "text.subtle"),
            (tt.mention_text, "mention.text"),
            (tt.accent_soft_text, "accent.soft-text"),
            (tt.border_subtle, "border.subtle"),
            (tt.success_soft, "status.success-soft"),
            (tt.warning_soft, "status.warning-soft"),
            (tt.danger_soft, "status.danger-soft"),
            (tt.border_control, "border.control"),
            (tt.bg_sunken, "bg.sunken"),
            (tt.bg_overlay, "bg.overlay"),
            (tt.code_border, "code.border"),
            (tt.viewer_bg, "viewer.bg"),
            (tt.viewer_stage, "viewer.stage"),
            (tt.viewer_text, "viewer.text"),
            (tt.viewer_muted, "viewer.muted"),
            (tt.viewer_border, "viewer.border"),
            (tt.viewer_control_hover, "viewer.control-hover"),
            (tt.viewer_nav_bg, "viewer.nav-bg"),
            (tt.viewer_focus, "viewer.focus"),
            (tt.code_keyword, "code.keyword"),
            (tt.code_string, "code.string"),
            (tt.code_function, "code.function"),
            (tt.code_comment, "code.comment"),
            (tt.code_number, "code.number"),
            (tt.code_type, "code.type"),
        ] {
            assert_eq!(got, t(path), "{name} {path}");
        }
        assert_eq!(theme.colors().text, t("text.default"), "{name} text");
        assert_eq!(theme.colors().background, t("bg.app"), "{name} background");
        assert_eq!(
            theme.colors().element_background,
            t("accent.default"),
            "{name} element"
        );
        assert_eq!(
            theme.colors().border_focused,
            t("border.focus"),
            "{name} focus ring"
        );
        assert_eq!(
            theme.colors().border_strong,
            t("border.strong"),
            "{name} strong border"
        );
        assert_eq!(
            theme.colors().text_on_accent,
            t("accent.text"),
            "{name} text on accent"
        );
    }
}

#[test]
fn every_builtin_colour_string_parses() {
    fn walk(v: &serde_json::Value, path: &str, bad: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) if s.starts_with('#') => {
                if !matches!(s.len(), 4 | 5 | 7 | 9)
                    || !s[1..].chars().all(|c| c.is_ascii_hexdigit())
                {
                    bad.push(format!("{path}: {s}"));
                }
            }
            serde_json::Value::Object(m) => m
                .iter()
                .for_each(|(k, v)| walk(v, &format!("{path}.{k}"), bad)),
            serde_json::Value::Array(a) => a
                .iter()
                .enumerate()
                .for_each(|(i, v)| walk(v, &format!("{path}[{i}]"), bad)),
            _ => {}
        }
    }
    let mut bad = Vec::new();
    for file in [BUILTIN, HIGH_CONTRAST] {
        let v: serde_json::Value = serde_json::from_str(file).unwrap();
        walk(&v, "", &mut bad);
    }
    assert!(bad.is_empty(), "unparsable colours: {bad:?}");
}

#[gpui::test]
fn list_has_builtins_then_user_themes_light_first_sorted(cx: &mut TestAppContext) {
    let registry = setup(cx);
    let names = |r: &ThemeRegistry| -> Vec<String> {
        r.list().iter().map(|t| t.name.to_string()).collect()
    };
    assert_eq!(
        names(&registry),
        [
            "Parla Light",
            "Parla Dark",
            "Parla High Contrast Light",
            "Parla High Contrast Dark"
        ]
    );
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.json"), SAMPLE).unwrap();
    load_user_themes(&registry, dir.path());
    assert_eq!(
        names(&registry),
        [
            "Parla Light",
            "Parla Dark",
            "Parla High Contrast Light",
            "Parla High Contrast Dark",
            "Sample Light",
            "Sample Dark"
        ]
    );
}

/// A hovered row in a menu or select list (drawn on the raised surface) must
/// look different from the list around it, in every built-in theme.
#[gpui::test]
fn hover_shows_on_the_raised_surface(cx: &mut TestAppContext) {
    let registry = setup(cx);
    for name in [
        "Parla Light",
        "Parla Dark",
        "Parla High Contrast Light",
        "Parla High Contrast Dark",
    ] {
        let c = *registry.get(name).unwrap().colors();
        assert_ne!(
            c.element_hover, c.elevated_surface_background,
            "{name}: hover is invisible on a menu"
        );
        assert_ne!(
            c.element_hover, c.surface_background,
            "{name}: hover is invisible on the surface"
        );
        assert_ne!(
            c.element_hover, c.element_active,
            "{name}: hover equals pressed"
        );
    }
}
