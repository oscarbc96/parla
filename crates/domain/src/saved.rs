//! Pins, bookmarks, saved items and the thread inbox.

use std::sync::Arc;

use crate::ids::{ConversationId, MessageId, MessageTs};
use crate::rich::EmojiRef;

#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub message: MessageId,
    pub by: Option<crate::ids::UserId>,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BookmarkId(pub Arc<str>);

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub id: BookmarkId,
    pub conversation: ConversationId,
    pub title: Arc<str>,
    pub link: Arc<str>,
    pub emoji: Option<EmojiRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SavedItem {
    pub message: MessageId,
    /// Unix seconds.
    pub saved_at: i64,
    pub due: Option<i64>,
    pub done: bool,
}

/// One thread in the aggregate Threads view.
#[derive(Clone, Debug, PartialEq)]
pub struct InboxThread {
    pub root: MessageId,
    pub unread_replies: u32,
    pub mentions: u32,
    pub latest: MessageTs,
}
