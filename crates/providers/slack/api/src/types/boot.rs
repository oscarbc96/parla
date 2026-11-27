use serde::Deserialize;

use super::Conversation;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BootSelf {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BootTeam {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub url: String,
    pub enterprise_id: Option<String>,
}

/// `client.userBoot` reduced to what the app needs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Boot {
    #[serde(rename = "self")]
    pub self_user: BootSelf,
    pub team: BootTeam,
    pub channels: Vec<Conversation>,
    pub ims: Vec<Conversation>,
    pub is_europe: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ChannelCount {
    pub id: String,
    pub last_read: String,
    pub latest: String,
    pub mention_count: u32,
    pub has_unreads: bool,
}

/// `client.counts`: per-conversation unread state.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Counts {
    pub channels: Vec<ChannelCount>,
    pub mpims: Vec<ChannelCount>,
    pub ims: Vec<ChannelCount>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ShouldReload {
    pub recommended_version: Option<String>,
}
