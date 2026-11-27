//! Built-in themes keep the focus ring visible against their backgrounds (WCAG 1.4.11).

use std::path::PathBuf;

use gpui::{Hsla, Rgba};
use serde_json::Value;
use themes::color::{hsla_to_rgba, rgb_to_hsla};

fn themes_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/themes")
}

fn parse_hex(hex: &str) -> Option<Hsla> {
    let digits = hex.strip_prefix('#')?;
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
    Some(rgb_to_hsla(gpui::rgba(rgba)))
}

fn color(style: &serde_json::Map<String, Value>, key: &str) -> Option<Hsla> {
    style.get(key).and_then(Value::as_str).and_then(parse_hex)
}

fn linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: Rgba) -> f32 {
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
    let la = luminance(hsla_to_rgba(a));
    let lb = luminance(hsla_to_rgba(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn focus_ring_contrast_at_least_3_to_1_in_every_builtin_theme() {
    let dir = themes_dir();
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let family: Value = serde_json_lenient::from_str(&text).unwrap();
        let themes = family["themes"].as_array().unwrap();
        for theme in themes {
            let name = theme["name"].as_str().unwrap_or("<unnamed>");
            let style = theme["style"].as_object().unwrap();
            let ring_keys = ["border.focused", "panel.focused_border"];
            let background_keys = [
                "background",
                "editor.background",
                "panel.background",
                "surface.background",
                "title_bar.background",
                "elevated_surface.background",
                "element.background",
            ];
            for rk in ring_keys {
                let Some(ring) = color(style, rk) else {
                    continue;
                };
                for bk in background_keys {
                    let Some(bg) = color(style, bk) else {
                        continue;
                    };
                    let ratio = contrast_ratio(ring, bg);
                    assert!(
                        ratio >= 3.0,
                        "{name} ({path:?}): {rk} vs {bk} contrast {ratio:.2}:1 < 3:1"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 0, "no focus ring tokens checked in {dir:?}");
}

/// The border of a text field or an unticked check box is a UI component
/// boundary: 3:1 on the app, surface and raised grounds (WCAG 1.4.11).
#[test]
fn control_border_contrast_at_least_3_to_1_on_every_ground() {
    let text = std::fs::read_to_string(themes_dir().join("parla.json")).unwrap();
    let family: Value = serde_json_lenient::from_str(&text).unwrap();
    for theme in family["themes"].as_array().unwrap() {
        let name = theme["name"].as_str().unwrap();
        let style = theme["style"].as_object().unwrap();
        let parla = style["parla"].as_object().unwrap();
        let border = color(parla, "border_control").unwrap();
        for bk in [
            "background",
            "surface.background",
            "elevated_surface.background",
        ] {
            let bg = color(style, bk).unwrap();
            let ratio = contrast_ratio(border, bg);
            assert!(
                ratio >= 3.0,
                "{name}: border_control vs {bk} contrast {ratio:.2}:1 < 3:1"
            );
        }
    }
}

/// Banners and filled buttons sit on `element.background` (the accent fill):
/// their text is `text.on_accent`, which reads at 4.5:1 there (WCAG 1.4.3). The
/// plain `text` colour does not (it is dark on the light fill's navy).
#[test]
fn text_on_accent_reads_on_the_accent_fill_in_every_builtin_theme() {
    let mut checked = 0;
    for entry in std::fs::read_dir(themes_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let family: Value =
            serde_json_lenient::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for theme in family["themes"].as_array().unwrap() {
            let name = theme["name"].as_str().unwrap_or("<unnamed>");
            let style = theme["style"].as_object().unwrap();
            let (Some(fill), Some(on)) = (
                color(style, "element.background"),
                color(style, "text.on_accent"),
            ) else {
                continue;
            };
            let ratio = contrast_ratio(on, fill);
            assert!(ratio >= 4.5, "{name}: text.on_accent on fill {ratio:.2}:1");
            checked += 1;
        }
    }
    assert!(checked > 0);
}
