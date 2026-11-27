//! The Parla logo drawn in the active theme's colours. The bundled mark
//! (`assets/brand/logo.svg`) carries one class per colour role; the theme's
//! `logo.*` tokens are written into the matching `fill` and `stroke`
//! attributes by string substitution, and the result is drawn as a coloured
//! image (not the monochrome `svg()` mask). One rendered image is kept and
//! replaced when the tokens change.

use std::sync::Arc;

use gpui::{
    App, Global, Hsla, Image, ImageFormat, InteractiveElement as _, IntoElement, Pixels, Styled,
    img,
};
use themes::{ActiveTheme, LogoTokens, color, parla_tokens};

const MARK: &str = include_str!("../../../assets/brand/logo.svg");

/// Edge length of the SVG handed to the renderer; gpui rasterises at 2x, so the
/// cached bitmap is 256 px (256 KiB) and the mark is drawn at or below that.
const RASTER_PX: u32 = 128;

struct LogoCache {
    tokens: LogoTokens,
    image: Arc<Image>,
}

impl Global for LogoCache {}

fn hex(c: Hsla) -> String {
    let c = color::hsla_to_rgba(c);
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if c.a >= 1.0 {
        format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b), b(c.a))
    }
}

/// Replaces the value of `attr="..."` inside the tag that carries `class="<class>"`.
fn set_attr(svg: &mut String, class: &str, attr: &str, value: &str) {
    let marker = format!("class=\"{class}\"");
    let Some(at) = svg.find(&marker) else { return };
    let end = svg[at..].find('>').map_or(svg.len(), |i| at + i);
    let needle = format!(" {attr}=\"");
    let Some(rel) = svg[at..end].find(&needle) else {
        return;
    };
    let start = at + rel + needle.len();
    let Some(len) = svg[start..end].find('"') else {
        return;
    };
    svg.replace_range(start..start + len, value);
}

/// The logo SVG with `tokens` substituted. A fully transparent rim removes the
/// bubble outline.
pub fn logo_svg(tokens: &LogoTokens) -> String {
    let mut svg = MARK.replacen(
        "width=\"1024\" height=\"1024\"",
        &format!("width=\"{RASTER_PX}\" height=\"{RASTER_PX}\""),
        1,
    );
    set_attr(&mut svg, "bubble", "fill", &hex(tokens.bubble));
    let rim = if tokens.rim.a <= 0.0 {
        "none".to_string()
    } else {
        hex(tokens.rim)
    };
    set_attr(&mut svg, "bubble", "stroke", &rim);
    set_attr(&mut svg, "accent-1", "fill", &hex(tokens.accent_1));
    set_attr(&mut svg, "accent-2", "fill", &hex(tokens.accent_2));
    set_attr(&mut svg, "accent-3", "fill", &hex(tokens.accent_3));
    svg
}

/// The rendered logo for `tokens`: the cached image while the tokens are
/// unchanged, otherwise a new one that replaces the cached entry.
pub fn logo_image(tokens: &LogoTokens, cx: &mut App) -> Arc<Image> {
    if let Some(cache) = cx.try_global::<LogoCache>()
        && cache.tokens == *tokens
    {
        return cache.image.clone();
    }
    let image = Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        logo_svg(tokens).into_bytes(),
    ));
    cx.set_global(LogoCache {
        tokens: *tokens,
        image: image.clone(),
    });
    image
}

/// The logo at `size` (a square) in the active theme's colours.
pub fn logo(size: Pixels, cx: &mut App) -> impl IntoElement {
    let tokens = parla_tokens(cx.theme()).logo;
    img(logo_image(&tokens, cx))
        .debug_selector(|| "logo".into())
        .size(size)
}
