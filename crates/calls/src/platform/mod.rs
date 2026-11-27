//! Platform layer of the call window. Every `cfg` lives here.

use crate::protocol::{CaptureState, MediaKind};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
mod other;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(target_os = "macos"))]
use other as imp;

/// Whether the call window works here. macOS only in v0.1 (plan Decision 9);
/// Windows and Linux keep the browser rung until M7.
pub fn window_supported() -> bool {
    imp::WINDOW_SUPPORTED
}

/// Hides the process from the Dock (accessory activation policy), so Parla
/// shows one icon while a call runs.
pub fn hide_from_dock() {
    imp::hide_from_dock();
}

/// Mutes or unmutes the webview's capture device through the platform, never
/// the page. `false` when the platform cannot.
pub fn set_capture_muted(wv: &wry::WebView, kind: MediaKind, muted: bool) -> bool {
    imp::set_capture_muted(wv, kind, muted)
}

/// Calls `f` when the webview's capture state changes (and once with the
/// current state). Elsewhere it reports `Unsupported` once.
pub fn observe_capture(wv: &wry::WebView, f: impl Fn(MediaKind, CaptureState) + 'static) {
    imp::observe_capture(wv, f);
}

/// Hands `url` to the default browser or mail client.
pub fn open_external(url: &url::Url) {
    imp::open_external(url);
}
