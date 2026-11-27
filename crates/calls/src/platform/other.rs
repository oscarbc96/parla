//! Windows and Linux: no call window until M7.

use crate::protocol::{CaptureState, MediaKind};

pub const WINDOW_SUPPORTED: bool = false;

pub fn hide_from_dock() {}

pub fn set_capture_muted(_wv: &wry::WebView, _kind: MediaKind, _muted: bool) -> bool {
    false
}

pub fn observe_capture(_wv: &wry::WebView, f: impl Fn(MediaKind, CaptureState) + 'static) {
    f(MediaKind::Microphone, CaptureState::Unsupported);
    f(MediaKind::Camera, CaptureState::Unsupported);
}

pub fn open_external(url: &url::Url) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer")
        .arg(url.as_str())
        .spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open")
        .arg(url.as_str())
        .spawn();
}
