use serde::Deserialize;
use serde_json::value::RawValue;

use crate::page::{Paged, ResponseMetadata};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Edited {
    pub user: Option<String>,
    pub ts: String,
}

/// The bot a message was posted by, as carried on the message itself.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BotProfile {
    pub id: Option<String>,
    pub name: Option<String>,
    /// The bot's icons (`image_48`, `image_72`), when the message carries them.
    #[serde(deserialize_with = "crate::people::null_default")]
    pub icons: super::user::BotIcons,
}

impl Message {
    /// The bot id and its name when the message carries both
    /// (`bot_profile.name`, else `username`).
    pub fn bot_identity(&self) -> Option<(&str, &str)> {
        let id = self.bot_id.as_deref()?;
        let name = self
            .bot_profile
            .as_ref()
            .and_then(|b| b.name.as_deref())
            .or(self.username.as_deref())
            .filter(|n| !n.is_empty())?;
        Some((id, name))
    }
}

/// A message as returned by history and replies, and as pushed on the socket.
/// Rich parts stay raw until M2 needs them.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Message {
    pub r#type: String,
    pub subtype: Option<String>,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub user: Option<String>,
    pub bot_id: Option<String>,
    /// Display name some bots and integrations post under.
    pub username: Option<String>,
    pub bot_profile: Option<BotProfile>,
    pub text: String,
    pub edited: Option<Edited>,
    pub reply_count: Option<u32>,
    pub latest_reply: Option<String>,
    pub reply_users: Option<Vec<String>>,
    pub reply_users_count: Option<u32>,
    pub client_msg_id: Option<String>,
    pub blocks: Option<Box<RawValue>>,
    pub files: Option<Box<RawValue>>,
    pub attachments: Option<Box<RawValue>>,
    pub reactions: Option<Box<RawValue>>,
    /// Conversations the message is pinned in (UNVERIFIED: no capture).
    pub pinned_to: Option<Vec<String>>,
    /// Saved for later (`is_starred`; UNVERIFIED: no capture).
    pub is_starred: Option<bool>,
    /// Huddle room of a `huddle_thread` message.
    pub room: Option<Box<RawValue>>,
}

impl Message {
    /// The raw `files` as typed values; entries that do not decode are skipped.
    pub fn files_typed(&self) -> Vec<super::file::File> {
        self.files
            .as_ref()
            .and_then(|r| serde_json::from_str::<Vec<serde_json::Value>>(r.get()).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect()
    }

    /// The raw `reactions` as typed values; malformed or absent gives empty.
    pub fn reactions_typed(&self) -> Vec<super::reaction::ReactionRaw> {
        self.reactions
            .as_ref()
            .and_then(|r| serde_json::from_str(r.get()).ok())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HistoryPage {
    pub messages: Vec<Message>,
    pub has_more: bool,
    pub response_metadata: Option<ResponseMetadata>,
}

impl Paged for HistoryPage {
    fn next_cursor(&self) -> Option<&str> {
        self.response_metadata.as_ref()?.next_cursor.as_deref()
    }
}
