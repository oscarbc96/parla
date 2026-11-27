//! The call helper: the process behind `parla call-window`.

mod core;
mod window;

pub use core::{CallWebview, HelperCore, HelperOut};
pub use window::run_helper;
