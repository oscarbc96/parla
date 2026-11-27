//! Channel bookmarks (`bookmarks.*`). The endpoints are unverified against a capture.

use serde::Deserialize;

use crate::client::Retry;
use crate::{ApiError, Priority, WebClient, method};

/// One bookmark as the server reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkRaw {
    pub id: String,
    pub channel: String,
    pub title: String,
    pub link: String,
    /// `:shortcode:` form.
    pub emoji: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Wire {
    id: String,
    channel_id: String,
    title: String,
    link: String,
    emoji: Option<String>,
}

impl Wire {
    /// `None` without an id (it could not be removed later).
    fn into_raw(self) -> Option<BookmarkRaw> {
        if self.id.is_empty() {
            return None;
        }
        Some(BookmarkRaw {
            id: self.id,
            channel: self.channel_id,
            title: self.title,
            link: self.link,
            emoji: self.emoji.filter(|e| !e.is_empty()),
        })
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Listed {
    bookmarks: Vec<Wire>,
}

#[derive(Deserialize, Default)]
struct Added {
    bookmark: Option<Wire>,
}

impl WebClient {
    pub async fn bookmarks_list(&self, channel: &str) -> Result<Vec<BookmarkRaw>, ApiError> {
        let r: Listed = self
            .call(
                method::BOOKMARKS_LIST,
                &[("channel_id", channel)],
                Priority::Visible,
            )
            .await?;
        Ok(r.bookmarks.into_iter().filter_map(Wire::into_raw).collect())
    }

    /// `bookmarks.add` of a link. Never resent after a timeout or 5xx (it may have been
    /// stored); only a 429 is retried.
    pub async fn bookmarks_add(
        &self,
        channel: &str,
        title: &str,
        link: &str,
    ) -> Result<BookmarkRaw, ApiError> {
        let r: Added = self
            .call_write(
                method::BOOKMARKS_ADD,
                &[
                    ("channel_id", channel),
                    ("title", title),
                    ("type", "link"),
                    ("link", link),
                ],
                Priority::User,
            )
            .await?;
        r.bookmark
            .and_then(Wire::into_raw)
            .ok_or_else(|| ApiError::Slack {
                method: method::BOOKMARKS_ADD.name,
                error: "no_bookmark_in_response".into(),
                needed: None,
            })
    }

    pub async fn bookmarks_remove(&self, channel: &str, id: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::BOOKMARKS_REMOVE,
            &[("channel_id", channel), ("bookmark_id", id)],
            Priority::User,
            Retry::Write,
        )
        .await
    }
}
