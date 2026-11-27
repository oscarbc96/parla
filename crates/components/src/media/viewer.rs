//! Decoded images of the media viewer. Only the files the viewer asks for
//! (the current one and its next neighbour) stay decoded: asking for a new set
//! drops every other image, so memory stays at most [`VIEWER_KEEP`] images of
//! at most [`VIEWER_MAX_PX`] a side, however many files the gallery steps
//! through. A larger image is decoded once and downscaled to that size, one
//! at a time and never as a preload; above [`VIEWER_DECODE_MAX_PIXELS`] it is
//! refused from its header, before any pixel is decoded. The strip's thumbnails are decoded small, at most
//! [`VIEWER_THUMBS`] of them. Bytes come through the viewer's own
//! [`MediaFetcher`] (the workspace it was opened from) into the shared disk
//! cache, whose [`DiskBudget`] each write counts against.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use domain::ImageRef;
use gpui::{AsyncApp, Context, RenderImage, WeakEntity};
use image::{DynamicImage, ImageDecoder, ImageError, ImageReader};

use super::cache::{DiskBudget, key_hash, src_key};
use super::decode::{decode, limits, render_frame};
use super::{DISK_CAP_BYTES, MAX_IMAGE_BYTES, MediaError, MediaFetcher};

/// Most images the viewer keeps decoded at once.
pub const VIEWER_KEEP: usize = 2;

/// Longest side, in pixels, the viewer shows; a larger image is refused.
pub const VIEWER_MAX_PX: u32 = 4096;

/// Most pixels the viewer shows at their own size (a full [`VIEWER_MAX_PX`]
/// square); a larger image is downscaled to fit.
pub const VIEWER_MAX_PIXELS: u64 = VIEWER_MAX_PX as u64 * VIEWER_MAX_PX as u64;

/// Most pixels the viewer decodes in full to downscale (50 MP: a 48 MP phone
/// photo fits); a larger image is refused from its header.
pub const VIEWER_DECODE_MAX_PIXELS: u64 = 50_000_000;

/// Most bytes that full decode may allocate: 50 MP of 8-bit RGBA, 200 MB
/// (JPEG decodes to RGB, 150 MB). Only one such decode runs at a time, so the
/// viewer's transient peak is this plus the downscaled copy (at most 64 MB)
/// and its BGRA conversion (at most 64 MB).
pub const VIEWER_DECODE_MAX_BYTES: u64 = VIEWER_DECODE_MAX_PIXELS * 4;

/// Held while an image over [`VIEWER_MAX_PIXELS`] is decoded in full.
static LARGE_DECODE: Mutex<()> = Mutex::new(());

/// Most thumbnails the viewer keeps decoded (its strip).
pub const VIEWER_THUMBS: usize = 9;

/// Device pixels a strip thumbnail is decoded at.
const THUMB_PX: (u32, u32) = (128, 88);

/// One decoded viewer image.
#[derive(Clone)]
pub struct ViewerImage {
    pub image: Arc<RenderImage>,
    /// Pixel size of the original file.
    pub natural: (u32, u32),
}

#[derive(Clone)]
pub enum ViewerState {
    Loading,
    Ready(ViewerImage),
    Failed,
}

struct Slot {
    key: String,
    state: ViewerState,
}

/// What a slot decodes to.
#[derive(Clone, Copy)]
enum Kind {
    Full,
    Thumb,
}

/// The viewer's decoded images. Owned by one viewer window.
pub struct ViewerCache {
    fetcher: Arc<dyn MediaFetcher>,
    disk: PathBuf,
    budget: Arc<DiskBudget>,
    slots: Vec<Slot>,
    thumbs: Vec<Slot>,
}

/// Decodes `bytes` for the viewer. The size is read from the header first:
/// an image within [`VIEWER_MAX_PX`] a side and [`VIEWER_MAX_PIXELS`] is shown
/// as is; a larger one up to [`VIEWER_DECODE_MAX_PIXELS`] (and
/// [`VIEWER_DECODE_MAX_BYTES`] decoded) is decoded once, one at a time, and
/// shown downscaled to fit [`VIEWER_MAX_PX`]; anything above is refused
/// without decoding it.
pub fn decode_viewer(bytes: &[u8]) -> Result<ViewerImage, MediaError> {
    decode_viewer_with(bytes, true).map(|img| img.expect("large images allowed"))
}

