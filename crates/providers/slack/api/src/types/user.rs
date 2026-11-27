use serde::Deserialize;

use crate::page::{Paged, ResponseMetadata};
use crate::people::null_default;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Profile {
    #[serde(deserialize_with = "null_default")]
    pub display_name: String,
    #[serde(deserialize_with = "null_default")]
    pub real_name: String,
    pub image_48: Option<String>,
    pub image_72: Option<String>,
    pub image_192: Option<String>,
    pub image_512: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub status_text: String,
    /// `:name:` form, empty for none.
    #[serde(deserialize_with = "null_default")]
    pub status_emoji: String,
    /// Unix seconds, `0` for none.
    #[serde(deserialize_with = "null_default")]
    pub status_expiration: i64,
    /// Job title; empty for none.
    #[serde(deserialize_with = "null_default")]
    pub title: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct User {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub deleted: bool,
    #[serde(deserialize_with = "null_default")]
    pub is_bot: bool,
    pub tz: Option<String>,
    #[serde(deserialize_with = "null_default")]
    pub profile: Profile,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UsersPage {
    pub members: Vec<User>,
    pub response_metadata: Option<ResponseMetadata>,
}

impl Paged for UsersPage {
    fn next_cursor(&self) -> Option<&str> {
        self.response_metadata.as_ref()?.next_cursor.as_deref()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UserInfo {
    pub user: User,
}

/// `bots.info`: a bot's display name.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub deleted: bool,
    pub user_id: Option<String>,
    /// The bot's icons (`image_36`, `image_48`, `image_72`).
    #[serde(deserialize_with = "crate::people::null_default")]
    pub icons: BotIcons,
}

/// A bot's icon URLs; each is `None` when absent.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BotIcons {
    pub image_48: Option<String>,
    pub image_72: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BotInfo {
    pub bot: Bot,
}
