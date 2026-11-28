//! Test-only UI harness: deterministic offscreen app, image capture and scenarios.

mod baseline;
mod compare;
mod fonts;
mod harness;
pub mod scenario;
pub mod source;
pub mod stage;
mod text_system;
mod variant;

pub use baseline::{Config, assert_matches, baseline_rel_path, check, encode_optimised};
pub use compare::{Comparison, SizeMismatch, Tolerance, compare};
pub use fonts::{
    INTER_BOLD, INTER_ITALIC, INTER_MEDIUM, INTER_REGULAR, INTER_SEMIBOLD, JETBRAINS_MONO_REGULAR,
    NOTO_EMOJI_SUBSET, require_family,
};
pub use harness::{LOGICAL_H, LOGICAL_W, UiHarness, text_system};
pub use text_system::{ParleyTextSystem, TestTextSystem};
pub use variant::{Appearance, Os, Variant};
