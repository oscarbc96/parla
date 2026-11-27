//! Bundled test fonts (SIL OFL 1.1, see `fonts/*LICENSE*` and `fonts/*OFL*`).
//! Test-only; never packaged.

use anyhow::{Result, bail};
use gpui::HeadlessAppContext;

/// Inter 4.1 Italic: the face `*italic*` runs select (same release as Regular).
pub const INTER_ITALIC: &[u8] = include_bytes!("../fonts/Inter-Italic.ttf");
pub const INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
/// Noto Emoji (monochrome, SIL OFL 1.1) cut down to the emoji the visual suites
/// show, so emoji do not render as missing-glyph boxes.
pub const NOTO_EMOJI_SUBSET: &[u8] = include_bytes!("../fonts/NotoEmoji-Regular-subset.ttf");
pub const JETBRAINS_MONO_REGULAR: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");

/// Hard failure instead of GPUI's silent fallback for a missing family.
pub fn require_family(cx: &HeadlessAppContext, family: &str) -> Result<()> {
    if cx
        .text_system()
        .all_font_names()
        .iter()
        .any(|n| n == family)
    {
        Ok(())
    } else {
        bail!("font family {family:?} is not available in the text system")
    }
}
