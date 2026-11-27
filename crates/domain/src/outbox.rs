use std::sync::Arc;

use crate::ids::{ClientMsgId, ConversationId, MessageTs};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboxState {
    Queued,
    Sending,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxItem {
    pub id: ClientMsgId,
    pub conversation: ConversationId,
    pub thread: Option<MessageTs>,
    pub broadcast: bool,
    pub text: Arc<str>,
    pub created_ms: i64,
    pub attempts: u32,
    pub state: OutboxState,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DraftKey {
    pub conversation: ConversationId,
    pub thread: Option<MessageTs>,
}
