//! Media pipeline: fetch through a [`MediaFetcher`], decode at display size,
//! keep a bounded memory LRU plus a capped plain-file disk cache, and gate GIF
//! playback on visibility and focus.

pub mod anim;
pub mod cache;
pub mod decode;
mod element;
pub mod viewer;

use domain::ImageRef;
use futures::future::BoxFuture;

pub use anim::{Animation, should_play};
pub use cache::{
    DISK_CAP_BYTES, DiskBudget, FailureHook, ImageCache, ImageState, SuccessHook, key_hash,
    prune_disk_cache,
};
pub use decode::{Decoded, decode};
pub use element::{
    MediaImage, animation_for, emoji_hook, emoji_hook_dimmed, image_hook, image_hook_dimmed,
};
pub use viewer::{
    VIEWER_DECODE_MAX_BYTES, VIEWER_DECODE_MAX_PIXELS, VIEWER_KEEP, VIEWER_MAX_PIXELS,
    VIEWER_MAX_PX, VIEWER_THUMBS, ViewerCache, ViewerImage, ViewerState, clipboard_image,
    decode_viewer,
};

/// Largest image fetched (10 MB).
pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    TooLarge,
    Refused,
    Network(String),
    Decode,
}

/// Fetches the bytes of a remote image. Implemented by the app so that
/// `ImageRef::Provider` goes through the owning session and `ImageRef::Web`
/// through a credential-free client.
pub trait MediaFetcher: Send + Sync {
    fn fetch(
        &self,
        src: &ImageRef,
        max_bytes: u64,
    ) -> BoxFuture<'static, Result<Vec<u8>, MediaError>>;
}

impl MediaError {
    /// A short fixed word for the log: never a URL, id or name.
    pub fn kind(&self) -> &'static str {
        match self {
            MediaError::TooLarge => "too_large",
            MediaError::Refused => "refused",
            MediaError::Network(_) => "network",
            MediaError::Decode => "decode",
        }
    }
}
