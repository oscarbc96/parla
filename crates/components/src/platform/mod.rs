//! Per-platform video decoding for clips.
//! macOS decodes through AVFoundation; Windows and Linux report
//! [`ClipError::Unsupported`] until M7, so clips fall back to the browser.

use std::path::Path;
use std::sync::Arc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Why a decoder could not play a clip. Rendered through the catalog
/// (`clip-error-*`), so it carries no prose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipError {
    Unsupported,
    FileMissing,
    PathNotUtf8,
    WrongThread,
    NoPlayableItem,
    /// The decoder gave up; the OS description when it has one.
    DecoderFailed(Option<String>),
}

/// Pixels of one decoded video frame.
#[derive(Clone, Debug)]
pub enum FrameData {
    /// Tightly packed BGRA, 4 bytes per pixel.
    Bgra(Arc<[u8]>),
    /// A decoder-owned buffer drawn by GPUI's surface element without a copy.
    #[cfg(target_os = "macos")]
    Surface(objc2_core_foundation::CFRetained<objc2_core_video::CVPixelBuffer>),
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: FrameData,
}

impl Frame {
    pub fn bgra(width: u32, height: u32, bytes: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data: FrameData::Bgra(bytes.into()),
        }
    }
}

/// An OS video decoder with audio, driven from the UI thread. Dropping it
/// stops playback and frees the decoder.
pub trait VideoPlayer {
    fn open(&mut self, path: &Path) -> Result<(), ClipError>;
    fn play(&mut self);
    fn pause(&mut self);
    fn seek(&mut self, ms: u64);
    /// The newest frame not yet returned, if any.
    fn frame(&mut self) -> Option<Frame>;
    fn position_ms(&self) -> u64;
    /// Set when the decoder gave up after `open` (for example an unsupported codec).
    fn failure(&self) -> Option<ClipError> {
        None
    }
    /// Playback speed (1.0 is normal) of a playing player. A paused player
    /// ignores it (a rate would start it), and `play` starts at normal speed,
    /// so callers set it again after `play`.
    fn set_rate(&mut self, _rate: f32) {}
    /// Output volume, 0.0 to 1.0.
    fn set_volume(&mut self, _volume: f32) {}
    fn set_muted(&mut self, _muted: bool) {}
}

/// Decoder of this platform.
pub fn new_player() -> Box<dyn VideoPlayer> {
    #[cfg(target_os = "macos")]
    return Box::new(macos::AvPlayer::new());
    #[cfg(target_os = "windows")]
    return Box::new(windows::Unsupported);
    #[cfg(target_os = "linux")]
    return Box::new(linux::Unsupported);
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Box::new(Unsupported);
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
struct Unsupported;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl VideoPlayer for Unsupported {
    fn open(&mut self, _: &Path) -> Result<(), ClipError> {
        Err(ClipError::Unsupported)
    }
    fn play(&mut self) {}
    fn pause(&mut self) {}
    fn seek(&mut self, _: u64) {}
    fn frame(&mut self) -> Option<Frame> {
        None
    }
    fn position_ms(&self) -> u64 {
        0
    }
}
