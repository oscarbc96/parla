//! `UiHarness`: an offscreen GPUI app with bundled fonts, an explicit theme and
//! an explicit scale. The construction sequence is exercised by the capture
//! tests in `tests/`.

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Result;
use gpui::{
    AnyWindowHandle, App, Entity, HeadlessAppContext, Render, Window, WindowHandle, px, size,
};
use image::RgbaImage;
use settings::{Settings, ThemeMode};

use crate::fonts::{
    INTER_ITALIC, INTER_REGULAR, JETBRAINS_MONO_REGULAR, NOTO_EMOJI_SUBSET, require_family,
};
use crate::variant::{Appearance, Variant};

/// Logical window size every scenario renders at.
pub const LOGICAL_W: f32 = 960.0;
pub const LOGICAL_H: f32 = 600.0;

/// The scale the headless test window renders its frame at.
const WINDOW_SCALE: f32 = 2.0;

pub struct UiHarness {
    cx: HeadlessAppContext,
    variant: Variant,
    /// Logical size of the last opened window.
    logical: (f32, f32),
    /// Whether windows opened next retain their views between frames.
    retention: bool,
}

/// Parley shaping and Swash rasterisation over the bundled fonts, so text
/// renders identically on every OS. It needs no platform, so the harness runs
/// on any thread.
pub fn text_system() -> Arc<dyn gpui::PlatformTextSystem> {
    Arc::new(crate::text_system::ParleyTextSystem::new())
}

impl UiHarness {
    /// Builds the context: real text system, bundled fonts, explicit theme.
    pub fn new(variant: Variant) -> Result<Self> {
        Self::with_catalogs(variant, &[])
    }

    /// As [`UiHarness::new`], with the message catalogs of more crates (the
    /// components catalog is always loaded) in the source language.
    pub fn with_catalogs(variant: Variant, extra: &[&'static i18n::Catalog]) -> Result<Self> {
        let mut catalogs = vec![components::catalog()];
        catalogs.extend_from_slice(extra);
        let cx = HeadlessAppContext::with_platform(
            text_system(),
            Arc::new(()),
            gpui_platform::current_headless_renderer,
        );
        cx.text_system().add_fonts(vec![
            Cow::Borrowed(INTER_REGULAR),
            Cow::Borrowed(INTER_ITALIC),
            Cow::Borrowed(JETBRAINS_MONO_REGULAR),
            Cow::Borrowed(NOTO_EMOJI_SUBSET),
        ])?;
        require_family(&cx, "Inter")?;
        require_family(&cx, "JetBrains Mono")?;
        require_family(&cx, "Noto Emoji")?;
        let mut h = Self {
            cx,
            variant,
            logical: (LOGICAL_W, LOGICAL_H),
            retention: true,
        };
        let mode = match variant.appearance {
            Appearance::Light => ThemeMode::Light,
            Appearance::Dark => ThemeMode::Dark,
        };
        h.cx.update(|app| {
            let mut s = Settings::default();
            s.theme.mode = mode;
            themes::init(app, &s);
            i18n::gpui::init(
                app,
                &catalogs,
                i18n::LocalePref::Code(i18n::LocaleSet::embedded().source.clone()),
            );
            components::init(app);
            // Nothing depends on the machine: clock, time zone and motion are fixed.
            components::set_now(
                app,
                Arc::new(domain::FixedNow(
                    jiff::Timestamp::from_second(crate::scenario::NOW_SECS as i64)
                        .expect("valid timestamp"),
                )),
            );
            components::set_time_zone(app, jiff::tz::TimeZone::UTC);
            components::set_motion(app, components::Motion::Frozen);
        });
        Ok(h)
    }

    /// Sets whether windows opened from now on retain their views between
    /// frames (GPUI Fast's default) or draw every view from scratch each frame.
    pub fn set_view_retention(&mut self, enabled: bool) {
        self.retention = enabled;
    }

    pub fn variant(&self) -> Variant {
        self.variant
    }

    pub fn open<V: Render>(
        &mut self,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V> + 'static,
    ) -> WindowHandle<V> {
        self.open_sized(LOGICAL_W, LOGICAL_H, build)
    }

    /// Like `open`, for a window of another logical size (a standalone window).
    pub fn open_sized<V: Render>(
        &mut self,
        width: f32,
        height: f32,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V> + 'static,
    ) -> WindowHandle<V> {
        self.logical = (width, height);
        let scale = self.variant.scale;
        let w = self
            .cx
            .open_window(size(px(width), px(height)), build)
            .expect("open window");
        let retention = self.retention;
        self.cx
            .update_window(w.into(), |_, window, _| {
                window.set_scale_factor(scale);
                window.set_view_retention(retention);
            })
            .expect("set scale");
        w
    }

    /// Runs pending work, then draws one frame.
    pub fn settle(&mut self) {
        self.cx.run_until_parked();
        self.cx.update(|app| {
            for w in app.windows() {
                let _ = w.update(app, |_, window, app| window.draw(app).clear(app));
            }
        });
    }

    /// Moves the simulated clock without settling animations, for a shot of a
    /// transition in progress.
    pub fn advance_clock(&mut self, by: std::time::Duration) {
        self.cx.advance_clock(by);
    }

    /// Draws one frame and captures it, without running animations to their
    /// end (unlike [`UiHarness::capture`]). Queued animation frames run first,
    /// so a shot after [`UiHarness::advance_clock`] shows the clock's progress.
    pub fn capture_frame(&mut self, window: AnyWindowHandle) -> Result<RgbaImage> {
        self.cx.update(|app| {
            let _ = window.update(app, |_, window, cx| {
                window.simulate_next_frame(cx);
                window.draw(cx).clear(cx);
            });
        });
        let full = self.cx.capture_screenshot(window)?;
        let s = self.variant.scale;
        if (s - WINDOW_SCALE).abs() < f32::EPSILON {
            return Ok(full);
        }
        let (w, h) = ((self.logical.0 * s) as u32, (self.logical.1 * s) as u32);
        Ok(image::imageops::crop_imm(&full, 0, 0, w, h).to_image())
    }

    /// Draws and captures the window at the variant's scale. The headless
    /// window images at 2x, so other scales are the top-left crop.
    pub fn capture(&mut self, window: AnyWindowHandle) -> Result<RgbaImage> {
        self.settle();
        let full = self.cx.capture_screenshot(window)?;
        let s = self.variant.scale;
        if (s - WINDOW_SCALE).abs() < f32::EPSILON {
            return Ok(full);
        }
        let (w, h) = ((self.logical.0 * s) as u32, (self.logical.1 * s) as u32);
        Ok(image::imageops::crop_imm(&full, 0, 0, w, h).to_image())
    }

    pub fn require_family(&self, family: &str) -> Result<()> {
        require_family(&self.cx, family)
    }

    pub fn app(&mut self) -> &mut HeadlessAppContext {
        &mut self.cx
    }
}