/// [`decode_viewer`]; `None` when the image is over the normal cap and
/// `large` is false (a preload that waits until it is the current image).
fn decode_viewer_with(bytes: &[u8], large: bool) -> Result<Option<ViewerImage>, MediaError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| MediaError::Decode)?;
    reader.limits(limits());
    let decoder = reader.into_decoder().map_err(|e| match e {
        ImageError::Limits(_) => MediaError::TooLarge,
        _ => MediaError::Decode,
    })?;
    let (w, h) = decoder.dimensions();
    let pixels = u64::from(w) * u64::from(h);
    if pixels > VIEWER_DECODE_MAX_PIXELS || decoder.total_bytes() > VIEWER_DECODE_MAX_BYTES {
        return Err(MediaError::TooLarge);
    }
    let natural = (w, h);
    let fits = w <= VIEWER_MAX_PX && h <= VIEWER_MAX_PX && pixels <= VIEWER_MAX_PIXELS;
    let img = if fits {
        DynamicImage::from_decoder(decoder).map_err(|_| MediaError::Decode)?
    } else if !large {
        return Ok(None);
    } else {
        // One full-size buffer at a time, process-wide.
        let _one = LARGE_DECODE.lock().unwrap_or_else(|e| e.into_inner());
        let full = DynamicImage::from_decoder(decoder).map_err(|_| MediaError::Decode)?;
        // Box-filter straight to the display size in the source's pixel type;
        // the full buffer is dropped before the BGRA copy is made.
        full.thumbnail(VIEWER_MAX_PX, VIEWER_MAX_PX)
    };
    let (image, _) = render_frame(img, VIEWER_MAX_PX, VIEWER_MAX_PX);
    Ok(Some(ViewerImage {
        image: Arc::new(image),
        natural,
    }))
}

impl ViewerCache {
    pub fn new(fetcher: Arc<dyn MediaFetcher>, disk: PathBuf) -> Self {
        let budget = Arc::new(DiskBudget::new(disk.clone(), DISK_CAP_BYTES));
        Self::with_budget(fetcher, disk, budget)
    }

    /// A cache that writes into `disk` under a budget shared with the
    /// workspace's [`super::ImageCache`].
    pub fn with_budget(
        fetcher: Arc<dyn MediaFetcher>,
        disk: PathBuf,
        budget: Arc<DiskBudget>,
    ) -> Self {
        Self {
            fetcher,
            disk,
            budget,
            slots: Vec::new(),
            thumbs: Vec::new(),
        }
    }

    /// The fetcher images load through.
    pub fn fetcher(&self) -> &Arc<dyn MediaFetcher> {
        &self.fetcher
    }

    /// Images held (loading, ready or failed).
    pub fn held(&self) -> usize {
        self.slots.len()
    }

