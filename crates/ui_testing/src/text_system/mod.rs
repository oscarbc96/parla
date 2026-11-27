//! One text pipeline for visual baselines on every OS.
//!
//! Ported from gpui-ce (Apache-2.0): `gpui_ce_parley::ParleyTextSystem`
//! shrunk to gpui-pre's `PlatformTextSystem` trait, with the shaping setup
//! from its paragraph layout and `SwashGlyphRasterizer` from
//! `gpui_ce_render`. See `../../NOTICE`.
//!
//! Only the fonts passed to [`ParleyTextSystem::add_fonts`] are visible, so
//! shaping never reads system fonts and identical bytes render identically
//! on macOS, Linux and Windows.

mod font_store;
mod rasterizer;

use std::{borrow::Cow, sync::Mutex};

use anyhow::{Context as _, Result, bail};
use fontique::{Attributes, Blob, Collection, CollectionOptions, QueryFamily, SourceCache};
use gpui::{
    Bounds, DevicePixels, Font, FontId, FontMetrics, FontRun, FontStyle as GpuiFontStyle, GlyphId,
    LineLayout, Pixels, PlatformTextSystem, RenderGlyphParams, ShapedGlyph, Size,
    TextRenderingMode, point, px,
};

// GPUI's `TextRenderingMode` and `Pixels` are re-exported above through the
// glob-free list; `Size` below is gpui's generic size.
use parley::{
    FontContext, FontFamily, FontFamilyName, FontStyle as ParleyFontStyle,
    FontWeight as ParleyFontWeight, FontWidth, LayoutContext, PositionedLayoutItem, StyleProperty,
};

use font_store::FontStore;
use rasterizer::SwashRasterizer;

/// Parley shaping plus Swash rasterisation over a closed set of fonts.
pub struct ParleyTextSystem {
    state: Mutex<State>,
    /// Families tried after the requested one, for every run (old harness:
    /// `.with_fallback_families(["Inter", "Noto Emoji"])`).
    fallback_families: Vec<String>,
}

struct State {
    font_context: FontContext,
    layout_context: LayoutContext,
    fonts: FontStore,
    rasterizer: SwashRasterizer,
}

/// Creates a font context that never discovers system fonts.
fn empty_font_context() -> FontContext {
    FontContext {
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        source_cache: SourceCache::default(),
    }
}

impl Default for ParleyTextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl ParleyTextSystem {
    /// Creates a text system with no fonts; call
    /// [`PlatformTextSystem::add_fonts`] before laying anything out.
    pub fn new() -> Self {
        Self::with_fallback_families(["Inter", "Noto Emoji"])
    }

    /// Sets the ordered families tried after a run's own, and where
    /// `.SystemUIFont` resolves to.
    pub fn with_fallback_families(families: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            state: Mutex::new(State {
                font_context: empty_font_context(),
                layout_context: LayoutContext::new(),
                fonts: FontStore::default(),
                rasterizer: SwashRasterizer::default(),
            }),
            fallback_families: families.into_iter().map(Into::into).collect(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("text system mutex poisoned")
    }

    /// Families to shape a descriptor with: requested families (or the
    /// system fallback), then the harness fallbacks.
    fn family_list(&self, descriptor: &Font) -> Vec<String> {
        let mut families = Vec::new();

        if descriptor.family == ".SystemUIFont" {
            if let Some(family) = self.fallback_families.first() {
                families.push(family.clone());
            }
        } else {
            families.push(descriptor.family.to_string());
        }

        if let Some(fallbacks) = &descriptor.fallbacks {
            for family in fallbacks.fallback_list() {
                if !families.contains(family) {
                    families.push(family.clone());
                }
            }
        }

        for family in &self.fallback_families {
            if !families.contains(family) {
                families.push(family.clone());
            }
        }

        families
    }

    fn resolve_font(&self, state: &mut State, descriptor: &Font) -> Result<FontId> {
        let families = self.family_list(descriptor);
        let weight = descriptor.weight.0;
        let style = match descriptor.style {
            GpuiFontStyle::Normal => ParleyFontStyle::Normal,
            GpuiFontStyle::Italic => ParleyFontStyle::Italic,
            GpuiFontStyle::Oblique => ParleyFontStyle::Oblique(None),
        };

        let resolved = {
            let collection = &mut state.font_context.collection;
            let source_cache = &mut state.font_context.source_cache;
            let mut query = collection.query(source_cache);
            query.set_families(families.iter().map(|family| QueryFamily::Named(family)));
            query.set_attributes(Attributes::new(
                FontWidth::from_percentage(100.0),
                style,
                ParleyFontWeight::new(weight),
            ));

            query
                .first_available_font()
                .with_context(|| format!("no registered font matches '{}'", descriptor.family))?
        };

        state.fonts.intern(
            resolved.blob,
            resolved.index,
            resolved.synthesis,
            &families,
            weight,
            style,
        )
    }

    /// Shapes each run's range, producing GPUI's single-line layout.
    fn shape_line(
        &self,
        state: &mut State,
        text: &str,
        font_size: Pixels,
        runs: &[FontRun],
    ) -> LineLayout {
        let mut run_styles = Vec::with_capacity(runs.len());
        let mut offset = 0_usize;
        for run in runs {
            let end = offset.saturating_add(run.len).min(text.len());
            if offset < end {
                let (families, weight, style) = match state.fonts.get(run.font_id) {
                    Some(font) => (font.families.clone(), font.weight, font.style),
                    None => (
                        self.fallback_families.clone(),
                        400.0,
                        ParleyFontStyle::Normal,
                    ),
                };
                run_styles.push((offset..end, families, weight, style));
            }
            offset = end;
        }

        let layout = {
            let State {
                font_context,
                layout_context,
                ..
            } = &mut *state;
            let mut builder = layout_context.ranged_builder(font_context, text, 1.0, false);
            builder.push_default(StyleProperty::FontSize(f32::from(font_size)));

            for (range, families, weight, style) in &run_styles {
                let names = families
                    .iter()
                    .map(|family| FontFamilyName::named(family.as_str()))
                    .collect::<Vec<_>>();
                builder.push(
                    StyleProperty::FontFamily(FontFamily::from(names.as_slice())),
                    range.clone(),
                );
                builder.push(
                    StyleProperty::FontWeight(ParleyFontWeight::new(*weight)),
                    range.clone(),
                );
                builder.push(StyleProperty::FontStyle(*style), range.clone());
            }

            let mut layout = builder.build(text);
            layout.break_all_lines(None);
            layout
        };

        let mut shaped_runs: Vec<gpui::ShapedRun> = Vec::new();
        let mut width = Pixels::ZERO;
        let mut ascent = Pixels::ZERO;
        let mut descent = Pixels::ZERO;

        for line in layout.lines() {
            width = width.max(px(line.metrics().advance));

            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };

                let run = glyph_run.run();
                let weight = run.font_attrs().weight.value();
                let style = run.font_attrs().style;
                let font_id = state
                    .fonts
                    .intern(
                        run.font().data.clone(),
                        run.font().index,
                        run.synthesis(),
                        &self.fallback_families,
                        weight,
                        style,
                    )
                    .expect("a shaped font face came from the registered collection");
                let is_emoji = state
                    .fonts
                    .get(font_id)
                    .is_some_and(|font| font.has_color_glyphs);

                let run_metrics = run.font_metrics();
                ascent = ascent.max(px(run_metrics.ascent));
                descent = descent.max(px(run_metrics.descent));

                let baseline = glyph_run.baseline();
                let mut positioned = glyph_run.positioned_glyphs();
                let mut next_glyph = positioned.next();
                let mut glyphs = Vec::new();

                for cluster in run.visual_clusters() {
                    let index = cluster.text_range().start;
                    for _ in 0..cluster.glyphs().count() {
                        let Some(glyph) = next_glyph.take() else {
                            break;
                        };

                        glyphs.push(ShapedGlyph {
                            id: GlyphId(glyph.id),
                            position: point(px(glyph.x), px(glyph.y - baseline)),
                            index,
                            is_emoji,
                        });
                        next_glyph = positioned.next();
                    }
                }

                match shaped_runs.last_mut() {
                    Some(run) if run.font_id == font_id => run.glyphs.extend(glyphs),
                    _ => shaped_runs.push(gpui::ShapedRun { font_id, glyphs }),
                }
            }
        }

