//! Huddles: live audio rooms attached to a conversation.

use std::sync::Arc;

use crate::ids::{ConversationId, UserId};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HuddleId(pub Arc<str>);

#[derive(Clone, Debug, PartialEq)]
pub struct Huddle {
    pub id: HuddleId,
    pub conversation: ConversationId,
    pub participants: Arc<[UserId]>,
    /// Unix seconds.
    pub started: i64,
    pub created_by: Option<UserId>,
    pub is_dm: bool,
}