    /// Images held decoded.
    pub fn decoded(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| matches!(s.state, ViewerState::Ready(_)))
            .count()
    }

    fn find(slots: &[Slot], src: &ImageRef) -> Option<ViewerState> {
        let k = src_key(src);
        slots.iter().find(|s| s.key == k).map(|s| s.state.clone())
    }

    /// State of `src`, `None` when not wanted.
    pub fn state(&self, src: &ImageRef) -> Option<ViewerState> {
        Self::find(&self.slots, src)
    }

    /// State of thumbnail `src`, `None` when not wanted.
    pub fn thumb(&self, src: &ImageRef) -> Option<ViewerState> {
        Self::find(&self.thumbs, src)
    }

    /// The disk cache file of `src` (its original bytes once fetched).
    pub fn cached_path(&self, src: &ImageRef) -> PathBuf {
        self.disk.join(key_hash(src_key(src)))
    }

    /// Keeps exactly `wanted` (at most [`VIEWER_KEEP`]): drops every other
    /// image and starts loading the missing ones. `cache` finds this cache in
    /// the view, which is notified as each image finishes.
    pub fn want<V: 'static>(
        &mut self,
        wanted: &[ImageRef],
        cache: impl Fn(&mut V) -> &mut ViewerCache + Clone + 'static,
        cx: &mut Context<V>,
    ) {
        self.want_kind(Kind::Full, wanted, VIEWER_KEEP, cache, cx);
    }

    /// Keeps exactly the strip thumbnails `wanted` (at most [`VIEWER_THUMBS`]).
    pub fn want_thumbs<V: 'static>(
        &mut self,
        wanted: &[ImageRef],
        cache: impl Fn(&mut V) -> &mut ViewerCache + Clone + 'static,
        cx: &mut Context<V>,
    ) {
        self.want_kind(Kind::Thumb, wanted, VIEWER_THUMBS, cache, cx);
    }

    fn slots_mut(&mut self, kind: Kind) -> &mut Vec<Slot> {
        match kind {
            Kind::Full => &mut self.slots,
            Kind::Thumb => &mut self.thumbs,
        }
    }

    fn want_kind<V: 'static>(
        &mut self,
        kind: Kind,
        wanted: &[ImageRef],
        max: usize,
        cache: impl Fn(&mut V) -> &mut ViewerCache + Clone + 'static,
        cx: &mut Context<V>,
    ) {
        let wanted: Vec<&ImageRef> = wanted.iter().take(max).collect();
        let keys: Vec<String> = wanted.iter().map(|s| src_key(s).to_string()).collect();
        self.slots_mut(kind).retain(|s| keys.contains(&s.key));
        for (i, (src, key)) in wanted.into_iter().zip(keys).enumerate() {
            // Only the current image (the first) may need a large decode.
            let large = i == 0;
            if self.slots_mut(kind).iter().any(|s| s.key == key) {
                continue;
            }
            self.slots_mut(kind).push(Slot {
                key: key.clone(),
                state: ViewerState::Loading,
            });
            let (fetcher, budget) = (self.fetcher.clone(), self.budget.clone());
            let path = self.disk.join(key_hash(&key));
            let (src, cache) = (src.clone(), cache.clone());
            cx.spawn(async move |this: WeakEntity<V>, cx: &mut AsyncApp| {
                let result = load(&*fetcher, &budget, &path, &src, kind, large, cx).await;
                let _ = this.update(cx, |v, cx| {
                    // Dropped while loading: keep nothing.
                    let slots = cache(v).slots_mut(kind);
                    let Some(at) = slots.iter().position(|s| s.key == key) else {
                        return;
                    };
                    match result {
                        Ok(Some(img)) => slots[at].state = ViewerState::Ready(img),
                        // A large preload: decoded once it is current.
                        Ok(None) => drop(slots.remove(at)),
                        Err(_) => slots[at].state = ViewerState::Failed,
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Puts a decoded image in place (visual tests).
    #[doc(hidden)]
    pub fn insert_ready(&mut self, src: &ImageRef, image: ViewerImage) {
        let k = src_key(src).to_string();
        self.slots.retain(|s| s.key != k);
        self.slots.push(Slot {
            key: k,
            state: ViewerState::Ready(image),
        });
    }

    /// Marks `src` as failed to load (visual tests).
    #[doc(hidden)]
    pub fn insert_failed(&mut self, src: &ImageRef) {
        let k = src_key(src).to_string();
        self.slots.retain(|s| s.key != k);
        self.slots.push(Slot {
            key: k,
            state: ViewerState::Failed,
        });
    }
}

async fn load(
    fetcher: &dyn MediaFetcher,
    budget: &Arc<DiskBudget>,
    path: &Path,
    src: &ImageRef,
    kind: Kind,
    large: bool,
    cx: &mut AsyncApp,
) -> Result<Option<ViewerImage>, MediaError> {
    let bg = cx.background_executor().clone();
    let p = path.to_path_buf();
    let cached = bg.spawn(async move { std::fs::read(p).ok() }).await;
    let (bytes, from_disk) = match cached {
        Some(b) => (b, true),
        None => (fetcher.fetch(src, MAX_IMAGE_BYTES).await?, false),
    };
    let (p, budget) = (path.to_path_buf(), budget.clone());
    bg.spawn(async move {
        if !from_disk {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if super::cache::write_atomic(&p, &bytes).is_ok() {
                budget.record(bytes.len() as u64);
            }
        }
        let r = match kind {
            Kind::Full => decode_viewer_with(&bytes, large),
            Kind::Thumb => decode(&bytes, THUMB_PX.0, THUMB_PX.1).map(|d| {
                let s = d.image.size(0);
                Some(ViewerImage {
                    image: d.image,
                    natural: (s.width.0 as u32, s.height.0 as u32),
                })
            }),
        };
        if matches!(r, Err(MediaError::Decode)) && from_disk {
            let _ = std::fs::remove_file(&p);
        }
        r
    })
    .await
}

impl ViewerImage {
    /// An image from RGBA pixels (tests and visual baselines).
    #[doc(hidden)]
    pub fn from_rgba(mut img: image::RgbaImage) -> Self {
        let natural = (img.width(), img.height());
        super::decode::to_bgra(&mut img);
        Self {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(img)])),
            natural,
        }
    }

    /// A one-colour image (tests).
    #[doc(hidden)]
    pub fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Self {
        Self::from_rgba(image::RgbaImage::from_pixel(w, h, image::Rgba(rgba)))
    }
}

/// The clipboard form of an image file's bytes, `None` when the type is not
/// one the clipboard takes.
pub fn clipboard_image(bytes: Vec<u8>) -> Option<gpui::Image> {
    let format = match image::guess_format(&bytes).ok()? {
        image::ImageFormat::Png => gpui::ImageFormat::Png,
        image::ImageFormat::Jpeg => gpui::ImageFormat::Jpeg,
        image::ImageFormat::Gif => gpui::ImageFormat::Gif,
        image::ImageFormat::WebP => gpui::ImageFormat::Webp,
        _ => return None,
    };
    Some(gpui::Image::from_bytes(format, bytes))
}
