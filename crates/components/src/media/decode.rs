//! Decoding to GPUI's BGRA `RenderImage`, downscaled to the display size.

use std::io::Cursor;
use std::sync::Arc;

use gpui::RenderImage;
use image::imageops::FilterType;
use image::{DynamicImage, Frame, ImageFormat, ImageReader, Limits, RgbaImage};

use super::MediaError;

/// A decoded image plus what the cache needs to account for it.
pub struct Decoded {
    pub image: Arc<RenderImage>,
    /// Decoded size in bytes.
    pub bytes: usize,
    /// The source is a GIF (an [`super::Animation`] may be built from it).
    pub gif: bool,
}

pub(crate) fn limits() -> Limits {
    let mut l = Limits::default();
    l.max_image_width = Some(16_384);
    l.max_image_height = Some(16_384);
    l.max_alloc = Some(256 * 1024 * 1024);
    l
}

/// Scales `(w, h)` down to fit `(tw, th)`; never up. `0` targets mean no limit.
pub(crate) fn fit(w: u32, h: u32, tw: u32, th: u32) -> (u32, u32) {
    if tw == 0 || th == 0 || (w <= tw && h <= th) {
        return (w, h);
    }
    let s = (tw as f64 / w as f64).min(th as f64 / h as f64);
    (
        ((w as f64 * s).round() as u32).max(1),
        ((h as f64 * s).round() as u32).max(1),
    )
}

/// Converts RGBA to the BGRA GPUI renders, in place.
pub(crate) fn to_bgra(img: &mut RgbaImage) {
    for p in img.pixels_mut() {
        p.0.swap(0, 2);
    }
}

pub(crate) fn render_frame(img: DynamicImage, tw: u32, th: u32) -> (RenderImage, usize) {
    let (w, h) = fit(img.width(), img.height(), tw, th);
    let img = if (w, h) != (img.width(), img.height()) {
        img.resize_exact(w, h, FilterType::Triangle)
    } else {
        img
    };
    let mut rgba = img.into_rgba8();
    to_bgra(&mut rgba);
    let bytes = rgba.as_raw().len();
    (RenderImage::new(vec![Frame::new(rgba)]), bytes)
}

/// Decodes `bytes` (PNG, JPEG, GIF first frame, WebP) at no more than
/// `target_w` x `target_h` device pixels.
pub fn decode(bytes: &[u8], target_w: u32, target_h: u32) -> Result<Decoded, MediaError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| MediaError::Decode)?;
    reader.limits(limits());
    let gif = reader.format() == Some(ImageFormat::Gif);
    let img = reader.decode().map_err(|_| MediaError::Decode)?;
    let (image, bytes) = render_frame(img, target_w, target_h);
    Ok(Decoded {
        image: Arc::new(image),
        bytes,
        gif,
    })
}
