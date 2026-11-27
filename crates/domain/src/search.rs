//! Search types shared by every provider.

use std::sync::Arc;

use crate::files::FileRef;
use crate::ids::{ConversationId, MessageTs, UserId};
use crate::model::Message;

/// One page of a cursor-paginated provider answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Opaque cursor for the next page; `None` when this was the last.
    pub next: Option<Arc<str>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Has {
    File,
    Link,
    Reaction,
    Pin,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchQuery {
    pub text: Arc<str>,
    pub conversations: Vec<ConversationId>,
    pub authors: Vec<UserId>,
    pub before: Option<jiff::civil::Date>,
    pub after: Option<jiff::civil::Date>,
    pub has: Vec<Has>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitSource {
    Local,
    Remote,
}

#[derive(Clone, Debug)]
pub struct MessageHit {
    pub message: Message,
    pub snippet: Arc<str>,
    pub source: HitSource,
}

#[derive(Clone, Debug)]
pub struct FileHit {
    pub file: FileRef,
    pub conversation: Option<ConversationId>,
    pub author: Option<UserId>,
    pub ts: Option<MessageTs>,
}

/// Messages around a target, ascending.
#[derive(Clone, Debug)]
pub struct AroundPage {
    pub messages: Vec<Message>,
    pub has_more_before: bool,
    pub has_more_after: bool,
}
