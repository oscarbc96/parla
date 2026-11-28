//! Registers the bundled Inter faces with the app's text system.
//!
//! It runs at start-up, before the first window, so the default UI font
//! resolves to its design weights (400/500/600/700) on a machine without
//! Inter installed.

use std::borrow::Cow;

use gpui::App;

/// Registers [`themes::fonts::INTER_FACES`].
///
/// A failure is logged, not fatal: the text system then falls back to the
/// system UI font.
pub fn install(cx: &mut App) {
    let fonts = themes::fonts::INTER_FACES
        .iter()
        .map(|bytes| Cow::Borrowed(*bytes))
        .collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        log::error!("could not load the bundled Inter faces: {error:#}");
    }
}
