// Ported from gpui-ce (Apache-2.0),
// crates/gpui_render/src/swash_glyph_rasterizer.rs. See ../../NOTICE.
// Adapted to gpui-pre's two-call raster API (bounds, then pixels).

use anyhow::{Context as _, Result, bail};
use gpui::{Bounds, DevicePixels, FontId, GlyphId, RenderGlyphParams, Size, point, size};
use std::collections::HashMap;
use swash::{
    CacheKey as SwashCacheKey, FontRef as SwashFontRef,
    scale::{
        Render, ScaleContext, Source, StrikeWith,
        image::{Content, Image},
    },
    zeno::{Angle, Format, Transform, Vector},
};

use super::font_store::LoadedFont;

/// Swash raster state: one scaling context and one cache key per font.
#[derive(Default)]
pub(crate) struct SwashRasterizer {
    scale_context: ScaleContext,
    cache_keys: HashMap<FontId, SwashCacheKey>,
}

impl SwashRasterizer {
    /// Renders the glyph image, honouring the subpixel variant and synthetic
    /// styling. `None` means the face has no bitmap or outline for it.
    fn render_glyph_image(
        &mut self,
        font_id: FontId,
        font: &LoadedFont,
        params: &RenderGlyphParams,
    ) -> Result<Option<Image>> {
        let pixel_size = f32::from(params.font_size) * params.scale_factor;

        if !pixel_size.is_finite() || pixel_size <= 0.0 {
            bail!("glyph rasterization got an invalid pixel size");
        }

        let cache_key = *self.cache_keys.entry(font_id).or_default();

        let mut font_ref = SwashFontRef::from_index(font.data.as_ref(), font.index as usize)
            .context("Swash could not parse the stored font face")?;

        font_ref.key = cache_key;
        let subpixel_offset = subpixel_offset(params);

        let mut scaler = self
            .scale_context
            .builder(font_ref)
            .size(pixel_size)
            .hint(true)
            .build();

        let sources: &[Source] = if params.is_emoji {
            &[
                Source::ColorOutline(0),
                Source::ColorBitmap(StrikeWith::BestFit),
                Source::Outline,
            ]
        } else {
            &[Source::ColorBitmap(StrikeWith::BestFit), Source::Outline]
        };

        let mut renderer = Render::new(sources);
        renderer.format(Format::Alpha);
        renderer.offset(subpixel_offset);

        if font.synthesis.embolden() {
            renderer.embolden(pixel_size / 48.0);
        }

        if let Some(degrees) = font.synthesis.skew() {
            renderer.transform(Some(Transform::skew(
                Angle::from_degrees(degrees),
                Angle::ZERO,
            )));
        }

        Ok(renderer.render(&mut scaler, glyph_id(params.glyph_id)?))
    }

    /// Raster bounds in device pixels, relative to the pen origin on the
    /// baseline, with y growing down (as GPUI paints them).
    pub(crate) fn raster_bounds(
        &mut self,
        font_id: FontId,
        font: &LoadedFont,
        params: &RenderGlyphParams,
    ) -> Result<Bounds<DevicePixels>> {
        let Some(image) = self.render_glyph_image(font_id, font, params)? else {
            return Ok(Bounds::default());
        };

        Ok(Bounds {
            origin: point(
                DevicePixels(image.placement.left),
                DevicePixels(-image.placement.top),
            ),
            size: size(
                DevicePixels(image.placement.width as i32),
                DevicePixels(image.placement.height as i32),
            ),
        })
    }

    /// Rasterizes the glyph into the pixels GPUI stores in its atlas.
    pub(crate) fn rasterize(
        &mut self,
        font_id: FontId,
        font: &LoadedFont,
        params: &RenderGlyphParams,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        let font_size = f32::from(params.font_size);

        if !font_size.is_finite() || font_size < 0.0 {
            bail!("glyph rasterization got an invalid font size");
        }

        if !params.scale_factor.is_finite() || params.scale_factor <= 0.0 {
            bail!("glyph rasterization got an invalid scale factor");
        }

        if font_size == 0.0 {
            return Ok((Size::default(), Vec::new()));
        }

        let Some(mut image) = self.render_glyph_image(font_id, font, params)? else {
            bail!("unable to rasterize glyph {:?}", params.glyph_id);
        };

        if image.placement.width == 0 || image.placement.height == 0 {
            return Ok((Size::default(), Vec::new()));
        }

        let bitmap_size = size(
            DevicePixels(image.placement.width as i32),
            DevicePixels(image.placement.height as i32),
        );

        let pixels = match image.content {
            Content::Color => {
                let premultiplied = matches!(image.source, Source::ColorOutline(_));
                for pixel in image.data.as_chunks_mut::<4>().0 {
                    if premultiplied {
                        swap_rgba_pa_to_bgra(pixel);
                    } else {
                        pixel.swap(0, 2);
                    }
                }

                image.data
            }
            Content::SubpixelMask => {
                for pixel in image.data.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }

                image.data
            }
            Content::Mask => image.data,
        };

        Ok((bitmap_size, pixels))
    }
}

fn subpixel_offset(params: &RenderGlyphParams) -> Vector {
    Vector::new(
        params.subpixel_variant.x as f32 / gpui::SUBPIXEL_VARIANTS_X as f32 / params.scale_factor,
        params.subpixel_variant.y as f32 / gpui::SUBPIXEL_VARIANTS_Y as f32 / params.scale_factor,
    )
}

fn glyph_id(glyph_id: GlyphId) -> Result<u16> {
    glyph_id
        .0
        .try_into()
        .with_context(|| format!("glyph ID {} is larger than u16", glyph_id.0))
}

/// Swash returns premultiplied RGBA for color outlines; GPUI's atlas wants
/// BGRA.
fn swap_rgba_pa_to_bgra(color: &mut [u8]) {
    color.swap(0, 2);
    if color[3] > 0 {
        let a = color[3] as f32 / 255.;
        color[0] = (color[0] as f32 / a) as u8;
        color[1] = (color[1] as f32 / a) as u8;
        color[2] = (color[2] as f32 / a) as u8;
    }
}
