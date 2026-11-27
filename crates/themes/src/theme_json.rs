//! Reads theme-family JSON: `{ "name", "author", "themes": [ { "name", "appearance",
//! "style": { ... } } ] }`. Colour keys in `style` are dotted (`element.selected`),
//! values are `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` strings. Comments and trailing
//! commas are allowed. Unknown keys are ignored and unparsable colours are dropped, so
//! files written for other editors load. The optional `parla` object holds chat tokens.

use gpui::Hsla;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::color::{hsla_to_rgba, rgb_to_hsla};
use crate::model::{Appearance, ParlaOverrides, StatusColors, Theme, ThemeColors};

#[derive(Deserialize)]
struct Family {
    #[serde(default)]
    themes: Vec<RawTheme>,
}

#[derive(Deserialize)]
struct RawTheme {
    name: String,
    appearance: RawAppearance,
    #[serde(default)]
    style: Map<String, Value>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum RawAppearance {
    Light,
    Dark,
}

/// Parses a colour string, `None` when it is not a hex colour.
pub(crate) fn parse_color(text: &str) -> Option<Hsla> {
    let digits = text.strip_prefix('#')?;
    if !digits.is_ascii() {
        return None;
    }
    let value = |s: &str| u32::from_str_radix(s, 16).ok();
    let rgba = match digits.len() {
        3 | 4 => {
            let mut v = 0u32;
            for ch in digits.chars() {
                let d = ch.to_digit(16)?;
                v = (v << 8) | (d * 17);
            }
            if digits.len() == 3 {
                (v << 8) | 0xff
            } else {
                v
            }
        }
        6 => (value(digits)? << 8) | 0xff,
        8 => value(digits)?,
        _ => return None,
    };
    Some(hex(rgba))
}

/// Parses a theme-family file into themes. Fails only when the file is not valid JSON
/// or a theme lacks its `name` or `appearance`.
pub(crate) fn parse_family(bytes: &[u8]) -> anyhow::Result<Vec<Theme>> {
    let family: Family = serde_json_lenient::from_slice(bytes)?;
    Ok(family.themes.into_iter().map(build_theme).collect())
}

fn color(style: &Map<String, Value>, key: &str) -> Option<Hsla> {
    style.get(key)?.as_str().and_then(parse_color)
}

/// The colour of syntax class `name` in the theme's `syntax` block.
fn syntax(style: &Map<String, Value>, name: &str) -> Option<Hsla> {
    style
        .get("syntax")?
        .get(name)?
        .get("color")?
        .as_str()
        .and_then(parse_color)
}

fn build_theme(raw: RawTheme) -> Theme {
    let appearance = match raw.appearance {
        RawAppearance::Light => Appearance::Light,
        RawAppearance::Dark => Appearance::Dark,
    };
    let base = defaults(appearance);
    let s = &raw.style;
    let c = |key: &str, default: Hsla| color(s, key).unwrap_or(default);
    let colors = ThemeColors {
        background: c("background", base.0.background),
        surface_background: c("surface.background", base.0.surface_background),
        elevated_surface_background: c(
            "elevated_surface.background",
            base.0.elevated_surface_background,
        ),
        panel_background: c("panel.background", base.0.panel_background),
        title_bar_background: c("title_bar.background", base.0.title_bar_background),
        editor_background: c("editor.background", base.0.editor_background),
        border: c("border", base.0.border),
        border_focused: c("border.focused", base.0.border_focused),
        border_strong: c("border.strong", base.0.border_strong),
        text_on_accent: color(s, "text.on_accent").unwrap_or_else(|| {
            let fill = c("element.background", base.0.element_background);
            readable_on(
                fill,
                c("text", base.0.text),
                c("background", base.0.background),
            )
        }),
        accent_soft: c("accent.soft", base.0.accent_soft),
        element_hover: c("element.hover", base.0.element_hover),
        element_background: c("element.background", base.0.element_background),
        element_selected: c("element.selected", base.0.element_selected),
        element_active: c("element.active", base.0.element_active),
        text: c("text", base.0.text),
        text_muted: c("text.muted", base.0.text_muted),
        text_accent: c("text.accent", base.0.text_accent),
        editor_document_highlight_read_background: c(
            "editor.document_highlight.read_background",
            base.0.editor_document_highlight_read_background,
        ),
    };
    let status = StatusColors {
        success: c("success", base.1.success),
        warning: c("warning", base.1.warning),
        error: c("error", base.1.error),
    };
    let parla = raw
        .style
        .get("parla")
        .and_then(Value::as_object)
        .map(|t| ParlaOverrides {
            sidebar_background: color(t, "sidebar_background"),
            sidebar_text: color(t, "sidebar_text"),
            sidebar_active: color(t, "sidebar_active"),
            message_mention_background: color(t, "message_mention_background"),
            presence_active: color(t, "presence_active"),
            presence_away: color(t, "presence_away"),
            avatar_text: color(t, "avatar_text"),
            danger_text: color(t, "danger_text"),
            warning_text: color(t, "warning_text"),
            sidebar_rail: color(t, "sidebar_rail"),
            sidebar_border: color(t, "sidebar_border"),
            sidebar_hover: color(t, "sidebar_hover"),
            sidebar_muted: color(t, "sidebar_muted"),
            sidebar_text_strong: color(t, "sidebar_text_strong"),
            signal: color(t, "signal"),
            signal_text: color(t, "signal_text"),
            signal_soft: color(t, "signal_soft"),
            text_subtle: color(t, "text_subtle"),
            mention_text: color(t, "mention_text"),
            accent_soft_text: color(t, "accent_soft_text"),
            border_subtle: color(t, "border_subtle"),
            success_soft: color(t, "success_soft"),
            warning_soft: color(t, "warning_soft"),
            danger_soft: color(t, "danger_soft"),
            border_control: color(t, "border_control"),
            bg_sunken: color(t, "bg_sunken"),
            bg_overlay: color(t, "bg_overlay"),
            code_border: color(t, "code_border"),
            viewer_bg: color(t, "viewer_bg"),
            viewer_stage: color(t, "viewer_stage"),
            viewer_text: color(t, "viewer_text"),
            viewer_muted: color(t, "viewer_muted"),
            viewer_border: color(t, "viewer_border"),
            viewer_control_hover: color(t, "viewer_control_hover"),
            viewer_nav_bg: color(t, "viewer_nav_bg"),
            viewer_focus: color(t, "viewer_focus"),
            code_keyword: syntax(&raw.style, "keyword"),
            code_string: syntax(&raw.style, "string"),
            code_function: syntax(&raw.style, "function"),
            code_comment: syntax(&raw.style, "comment"),
            code_number: syntax(&raw.style, "number"),
            code_type: syntax(&raw.style, "type"),
            logo_background: color(t, "logo.background"),
            logo_bubble: color(t, "logo.bubble"),
            logo_rim: color(t, "logo.rim"),
            logo_accent_1: color(t, "logo.accent_1"),
            logo_accent_2: color(t, "logo.accent_2"),
            logo_accent_3: color(t, "logo.accent_3"),
        })
        .unwrap_or_default();
    Theme::new(raw.name.into(), appearance, colors, status, parla)
}

/// WCAG relative luminance of `c` (alpha ignored).
fn luminance(c: Hsla) -> f32 {
    let c = hsla_to_rgba(c);
    let lin = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio between two colours.
pub(crate) fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Whichever of `a` and `b` has the higher contrast ratio against `fill`.
pub(crate) fn readable_on(fill: Hsla, a: Hsla, b: Hsla) -> Hsla {
    if contrast(fill, a) >= contrast(fill, b) {
        a
    } else {
        b
    }
}

fn hex(rgba: u32) -> Hsla {
    rgb_to_hsla(gpui::rgba(rgba))
}

/// Colours a theme file falls back to for keys it does not set.
fn defaults(appearance: Appearance) -> (ThemeColors, StatusColors) {
    match appearance {
        Appearance::Light => (
            ThemeColors {
                background: hex(0xf7f4ecff),
                surface_background: hex(0xefebe0ff),
                elevated_surface_background: hex(0xfffdf8ff),
                panel_background: hex(0xece7dbff),
                title_bar_background: hex(0xece7dbff),
                editor_background: hex(0xf7f4ecff),
                border: hex(0xd3ccbcff),
                border_focused: hex(0x1d7a74ff),
                border_strong: hex(0xb9b2a0ff),
                text_on_accent: hex(0xffffffff),
                accent_soft: hex(0xc6dedcff),
                element_hover: hex(0xece7dbff),
                element_background: hex(0xe6e0d2ff),
                element_selected: hex(0xc6dedcff),
                element_active: hex(0xdcd5c4ff),
                text: hex(0x27231dff),
                text_muted: hex(0x6b6558ff),
                text_accent: hex(0x1d7a74ff),
                editor_document_highlight_read_background: hex(0xf6e3a066),
            },
            StatusColors {
                success: hex(0x2e8b57ff),
                warning: hex(0xb7791fff),
                error: hex(0xc0392bff),
            },
        ),
        Appearance::Dark => (
            ThemeColors {
                background: hex(0x1d1a17ff),
                surface_background: hex(0x24201cff),
                elevated_surface_background: hex(0x2b2621ff),
                panel_background: hex(0x161412ff),
                title_bar_background: hex(0x161412ff),
                editor_background: hex(0x1d1a17ff),
                border: hex(0x3a342dff),
                border_focused: hex(0xf08a4bff),
                border_strong: hex(0x4f483fff),
                text_on_accent: hex(0x000000ff),
                accent_soft: hex(0x5a3a26ff),
                element_hover: hex(0x2b2621ff),
                element_background: hex(0x2e2924ff),
                element_selected: hex(0x5a3a26ff),
                element_active: hex(0x3a342dff),
                text: hex(0xece6dbff),
                text_muted: hex(0xa39b8cff),
                text_accent: hex(0xf08a4bff),
                editor_document_highlight_read_background: hex(0xf08a4b2e),
            },
            StatusColors {
                success: hex(0x6fc08aff),
                warning: hex(0xe0b04aff),
                error: hex(0xe5655aff),
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(style: &str, appearance: &str) -> Theme {
        let json = format!(
            r#"{{ "name": "F", "themes": [
                {{ "name": "T", "appearance": "{appearance}", "style": {style} }} ] }}"#
        );
        parse_family(json.as_bytes()).unwrap().remove(0)
    }

    #[test]
    fn element_hover_is_read_and_falls_back() {
        let t = one(r##"{ "element.hover": "#112233" }"##, "light");
        assert_eq!(t.colors().element_hover, hex(0x112233ff));
        let t = one("{}", "light");
        assert_eq!(t.colors().element_hover, hex(0xece7dbff));
    }

    #[test]
    fn dotted_keys_map_to_fields() {
        let t = one(
            r##"{ "element.selected": "#ff0000", "text.muted": "#00ff0080", "error": "#0000ff" }"##,
            "dark",
        );
        assert_eq!(t.colors().element_selected, hex(0xff0000ff));
        assert_eq!(t.colors().text_muted, hex(0x00ff0080));
        assert_eq!(t.status().error, hex(0x0000ffff));
    }

    #[test]
    fn text_on_a_yellow_accent_in_a_dark_theme_is_dark() {
        // Dark theme: light text, dark background; a yellow fill needs the dark one.
        let t = one(
            r##"{ "element.background": "#ffd400", "text": "#ece6db", "background": "#121215" }"##,
            "dark",
        );
        assert_eq!(t.colors().text_on_accent, hex(0x121215ff));
        let set = one(r##"{ "text.on_accent": "#112233" }"##, "dark");
        assert_eq!(set.colors().text_on_accent, hex(0x112233ff));
    }

    #[test]
    fn focus_ring_reads_border_focused() {
        let t = one(r##"{ "border.focused": "#ff0000" }"##, "dark");
        assert_eq!(t.colors().border_focused, hex(0xff0000ff));
        let d = one("{}", "dark");
        assert_eq!(d.colors().border_focused, hex(0xf08a4bff));
    }

    #[test]
    fn short_hex_forms_parse() {
        assert_eq!(parse_color("#f00"), Some(hex(0xff0000ff)));
        assert_eq!(parse_color("#f008"), Some(hex(0xff000088)));
        assert_eq!(parse_color("#ff0000"), Some(hex(0xff0000ff)));
        assert_eq!(parse_color("ff0000"), None);
        assert_eq!(parse_color("#ff00"), Some(hex(0xffff0000)));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#gggggg"), None);
    }

    #[test]
    fn bad_colours_and_unknown_keys_are_dropped() {
        let t = one(
            r##"{ "border": "nope", "text": 5, "players": [1], "syntax": {"a": 1}, "x.y": "#fff" }"##,
            "light",
        );
        assert_eq!(t.colors().border, defaults(Appearance::Light).0.border);
        assert_eq!(t.colors().text, defaults(Appearance::Light).0.text);
    }

    #[test]
    fn comments_and_trailing_commas_parse() {
        let t = one(
            r##"{ // c
            "background": "#102030", }"##,
            "dark",
        );
        assert_eq!(t.colors().background, hex(0x102030ff));
    }

    #[test]
    fn parla_object_is_kept() {
        let t = one(
            r##"{ "parla": { "sidebar_text": "#010203", "presence_away": "bad" } }"##,
            "dark",
        );
        // The good token is kept; the bad one falls back to the derived value.
        assert_eq!(t.tokens().sidebar_text, hex(0x010203ff));
        let plain = one("{}", "dark");
        assert_eq!(t.tokens().presence_away, plain.tokens().presence_away);
    }
}
