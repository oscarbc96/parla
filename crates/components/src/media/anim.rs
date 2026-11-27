//! Lazily decoded GIF playback that only advances when allowed.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::RenderImage;
use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, ImageDecoder};
use image::{DynamicImage, Frames};
use settings::GifAutoplay;

use super::MediaError;
use super::decode::{fit, limits, to_bgra};

/// Shortest frame time honoured; browsers clamp tiny GIF delays the same way.
const MIN_DELAY: Duration = Duration::from_millis(20);

/// Whether an animation may advance: the element was painted this frame, the
/// window is active, and the setting allows it (`hover` also needs the pointer
/// over it, `never` shows the first frame only).
pub fn should_play(mode: GifAutoplay, painted: bool, window_active: bool, hovered: bool) -> bool {
    painted
        && window_active
        && match mode {
            GifAutoplay::Visible => true,
            GifAutoplay::Hover => hovered,
            GifAutoplay::Never => false,
        }
}

/// Decoded frames one animation may keep for replay. A longer GIF stops
/// caching and decodes frame by frame, restarting the decoder at each loop.
pub const ANIM_CACHE_CAP: usize = 8 << 20;

/// The shared GIF bytes as a reader source.
struct Shared(Arc<Vec<u8>>);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

type Decoded = (Arc<RenderImage>, Duration, usize);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Still reading the first pass; every frame is kept.
    Caching,
    /// Every frame is cached; loops replay from memory.
    Complete,
    /// Over the cap: only the current frame is held.
    Streaming,
}

pub struct Animation {
    bytes: Arc<Vec<u8>>,
    frames: Frames<'static>,
    /// Frames of the first pass, while they fit under `cap`.
    cache: Vec<Decoded>,
    cache_bytes: usize,
    cap: usize,
    mode: Mode,
    cur: Decoded,
    index: usize,
    shown_at: Option<Instant>,
    target: (u32, u32),
}

fn open(bytes: &Arc<Vec<u8>>) -> Result<Frames<'static>, MediaError> {
    let mut dec =
        GifDecoder::new(Cursor::new(Shared(bytes.clone()))).map_err(|_| MediaError::Decode)?;
    dec.set_limits(limits()).map_err(|_| MediaError::Decode)?;
    Ok(dec.into_frames())
}

fn pull(frames: &mut Frames<'static>, target: (u32, u32)) -> Option<Decoded> {
    let f = frames.next()?.ok()?;
    let delay = Duration::from(f.delay()).max(MIN_DELAY);
    let img = DynamicImage::ImageRgba8(f.into_buffer());
    let (w, h) = (img.width(), img.height());
    let (fw, fh) = fit(w, h, target.0, target.1);
    let img = if (fw, fh) != (w, h) {
        img.resize_exact(fw, fh, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut rgba = img.into_rgba8();
    to_bgra(&mut rgba);
    let size = rgba.as_raw().len();
    Some((
        Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])),
        delay,
        size,
    ))
}

impl Animation {
    /// Reads the GIF header and the first frame; later frames decode on demand.
    pub fn new(
        bytes: impl Into<Arc<Vec<u8>>>,
        target_w: u32,
        target_h: u32,
    ) -> Result<Self, MediaError> {
        Self::with_cap(bytes, target_w, target_h, ANIM_CACHE_CAP)
    }

    /// Like [`Animation::new`] with a replay cache size in bytes.
    pub fn with_cap(
        bytes: impl Into<Arc<Vec<u8>>>,
        target_w: u32,
        target_h: u32,
        cap: usize,
    ) -> Result<Self, MediaError> {
        let bytes = bytes.into();
        let mut frames = open(&bytes)?;
        let target = (target_w, target_h);
        let first = pull(&mut frames, target).ok_or(MediaError::Decode)?;
        Ok(Self {
            bytes,
            frames,
            cache_bytes: first.2,
            cache: vec![first.clone()],
            cap,
            mode: Mode::Caching,
            cur: first,
            index: 0,
            shown_at: None,
            target,
        })
    }

    /// Decoded bytes this animation holds right now (for the image budget).
    pub fn memory_bytes(&self) -> usize {
        match self.mode {
            Mode::Streaming => self.cur.2,
            _ => self.cache_bytes,
        }
    }

    /// Moves to the next frame, looping.
    fn advance(&mut self) {
        match self.mode {
            Mode::Complete => {
                self.index = (self.index + 1) % self.cache.len();
                self.cur = self.cache[self.index].clone();
            }
            Mode::Caching => match pull(&mut self.frames, self.target) {
                Some(f) if self.cache_bytes + f.2 <= self.cap => {
                    self.cache_bytes += f.2;
                    self.cache.push(f.clone());
                    self.index += 1;
                    self.cur = f;
                }
                Some(f) => {
                    self.cache = Vec::new();
                    self.cache_bytes = 0;
                    self.mode = Mode::Streaming;
                    self.index += 1;
                    self.cur = f;
                }
                None => {
                    self.mode = Mode::Complete;
                    self.index = 0;
                    self.cur = self.cache[0].clone();
                }
            },
            Mode::Streaming => {
                if let Some(f) = pull(&mut self.frames, self.target) {
                    self.index += 1;
                    self.cur = f;
                } else if let Ok(mut frames) = open(&self.bytes)
                    && let Some(f) = pull(&mut frames, self.target)
                {
                    self.frames = frames;
                    self.index = 0;
                    self.cur = f;
                }
            }
        }
    }

    pub fn frame_index(&self) -> usize {
        self.index
    }

    pub fn current(&self) -> Arc<RenderImage> {
        self.cur.0.clone()
    }

    fn delay(&self) -> Duration {
        self.cur.1
    }

    /// Advances at most one frame when `play` is true. Returns the time until
    /// the next frame is due, or `None` when stopped (the caller then
    /// schedules nothing: no timer runs for a frozen animation).
    pub fn tick(&mut self, now: Instant, play: bool) -> Option<Duration> {
        if !play {
            self.shown_at = None;
            return None;
        }
        let Some(at) = self.shown_at else {
            self.shown_at = Some(now);
            return Some(self.delay());
        };
        let elapsed = now.saturating_duration_since(at);
        if elapsed >= self.delay() {
            self.advance();
            self.shown_at = Some(now);
            return Some(self.delay());
        }
        Some(self.delay() - elapsed)
    }
}
