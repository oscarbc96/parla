//! The element that shows an image from the [`ImageCache`].

use domain::ImageRef;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, IntoElement, ParentElement, Pixels, RenderOnce, Size, Styled, Window,
    canvas, div, img, px, size,
};
use settings::GifAutoplay;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use super::anim::{Animation, should_play};
use super::cache::{ImageCache, ImageState};

/// An image fit into a `max_w` x `max_h` box. Without an [`ImageCache`] global
/// (tests, early startup) it falls back to its alt text.
#[derive(IntoElement)]
pub struct MediaImage {
    src: ImageRef,
    alt: String,
    max: (f32, f32),
    fixed: bool,
    /// Fill `max`, cropping the overflow (`MediaImage::cover`).
    cover: bool,
    /// `None` follows `gifs.autoplay_in_messages`.
    autoplay: Option<GifAutoplay>,
    /// Paint strength, for a dimmed (pending) row. An image ignores the text
    /// colour, so main's group opacity becomes element opacity on this leaf.
    strength: f32,
}

impl MediaImage {
    pub fn new(src: ImageRef, alt: impl Into<String>) -> Self {
        Self {
            src,
            alt: alt.into(),
            max: (
                themes::metrics::LAYOUT_MEDIA_WIDTH.as_f32(),
                themes::metrics::LAYOUT_MEDIA_HEIGHT.as_f32(),
            ),
            fixed: false,
            cover: false,
            autoplay: None,
            strength: 1.0,
        }
    }

    /// Fit within a box of this logical size.
    pub fn max_size(mut self, w: f32, h: f32) -> Self {
        self.max = (w, h);
        self
    }

    /// Always occupy exactly the box (custom emoji, avatars).
    pub fn fixed(mut self) -> Self {
        self.fixed = true;
        self
    }

    /// Fill the box, cropping the overflow instead of fitting inside it (the
    /// GIF grid, whose tiles are their item's shape with the height capped).
    pub fn cover(mut self) -> Self {
        self.cover = true;
        self
    }

    /// Paint at `strength`: a leaf, so opacity multiplies each picture once
    /// and dims a pending row the way main's group opacity did.
    pub fn dimmed(mut self, strength: f32) -> Self {
        self.strength = strength;
        self
    }

    pub fn autoplay(mut self, mode: GifAutoplay) -> Self {
        self.autoplay = Some(mode);
        self
    }

    /// The autoplay override this image was built with (a test hook for the
    /// picker's focused tile).
    pub fn autoplay_mode(&self) -> Option<GifAutoplay> {
        self.autoplay
    }

    fn alt_text(&self, cx: &App) -> String {
        t!(cx, "media-image-placeholder", alt = self.alt.to_string()).into_owned()
    }

    /// The box before the picture loads (or when it fails): `text` is the alt
    /// text. It keeps the row's colour, which is already dimmed for a pending
    /// row, so it takes no element opacity of its own.
    fn placeholder(&self, text: Option<String>) -> Div {
        let (w, h) = if self.fixed {
            self.max
        } else {
            (self.max.0.min(120.0), self.max.1.min(80.0))
        };
        div()
            .w(px(w))
            .h(px(h))
            .text_xs()
            .overflow_hidden()
            .children(text)
    }
}

fn fit_logical(w: f32, h: f32, mw: f32, mh: f32) -> (f32, f32) {
    let s = (mw / w).min(mh / h).min(1.0);
    (w * s, h * s)
}

impl RenderOnce for MediaImage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !cx.has_global::<ImageCache>() {
            return self.placeholder(Some(self.alt_text(cx))).into_any_element();
        }
        let scale = window.scale_factor();
        let boxed = size(px(self.max.0), px(self.max.1));
        let state = cx.update_global::<ImageCache, _>(|c, cx| c.get(&self.src, boxed, scale, cx));
        match state {
            ImageState::Loading => self.placeholder(None).into_any_element(),
            ImageState::Failed => self.placeholder(Some(self.alt_text(cx))).into_any_element(),
            ImageState::Ready(image) => {
                let s = image.size(0);
                let (iw, ih) = (s.width.0 as f32 / scale, s.height.0 as f32 / scale);
                let (w, h) = if self.fixed || self.cover {
                    self.max
                } else {
                    fit_logical(iw, ih, self.max.0, self.max.1)
                };
                let cover = self.cover;
                let mode = self.autoplay.unwrap_or_else(|| autoplay_setting(cx));
                let anim = animation_for(cx, &self.src, boxed, scale, mode);
                match anim {
                    None => img(image)
                        .w(px(w))
                        .h(px(h))
                        .when(cover, |i| i.object_fit(gpui::ObjectFit::Cover))
                        .when(self.strength < 1.0, |i| i.opacity(self.strength))
                        .into_any_element(),
                    Some(anim) => {
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, cx| {
                                let hovered = bounds.contains(&window.mouse_position());
                                let play =
                                    should_play(mode, true, window.is_window_active(), hovered);
                                let next =
                                    anim.borrow_mut().tick(cx.background_executor().now(), play);
                                let frame = anim.borrow().current();
                                let (bw, bh) =
                                    (bounds.size.width.as_f32(), bounds.size.height.as_f32());
                                // Cover: the widest scale, centred, cropped by
                                // the tile's `overflow_hidden`.
                                let (dw, dh) = if cover {
                                    let k = (bw / iw).max(bh / ih);
                                    (iw * k, ih * k)
                                } else {
                                    (bw, bh)
                                };
                                let draw = gpui::Bounds {
                                    origin: bounds.origin
                                        + gpui::point(px((bw - dw) / 2.0), px((bh - dh) / 2.0)),
                                    size: size(px(dw), px(dh)),
                                };
                                let _ = window.paint_image(
                                    draw,
                                    draw,
                                    Default::default(),
                                    frame,
                                    0,
                                    false,
                                );
                                if let Some(d) = next {
                                    // Wake only for the next frame while painted.
                                    wake_after(cx, d);
                                }
                            },
                        )
                        .w(px(w))
                        .h(px(h))
                        .when(self.strength < 1.0, |c| c.opacity(self.strength))
                        .into_any_element()
                    }
                }
            }
        }
    }
}

