use serde::Deserialize;

use crate::page::{Paged, ResponseMetadata};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Conversation {
    pub id: String,
    pub name: Option<String>,
    pub is_channel: bool,
    pub is_group: bool,
    pub is_im: bool,
    pub is_mpim: bool,
    pub is_private: bool,
    pub is_archived: bool,
    pub is_member: bool,
    /// Direct and group conversations only: whether the user has it open in
    /// the sidebar. Absent means open (not verified against a capture).
    pub is_open: Option<bool>,
    pub user: Option<String>,
    /// Group conversations only: the members' ids (carried by `client.userBoot`).
    pub members: Option<Vec<String>>,
    pub updated: Option<i64>,
    pub topic: Option<Topic>,
    /// The description (`{value, creator, last_set}`, like the topic).
    pub purpose: Option<Topic>,
    /// `member_count` in `search.modules.channels` items.
    #[serde(alias = "member_count")]
    pub num_members: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Topic {
    pub value: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ConversationsPage {
    pub channels: Vec<Conversation>,
    pub response_metadata: Option<ResponseMetadata>,
}

impl Paged for ConversationsPage {
    fn next_cursor(&self) -> Option<&str> {
        self.response_metadata.as_ref()?.next_cursor.as_deref()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ConversationInfo {
    pub channel: Conversation,
}
