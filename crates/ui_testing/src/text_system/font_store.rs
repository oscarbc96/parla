// Ported from gpui-ce (Apache-2.0), crates/gpui_ce_parley/src/store.rs.
// See ../../NOTICE. Adapted to gpui-pre's PlatformTextSystem trait and to
// static bundled fonts (no variable instances beyond fontique synthesis).

use anyhow::{Context as _, Result};
use fontique::{Blob, Synthesis};
use gpui::{Bounds, FontId, FontMetrics, GlyphId, Size, point, size};
use parley::FontStyle as ParleyFontStyle;
use skrifa::{
    FontRef, MetadataProvider as _, Tag,
    instance::{LocationRef, Size as SkrifaSize},
};
use std::collections::HashMap;

/// Full identity of a selected font instance.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FontKey {
    source_id: u64,
    face_index: u32,
    embolden: bool,
    skew_bits: Option<u32>,
    /// 0 normal, 1 italic, 2 oblique: the same face asked for in another
    /// style shapes with that style.
    style: u8,
}

/// A concrete font face selected for GPUI, plus the style the shaping run
/// asked for. Glyph runs shaped from it map back to this entry by face.
#[derive(Clone, Debug)]
pub(crate) struct LoadedFont {
    /// Immutable bytes containing the selected face.
    pub(crate) data: Blob<u8>,
    /// Face index within the font collection.
    pub(crate) index: u32,
    /// Synthetic styling requested for the selected face.
    pub(crate) synthesis: Synthesis,
    /// Family names to shape with: requested families, then the harness
    /// fallbacks.
    pub(crate) families: Vec<String>,
    /// Requested weight, used as a shaping attribute.
    pub(crate) weight: f32,
    /// Requested style, used as a shaping attribute so italic runs keep
    /// selecting the italic face.
    pub(crate) style: ParleyFontStyle,
    /// Whether this face advertises color glyph data.
    pub(crate) has_color_glyphs: bool,
}

impl LoadedFont {
    fn skrifa_ref(&self) -> Result<FontRef<'_>> {
        FontRef::from_index(self.data.as_ref(), self.index)
            .context("Skrifa could not parse the stored font face")
    }

    /// Reads global metrics in font units from the face.
    pub(crate) fn metrics(&self) -> Result<FontMetrics> {
        let font = self.skrifa_ref()?;
        let metrics = font.metrics(SkrifaSize::unscaled(), LocationRef::new(&[]));

        Ok(FontMetrics {
            units_per_em: metrics.units_per_em.into(),
            ascent: metrics.ascent,
            // GPUI documents descent as a positive distance below the
            // baseline; skrifa reports it negative.
            descent: -metrics.descent,
            line_gap: metrics.leading,
            underline_position: metrics.underline.map_or(0.0, |underline| underline.offset),
            underline_thickness: metrics
                .underline
                .map_or(0.0, |underline| underline.thickness),
            cap_height: metrics.cap_height.unwrap_or(metrics.ascent),
            x_height: metrics.x_height.unwrap_or(metrics.ascent),
            bounding_box: Bounds {
                origin: point(0.0, 0.0),
                size: size(
                    metrics.max_width.unwrap_or(0.0),
                    metrics.ascent - metrics.descent,
                ),
            },
        })
    }

    /// Maps a Unicode scalar to a nominal glyph.
    pub(crate) fn glyph_for_char(&self, character: char) -> Result<Option<GlyphId>> {
        Ok(self
            .skrifa_ref()?
            .charmap()
            .map(character)
            .map(|glyph| GlyphId(glyph.to_u32())))
    }

    /// Returns the unscaled advance for a glyph.
    pub(crate) fn advance(&self, glyph_id: GlyphId) -> Result<Size<f32>> {
        let font = self.skrifa_ref()?;
        let metrics = font.glyph_metrics(SkrifaSize::unscaled(), LocationRef::new(&[]));
        let glyph_id = skrifa::GlyphId::new(glyph_id.0);

        Ok(size(metrics.advance_width(glyph_id).unwrap_or(0.0), 0.0))
    }

    /// Returns the glyph's control bounds in font units, relative to its
    /// origin on the baseline.
    pub(crate) fn glyph_bounds(&self, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        let font = self.skrifa_ref()?;
        let bounds = font
            .glyph_metrics(SkrifaSize::unscaled(), LocationRef::new(&[]))
            .bounds(skrifa::GlyphId::new(glyph_id.0));
        let Some(bounds) = bounds else {
            return Ok(Bounds::default());
        };

        Ok(Bounds {
            origin: point(bounds.x_min, bounds.y_min),
            size: size(bounds.x_max - bounds.x_min, bounds.y_max - bounds.y_min),
        })
    }
}

/// Storage for concrete font faces plus the shaping style that reached them.
#[derive(Default)]
pub(crate) struct FontStore {
    fonts: Vec<LoadedFont>,
    ids_by_key: HashMap<FontKey, FontId>,
}

impl FontStore {
    /// Interns a face selected for a descriptor or a shaped run, returning its
    /// GPUI id. Glyph runs report the face they resolved to (including
    /// fallbacks), so this keeps rasterisation pinned to the same bytes.
    pub(crate) fn intern(
        &mut self,
        data: Blob<u8>,
        index: u32,
        synthesis: Synthesis,
        families: &[String],
        weight: f32,
        style: ParleyFontStyle,
    ) -> Result<FontId> {
        let key = FontKey {
            source_id: data.id(),
            face_index: index,
            embolden: synthesis.embolden(),
            skew_bits: synthesis.skew().map(f32::to_bits),
            style: match style {
                ParleyFontStyle::Normal => 0,
                ParleyFontStyle::Italic => 1,
                ParleyFontStyle::Oblique(_) => 2,
            },
        };

        if let Some(id) = self.ids_by_key.get(&key) {
            return Ok(*id);
        }

        let font = FontRef::from_index(data.as_ref(), index)
            .context("cannot intern a font face Skrifa cannot parse")?;
        let has_color_glyphs = [*b"CBDT", *b"sbix", *b"COLR", *b"SVG "]
            .into_iter()
            .any(|tag| font.table_data(Tag::new(&tag)).is_some());

        let id = FontId(self.fonts.len());
        self.fonts.push(LoadedFont {
            data,
            index,
            synthesis,
            families: families.to_vec(),
            weight,
            style,
            has_color_glyphs,
        });

        self.ids_by_key.insert(key, id);
        Ok(id)
    }

    /// Returns the stored font for an id.
    pub(crate) fn get(&self, id: FontId) -> Option<&LoadedFont> {
        self.fonts.get(id.0)
    }
}