/// `gifs.autoplay_in_messages`, or `visible` before settings exist.
fn autoplay_setting(cx: &App) -> GifAutoplay {
    cx.try_global::<settings::SettingsStore>()
        .map_or(GifAutoplay::Visible, |s| {
            s.current().gifs.autoplay_in_messages
        })
}

/// Time comes from gpui's executor clock (fake in tests), never the wall clock.
///
/// Redraws the windows after `d`, sharing one pending timer between every
/// animation on screen: a later request while an earlier timer is armed adds
/// nothing, and the redraw repaints (and re-requests) all of them.
///
/// Wake-ups land on a shared 100 ms grid, so animations with different frame
/// times redraw together and the window never redraws more than 10 times a
/// second for them, however many play. Every tick re-lays out the window, so
/// the cap (not the GIF's own frame time) bounds the CPU cost.
fn wake_after(cx: &mut App, d: Duration) {
    const GRID: Duration = Duration::from_millis(100);
    let now = cx.background_executor().now();
    let epoch = cx.update_global::<ImageCache, _>(|c, _| c.grid_epoch(now));
    let steps = (now + d).saturating_duration_since(epoch).as_nanos() / GRID.as_nanos() + 1;
    let due = epoch + GRID * steps as u32;
    let d = due.saturating_duration_since(now);
    if !cx.update_global::<ImageCache, _>(|c, _| c.arm_wake(due)) {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(d).await;
        cx.update(|cx| {
            if cx.update_global::<ImageCache, _>(|c, _| c.take_wake(due)) {
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

/// Hook for [`crate::rich_text::RichTextCx::image`].
pub fn image_hook(src: &ImageRef, alt: &str) -> AnyElement {
    MediaImage::new(src.clone(), alt).into_any_element()
}

/// [`image_hook`] for a dimmed (pending) row.
pub fn image_hook_dimmed(src: &ImageRef, alt: &str, strength: f32) -> AnyElement {
    MediaImage::new(src.clone(), alt)
        .dimmed(strength)
        .into_any_element()
}

/// Hook for [`crate::rich_text::RichTextCx::emoji`]: `alt` is the shortcode.
pub fn emoji_hook(src: &ImageRef, alt: &str) -> AnyElement {
    MediaImage::new(src.clone(), alt)
        .max_size(
            themes::metrics::SPACE_7.as_f32(),
            themes::metrics::SPACE_7.as_f32(),
        )
        .fixed()
        .into_any_element()
}

/// [`emoji_hook`] for a dimmed (pending) row.
pub fn emoji_hook_dimmed(src: &ImageRef, alt: &str, strength: f32) -> AnyElement {
    MediaImage::new(src.clone(), alt)
        .max_size(
            themes::metrics::SPACE_7.as_f32(),
            themes::metrics::SPACE_7.as_f32(),
        )
        .fixed()
        .dimmed(strength)
        .into_any_element()
}

/// The animation to drive for `src`, or `None` when it must stay on its first
/// frame: autoplay is off or the app runs with frozen motion.
pub fn animation_for(
    cx: &mut App,
    src: &ImageRef,
    boxed: Size<Pixels>,
    scale: f32,
    autoplay: GifAutoplay,
) -> Option<Rc<RefCell<Animation>>> {
    if autoplay == GifAutoplay::Never || crate::motion(cx) == crate::Motion::Frozen {
        return None;
    }
    cx.update_global::<ImageCache, _>(|c, _| c.animation(src, boxed, scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholder(strength: f32) -> Div {
        MediaImage::new(ImageRef::Web("https://example.test/x.png".into()), "x")
            .dimmed(strength)
            .placeholder(Some("x".into()))
    }

    /// A pending row's inherited text colour is already faded: an element
    /// opacity here would fade the placeholder's alt text a second time.
    #[test]
    fn a_dimmed_placeholders_alt_text_is_not_faded_twice() {
        assert_eq!(placeholder(0.5).style().opacity, None);
        assert_eq!(placeholder(1.0).style().opacity, None);
    }
}
