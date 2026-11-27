//! Rate limits and the confirmation queue cap. The limiter
//! lives in `write_confirm`, shared with plugins; this module re-exports it.

pub use write_confirm::limits::*;
