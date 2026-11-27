//! Tool result shapes. Every free-text field a remote person controls is
//! named `untrusted_*` so a client can tell data from instructions. No
//! provider names appear here: `provider` is the provider's display name.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceOut {
    /// Workspace key, `<provider>/<id>`.
    pub workspace: String,
    pub untrusted_name: String,
    /// Provider display name.
    pub provider: String,
    pub allow_writes: bool,
    pub confirm_writes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationOut {
    pub workspace: String,
    pub conversation: String,
    /// `channel`, `private`, `dm` or `group`.
    pub kind: String,
    pub untrusted_name: String,
    pub unread: bool,
    pub mentions: u32,
    pub latest: Option<String>,
    pub last_read: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuthorOut {
    pub user: String,
    pub untrusted_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MessageOut {
    pub workspace: String,
    pub conversation: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub reply_count: u32,
    pub author: Option<AuthorOut>,
    pub untrusted_text: String,
    /// The text was cut to the tool's size limit.
    pub truncated: bool,
    pub edited: bool,
    /// Reaction shortcode and count.
    pub reactions: Vec<(String, u32)>,
    /// File names, untrusted.
    pub files: Vec<String>,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageOut<T> {
    pub items: Vec<T>,
    /// Pass back unchanged to get the next page; absent on the last.
    pub next_cursor: Option<String>,
    /// Items were dropped to fit the result size cap; `next_cursor` continues.
    pub truncated: bool,
    /// Part of the page is missing because the provider read budget is spent;
    /// retry later for the rest.
    pub partial: bool,
}

/// A conversation with unread messages and its newest unread messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnreadOut {
    pub conversation: ConversationOut,
    /// Newest first.
    pub messages: Vec<MessageOut>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UserOut {
    pub workspace: String,
    pub user: String,
    pub untrusted_display_name: String,
    pub untrusted_real_name: String,
    pub is_bot: bool,
    pub deleted: bool,
    pub untrusted_status_text: Option<String>,
    /// Status emoji shortcode.
    pub status_emoji: Option<String>,
}

/// A message found by `search_messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchHitOut {
    pub workspace: String,
    pub conversation: String,
    pub untrusted_conversation_name: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub author: Option<AuthorOut>,
    /// The start of the message text, cut to a short length.
    pub untrusted_snippet: String,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScheduledOut {
    pub workspace: String,
    pub id: String,
    pub conversation: String,
    pub thread_ts: Option<String>,
    pub untrusted_text: String,
    /// Unix seconds.
    pub post_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SendOut {
    pub workspace: String,
    pub conversation: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReactionOut {
    pub workspace: String,
    pub conversation: String,
    pub ts: String,
    /// Shortcode that was added.
    pub emoji: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusOut {
    pub workspace: String,
    /// The status was cleared rather than set.
    pub cleared: bool,
}
