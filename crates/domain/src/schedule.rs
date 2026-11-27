//! Scheduled messages: provider-side and local.

use std::sync::Arc;

use crate::files::Upload;
use crate::ids::{ConversationId, MessageTs};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScheduleId(pub Arc<str>);

/// A message the provider will post later.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledMessage {
    pub id: ScheduleId,
    pub conversation: ConversationId,
    pub thread: Option<MessageTs>,
    pub text: Arc<str>,
    /// Unix seconds.
    pub post_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalScheduleState {
    Pending,
    Sending,
    Failed(String),
}

/// A schedule Parla keeps and sends itself (provider cannot, or the message
/// carries files).
#[derive(Clone, Debug)]
pub struct LocalSchedule {
    pub id: ScheduleId,
    pub conversation: ConversationId,
    pub thread: Option<MessageTs>,
    pub text: Arc<str>,
    pub files: Vec<Upload>,
    /// Unix seconds.
    pub post_at: i64,
    pub created_ms: i64,
    pub state: LocalScheduleState,
}
