//! Conversions between the colour models used by themes.

/// Converts an RGBA colour to HSLA using GPUI's `From` conversion.
pub fn rgb_to_hsla(c: gpui::Rgba) -> gpui::Hsla {
    c.into()
}

/// Converts an HSLA colour to RGBA using GPUI's `From` conversion.
pub fn hsla_to_rgba(c: gpui::Hsla) -> gpui::Rgba {
    c.into()
}

/// `colour` at `strength` composited over `ground`, opaque.
///
/// This is how group opacity (`div().opacity(x)` on the old GPUI) drew a
/// dimmed element: as one image over what was behind it. GPUI Fast multiplies
/// opacity into every paint instead, so a fill drawn with alpha lets a ring
/// behind it show through and a label drawn with alpha over its own faded fill
/// fades twice. Dimming a colour with `faded` keeps main's look and overlaps
/// nothing.
pub fn faded(colour: gpui::Hsla, ground: gpui::Hsla, strength: f32) -> gpui::Hsla {
    let fg = hsla_to_rgba(colour);
    let bg = hsla_to_rgba(ground);
    let a = fg.a * strength;
    let mix = |f: f32, g: f32| f * a + g * (1.0 - a);
    gpui::Rgba {
        r: mix(fg.r, bg.r),
        g: mix(fg.g, bg.g),
        b: mix(fg.b, bg.b),
        a: 1.0,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::{hsla_to_rgba, rgb_to_hsla};

    fn assert_round_trip(c: gpui::Rgba) {
        let back = hsla_to_rgba(rgb_to_hsla(c));
        let tolerance = 1.0 / 255.0;
        for (name, got, want) in [
            ("r", back.r, c.r),
            ("g", back.g, c.g),
            ("b", back.b, c.b),
            ("a", back.a, c.a),
        ] {
            assert!(
                (got - want).abs() <= tolerance,
                "{name}: {got} differs from {want} by more than {tolerance}"
            );
        }
    }

    #[test]
    fn hex_colours_survive_an_hsla_round_trip() {
        for hex in [0x16151f, 0xffa611, 0xffffff, 0x000000] {
            assert_round_trip(gpui::rgb(hex));
        }
    }

    #[test]
    fn a_hex_colour_with_alpha_survives_an_hsla_round_trip() {
        assert_round_trip(gpui::rgba(0x11223380));
    }

    #[test]
    fn faded_composites_towards_the_ground() {
        let white = super::rgb_to_hsla(gpui::rgb(0xffffff));
        let black = super::rgb_to_hsla(gpui::rgb(0x000000));
        let half = super::faded(white, black, 0.5);
        assert!(
            (half.l - 0.5).abs() < 0.01,
            "white at half over black: {half:?}"
        );
        assert!((half.a - 1.0).abs() < 0.001, "opaque: {half:?}");
        let want = super::rgb_to_hsla(gpui::rgb(0xffa611));
        let ground = super::rgb_to_hsla(gpui::rgb(0x211e4c));
        let full = super::faded(want, ground, 1.0);
        assert!((full.h - want.h).abs() < 0.001, "{full:?} vs {want:?}");
        assert!((full.s - want.s).abs() < 0.001, "{full:?} vs {want:?}");
        assert!((full.l - want.l).abs() < 0.001, "{full:?} vs {want:?}");
    }
}