        LineLayout {
            font_size,
            width,
            ascent,
            descent,
            runs: shaped_runs,
            len: text.len(),
        }
    }
}

impl PlatformTextSystem for ParleyTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        let blobs = fonts
            .into_iter()
            .map(|bytes| Blob::from(bytes.into_owned()))
            .collect::<Vec<_>>();

        let mut state = self.lock();
        register_font_blobs(&mut state.font_context, &blobs)
    }

    fn all_font_names(&self) -> Vec<String> {
        let mut state = self.lock();
        let mut names = state
            .font_context
            .collection
            .family_names()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        drop(state);

        names.push(".SystemUIFont".to_owned());
        names.sort_unstable();
        names.dedup();
        names
    }

    fn font_id(&self, descriptor: &Font) -> Result<FontId> {
        let mut state = self.lock();
        self.resolve_font(&mut state, descriptor)
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.lock()
            .fonts
            .get(font_id)
            .expect("FontId missing from the text system's font store")
            .metrics()
            .expect("stored font failed Skrifa metrics")
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        let state = self.lock();
        let font = state
            .fonts
            .get(font_id)
            .context("FontId missing from the text system's font store")?;

        font.glyph_bounds(glyph_id)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        let state = self.lock();
        let font = state
            .fonts
            .get(font_id)
            .context("FontId missing from the text system's font store")?;

        font.advance(glyph_id)
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        let state = self.lock();
        state.fonts.get(font_id)?.glyph_for_char(ch).ok()?
    }

    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        let mut state = self.lock();
        let State {
            fonts, rasterizer, ..
        } = &mut *state;
        let font = fonts
            .get(params.font_id)
            .context("FontId missing from the text system's font store")?;

        rasterizer.raster_bounds(params.font_id, font, params)
    }

    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        _raster_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        let mut state = self.lock();
        let State {
            fonts, rasterizer, ..
        } = &mut *state;
        let font = fonts
            .get(params.font_id)
            .context("FontId missing from the text system's font store")?;

        rasterizer.rasterize(params.font_id, font, params)
    }

    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
        let mut state = self.lock();
        self.shape_line(&mut state, text, font_size, runs)
    }

    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        TextRenderingMode::Grayscale
    }
}

/// Registers font blobs after validating the entire batch, so a bad blob
/// leaves the collection unchanged.
fn register_font_blobs(context: &mut FontContext, fonts: &[Blob<u8>]) -> Result<()> {
    let mut validator = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });

    for blob in fonts {
        if validator.register_fonts(blob.clone(), None).is_empty() {
            bail!("font data did not contain a supported font face");
        }
    }

    for blob in fonts {
        context.collection.register_fonts(blob.clone(), None);
    }

    Ok(())
}

/// Re-exported so tests can name the concrete type the harness uses.
pub type TestTextSystem = ParleyTextSystem;
