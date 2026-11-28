//! One text pipeline on every OS: shaping must not depend on the platform's
//! native text system.

use std::borrow::Cow;

use gpui::{FontRun, FontStyle, FontWeight, PlatformTextSystem, font, px};
use ui_testing::{
    INTER_BOLD, INTER_ITALIC, INTER_MEDIUM, INTER_REGULAR, INTER_SEMIBOLD, JETBRAINS_MONO_REGULAR,
    NOTO_EMOJI_SUBSET, ParleyTextSystem,
};

const TEXT: &str = "Hello, Parla 123";
const FONT_SIZE: f32 = 14.0;

fn system() -> ParleyTextSystem {
    let system = ParleyTextSystem::new();
    system
        .add_fonts(vec![
            Cow::Borrowed(INTER_REGULAR),
            Cow::Borrowed(INTER_ITALIC),
            Cow::Borrowed(INTER_MEDIUM),
            Cow::Borrowed(INTER_SEMIBOLD),
            Cow::Borrowed(INTER_BOLD),
            Cow::Borrowed(JETBRAINS_MONO_REGULAR),
            Cow::Borrowed(NOTO_EMOJI_SUBSET),
        ])
        .expect("bundled fonts must load");
    system
}

fn round(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

#[test]
fn inter_shapes_a_known_string_identically_everywhere() {
    let system = system();
    let inter = system.font_id(&font("Inter")).expect("Inter resolves");
    assert!(require_family_names(&system, "Inter"));

    let layout = system.layout_line(
        TEXT,
        px(FONT_SIZE),
        &[FontRun {
            len: TEXT.len(),
            font_id: inter,
        }],
    );

    let glyphs = layout
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .collect::<Vec<_>>();
    assert_eq!(glyphs.len(), 16, "glyphs: {glyphs:?}");

    // No cfg branches: these numbers come from the bundled font bytes, not
    // from CoreText, DirectWrite or cosmic-text.
    let x = glyphs
        .iter()
        .take(5)
        .map(|glyph| round(f32::from(glyph.position.x)))
        .collect::<Vec<_>>();
    assert_eq!(x, vec![0.0, 10.4, 18.57, 21.96, 25.35]);
    assert_eq!(round(f32::from(layout.width)), 102.07);

    assert_eq!(glyphs.first().unwrap().index, 0);
    assert_eq!(glyphs.last().unwrap().index, TEXT.len() - 1);
}

#[test]
fn a_missing_family_falls_back_to_inter() {
    let system = system();
    let inter = system.font_id(&font("Inter")).expect("Inter resolves");
    let missing = system
        .font_id(&font("No Such Family In The Harness"))
        .expect("a missing family falls back");

    for ch in ['H', 'e', 'l', 'o'] {
        let inter_glyph = system.glyph_for_char(inter, ch).unwrap();
        let missing_glyph = system.glyph_for_char(missing, ch).unwrap();
        assert_eq!(missing_glyph, inter_glyph, "{ch}");
        assert_eq!(
            system.advance(missing, missing_glyph).unwrap(),
            system.advance(inter, inter_glyph).unwrap(),
            "{ch}"
        );
    }
}

/// `*italic*` must shape with the italic face, not the upright one: the
/// run's style survives font resolution and shaping.
#[test]
fn an_italic_run_shapes_with_the_italic_face() {
    let system = system();
    let upright = system.font_id(&font("Inter")).expect("Inter resolves");
    let mut italic_font = font("Inter");
    italic_font.style = FontStyle::Italic;
    let italic = system.font_id(&italic_font).expect("Inter Italic resolves");
    assert_ne!(upright, italic, "italic is a different face");

    let shape = |font_id| {
        system.layout_line(
            "italic",
            px(FONT_SIZE),
            &[FontRun {
                len: "italic".len(),
                font_id,
            }],
        )
    };
    let upright_layout = shape(upright);
    let italic_layout = shape(italic);
    assert_eq!(
        italic_layout.runs[0].font_id, italic,
        "the shaped run keeps the italic face"
    );
    assert_ne!(
        upright_layout.runs[0].glyphs[1].id, italic_layout.runs[0].glyphs[1].id,
        "italic glyphs differ from upright glyphs"
    );
}

/// The design uses Inter at 400/500/600/700, so a weighted run must shape with
/// the bundled face for that weight (its own advances), not the regular face.
#[test]
fn a_semibold_run_shapes_with_the_semibold_face() {
    let system = system();
    let regular = system.font_id(&font("Inter")).expect("Inter resolves");
    let mut semibold_font = font("Inter");
    semibold_font.weight = FontWeight::SEMIBOLD;
    let semibold = system
        .font_id(&semibold_font)
        .expect("Inter SemiBold resolves");
    assert_ne!(regular, semibold, "semibold is its own face");

    let shape = |font_id| {
        system.layout_line(
            TEXT,
            px(FONT_SIZE),
            &[FontRun {
                len: TEXT.len(),
                font_id,
            }],
        )
    };
    let regular_layout = shape(regular);
    let semibold_layout = shape(semibold);
    assert_eq!(
        semibold_layout.runs[0].font_id, semibold,
        "the shaped run keeps the semibold face"
    );
    assert_ne!(
        round(f32::from(regular_layout.width)),
        round(f32::from(semibold_layout.width)),
        "semibold advances differ from regular"
    );
}

fn require_family_names(system: &ParleyTextSystem, family: &str) -> bool {
    system.all_font_names().iter().any(|name| name == family)
}
