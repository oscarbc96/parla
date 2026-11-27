//! No decoder yet (M7): clips show the poster and "Open in browser".

use std::path::Path;

use super::{ClipError, Frame, VideoPlayer};

pub struct Unsupported;

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
