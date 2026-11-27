use crate::ids::{ConversationId, MessageId, MessageTs};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationCounts {
    pub conversation: ConversationId,
    pub latest: Option<MessageTs>,
    pub last_read: Option<MessageTs>,
    pub unread: bool,
    pub mentions: u32,
}

/// Messages strictly between `after` and `before` are missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub conversation: ConversationId,
    pub after: MessageTs,
    pub before: MessageTs,
}

impl Gap {
    /// The pieces of this gap left after `closed` was filled: the gap whole
    /// when they do not overlap, nothing when `closed` covers it.
    pub fn subtract(&self, closed: &Gap) -> Vec<Gap> {
        if !(self.after < closed.before && closed.after < self.before) {
            return vec![self.clone()];
        }
        let mut out = Vec::with_capacity(2);
        if self.after < closed.after {
            out.push(Gap {
                conversation: self.conversation.clone(),
                after: self.after.clone(),
                before: closed.after.clone(),
            });
        }
        if closed.before < self.before {
            out.push(Gap {
                conversation: self.conversation.clone(),
                after: closed.before.clone(),
                before: self.before.clone(),
            });
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub id: MessageId,
    pub snippet: Arc<str>,
    pub rank: f64,
}
