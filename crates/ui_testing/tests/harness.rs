//! the deterministic offscreen harness.
#![cfg(not(target_os = "windows"))]

use gpui::{AppContext as _, Context, IntoElement, ParentElement as _, Styled as _, div, px};
use themes::ActiveTheme as _;
use ui_testing::{Appearance, INTER_REGULAR, LOGICAL_H, LOGICAL_W, UiHarness, Variant};

struct Blank;
impl gpui::Render for Blank {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = cx.theme().colors().background;
        div().size_full().bg(bg).child(div().w(px(10.0)))
    }
}

fn pixel_matches(img: &image::RgbaImage, c: gpui::Hsla) -> bool {
    let rgba = themes::color::hsla_to_rgba(c);
    let want = [rgba.r, rgba.g, rgba.b].map(|v| (v * 255.0).round() as i32);
    let got = img.get_pixel(img.width() / 2, img.height() / 2).0;
    (0..3).all(|i| (got[i] as i32 - want[i]).abs() <= 2)
}

#[test]
fn fonts_are_bundled_ones() {
    let h = UiHarness::new(Variant {
        appearance: Appearance::Light,
        scale: 2.0,
    })
    .unwrap();
    // Not a system font: the bundled bytes are what resolved the families.
    assert!(!INTER_REGULAR.is_empty());
    assert!(h.require_family("Inter").is_ok());
    assert!(h.require_family("JetBrains Mono").is_ok());
    assert!(h.require_family("NoSuchFont").is_err());
}

#[test]
fn variant_sets_theme_and_scale() {
    for scale in [1.0_f32, 2.0] {
        let mut h = UiHarness::new(Variant {
            appearance: Appearance::Dark,
            scale,
        })
        .unwrap();
        let w = h.open(|_, app| app.new(|_| Blank));
        let img = h.capture(w.into()).unwrap();
        assert_eq!(
            (img.width(), img.height()),
            ((LOGICAL_W * scale) as u32, (LOGICAL_H * scale) as u32)
        );
        let bg = h.app().update(|app| app.theme().colors().background);
        assert!(pixel_matches(&img, bg), "dark background at {scale}x");
        assert_eq!(
            h.app().update(|app| app.theme().appearance),
            themes::Appearance::Dark
        );
    }
}

#[test]
fn system_appearance_is_ignored() {
    // The test platform reports a fixed system appearance; both explicit
    // variants must still get their own theme.
    for (a, want) in [
        (Appearance::Light, themes::Appearance::Light),
        (Appearance::Dark, themes::Appearance::Dark),
    ] {
        let mut h = UiHarness::new(Variant {
            appearance: a,
            scale: 2.0,
        })
        .unwrap();
        assert_eq!(h.app().update(|app| app.theme().appearance), want);
    }
}

struct Hello;
impl gpui::Render for Hello {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        // No font family: the default `.SystemUIFont` must resolve to a bundled face.
        div()
            .size_full()
            .bg(gpui::rgb(0xffffff))
            .text_color(gpui::rgb(0))
            .child("Hello world")
    }
}

#[test]
fn default_font_text_renders() {
    let mut h = UiHarness::new(Variant {
        appearance: Appearance::Light,
        scale: 1.0,
    })
    .unwrap();
    let w = h.open(|_, app| app.new(|_| Hello));
    let img = h.capture(w.into()).unwrap();
    let dark = img.pixels().filter(|p| p.0[0] < 128).count();
    assert!(dark > 50, "text pixels drawn: {dark}");
}

/// "old italic" with the highlights a message's `~old~` and `_italic_` become
/// (components `rich_text::highlight`), or none.
struct Styled(bool);
impl gpui::Render for Styled {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        let text = gpui::StyledText::new("old italic");
        let text = if self.0 {
            text.with_highlights([
                (
                    0..3,
                    gpui::HighlightStyle {
                        strikethrough: Some(gpui::StrikethroughStyle {
                            thickness: px(1.0),
                            color: None,
                        }),
                        ..Default::default()
                    },
                ),
                (
                    4..10,
                    gpui::HighlightStyle {
                        font_style: Some(gpui::FontStyle::Italic),
                        ..Default::default()
                    },
                ),
            ])
        } else {
            text
        };
        div()
            .size_full()
            .bg(gpui::rgb(0xffffff))
            .text_color(gpui::rgb(0))
            .text_size(px(40.0))
            .font_family("Inter")
            .child(text)
    }
}

fn styled_image(styled: bool) -> image::RgbaImage {
    let mut h = UiHarness::new(Variant {
        appearance: Appearance::Light,
        scale: 1.0,
    })
    .unwrap();
    let w = h.open(move |_, app| app.new(|_| Styled(styled)));
    h.capture(w.into()).unwrap()
}

/// The longest run of dark pixels along any single row: a strike line makes
/// one a word wide, letters alone never do.
fn longest_dark_run(img: &image::RgbaImage) -> u32 {
    (0..img.height())
        .map(|y| {
            let (mut best, mut run) = (0, 0);
            for x in 0..img.width() {
                if img.get_pixel(x, y).0[0] < 128 {
                    run += 1;
                    best = best.max(run);
                } else {
                    run = 0;
                }
            }
            best
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn strikethrough_highlights_draw_a_line() {
    let plain = longest_dark_run(&styled_image(false));
    let struck = longest_dark_run(&styled_image(true));
    assert!(plain < 20, "letters alone: {plain}");
    assert!(struck >= 40, "strike line over `old`: {struck}");
}

#[test]
fn italic_highlights_draw_the_italic_face() {
    let (plain, italic) = (styled_image(false), styled_image(true));
    // Right of "old" (its strike line aside) is "italic": slanted glyphs
    // differ from upright ones.
    let differing = (0..plain.height())
        .flat_map(|y| (110..plain.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| plain.get_pixel(x, y) != italic.get_pixel(x, y))
        .count();
    assert!(
        differing > 200,
        "italic pixels differ from upright: {differing}"
    );
}
