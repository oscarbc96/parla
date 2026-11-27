use std::sync::Arc;

use crate::ids::UserId;
use crate::rich::{EmojiRef, ImageRef};

#[derive(Clone, Debug, PartialEq)]
pub struct Reaction {
    pub emoji: EmojiRef,
    pub count: u32,
    pub users: Arc<[UserId]>,
    pub me: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomEmoji {
    pub shortcode: Arc<str>,
    pub image: ImageRef,
}
