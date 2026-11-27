//! Custom emoji.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{ApiError, Priority, WebClient, method};

#[derive(Deserialize, Default)]
#[serde(default)]
struct EmojiList {
    emoji: BTreeMap<String, String>,
}

impl WebClient {
    /// Name to image URL, or `alias:<name>` for an alias.
    pub async fn emoji_list(&self) -> Result<BTreeMap<String, String>, ApiError> {
        let r: EmojiList = self
            .call(method::EMOJI_LIST, &[], Priority::Background)
            .await?;
        Ok(r.emoji)
    }
}
