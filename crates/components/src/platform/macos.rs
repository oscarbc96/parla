//! AVFoundation playback. `AVPlayer` decodes video and plays the audio; an
//! `AVPlayerItemVideoOutput` hands out `CVPixelBuffer`s in the bi-planar
//! YCbCr format GPUI's surface element draws without a copy.

use std::path::Path;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_av_foundation::{AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerItemVideoOutput};
use objc2_core_foundation::CFRetained;
use objc2_core_media::CMTime;
use objc2_core_video::{
    CVPixelBufferGetHeight, CVPixelBufferGetWidth, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

use super::{ClipError, Frame, FrameData, VideoPlayer};

struct Open {
    player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    output: Retained<AVPlayerItemVideoOutput>,
}

pub struct AvPlayer {
    open: Option<Open>,
}

impl AvPlayer {
    pub fn new() -> Self {
        Self { open: None }
    }
}

fn time(ms: u64) -> CMTime {
    // SAFETY: plain value constructor.
    unsafe { CMTime::with_seconds(ms as f64 / 1000.0, 1000) }
}

impl VideoPlayer for AvPlayer {
    fn open(&mut self, path: &Path) -> Result<(), ClipError> {
        let path = path.to_str().ok_or(ClipError::PathNotUtf8)?;
        if !Path::new(path).is_file() {
            return Err(ClipError::FileMissing);
        }
        let mtm = MainThreadMarker::new().ok_or(ClipError::WrongThread)?;
        // SAFETY: AVFoundation objects are created and used on the UI thread.
        unsafe {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            let player = AVPlayer::initWithURL(AVPlayer::alloc(mtm), &url);
            let item = player.currentItem().ok_or(ClipError::NoPlayableItem)?;
            let key: &NSString =
                &*(kCVPixelBufferPixelFormatTypeKey as *const _ as *const NSString);
            let fmt = NSNumber::new_u32(kCVPixelFormatType_420YpCbCr8BiPlanarFullRange);
            let fmt: &AnyObject = &fmt;
            let attrs = NSDictionary::from_slices(&[key], &[fmt]);
            let output = AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
                AVPlayerItemVideoOutput::alloc(),
                Some(&attrs),
            );
            item.addOutput(&output);
            self.open = Some(Open {
                player,
                item,
                output,
            });
        }
        Ok(())
    }

    fn play(&mut self) {
        if let Some(o) = &self.open {
            // SAFETY: UI thread, live player.
            unsafe { o.player.play() };
        }
    }

    fn pause(&mut self) {
        if let Some(o) = &self.open {
            // SAFETY: UI thread, live player.
            unsafe { o.player.pause() };
        }
    }

    fn seek(&mut self, ms: u64) {
        if let Some(o) = &self.open {
            // SAFETY: UI thread, live player.
            unsafe { o.player.seekToTime(time(ms)) };
        }
    }

    fn frame(&mut self) -> Option<Frame> {
        let o = self.open.as_ref()?;
        // SAFETY: UI thread; the output belongs to the live item.
        unsafe {
            let now = o.player.currentTime();
            if !o.output.hasNewPixelBufferForItemTime(now) {
                return None;
            }
            let buf = o
                .output
                .copyPixelBufferForItemTime_itemTimeForDisplay(now, std::ptr::null_mut())?;
            let buf: CFRetained<_> = CFRetained::retain(NonNull::from(&*buf).cast());
            Some(Frame {
                width: CVPixelBufferGetWidth(&buf) as u32,
                height: CVPixelBufferGetHeight(&buf) as u32,
                data: FrameData::Surface(buf),
            })
        }
    }

    fn position_ms(&self) -> u64 {
        let Some(o) = &self.open else { return 0 };
        // SAFETY: UI thread, live player.
        let s = unsafe { o.player.currentTime().seconds() };
        if s.is_finite() && s > 0.0 {
            (s * 1000.0) as u64
        } else {
            0
        }
    }

    fn set_rate(&mut self, rate: f32) {
        if let Some(o) = &self.open {
            // SAFETY: UI thread, live player. A paused player stays paused:
            // a non-zero `setRate` would start it.
            unsafe {
                if o.player.rate() != 0.0 {
                    o.player.setRate(rate);
                }
            }
        }
    }

    fn set_volume(&mut self, volume: f32) {
        if let Some(o) = &self.open {
            // SAFETY: UI thread, live player.
            unsafe { o.player.setVolume(volume.clamp(0.0, 1.0)) };
        }
    }

    fn set_muted(&mut self, muted: bool) {
        if let Some(o) = &self.open {
            // SAFETY: UI thread, live player.
            unsafe { o.player.setMuted(muted) };
        }
    }

    fn failure(&self) -> Option<ClipError> {
        let o = self.open.as_ref()?;
        // SAFETY: UI thread, live item.
        unsafe {
            if o.item.status() == AVPlayerItemStatus::Failed {
                let why = o.item.error().map(|e| e.localizedDescription().to_string());
                Some(ClipError::DecoderFailed(why))
            } else {
                None
            }
        }
    }
}

impl Drop for AvPlayer {
    fn drop(&mut self) {
        if let Some(o) = self.open.take() {
            // SAFETY: UI thread; detach the item so its buffers are released now.
            unsafe {
                o.player.pause();
                o.item.removeOutput(&o.output);
                o.player.replaceCurrentItemWithPlayerItem(None);
            }
        }
    }
}
