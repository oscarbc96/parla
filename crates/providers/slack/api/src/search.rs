//! Search: messages, files, people and channels (UNVERIFIED, see
//! docs/providers/slack/web-client/search.md). The `search.modules.*` family
//! takes a `module` name and a cursor.

use serde::Deserialize;

use crate::types::{Conversation, File, Message, User};
use crate::{ApiError, Method, Priority, WebClient, method};

/// One page of search results and the cursor for the next, when there is one.
#[derive(Debug, Clone)]
pub struct SearchResults<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

/// A matching message with the channel it was found in.
#[derive(Debug, Clone)]
pub struct SearchMessage {
    pub channel: String,
    /// `text` carries the match between U+E000 and U+E001 when `highlight` is on.
    pub message: Message,
}

/// A matching file with where and by whom it was shared, when known.
#[derive(Debug, Clone)]
pub struct SearchFile {
    pub file: File,
    pub channel: Option<String>,
    pub user: Option<String>,
    pub ts: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Pagination {
    next_cursor: Option<String>,
    /// The web client pages by number (1-based) and reports where it is.
    page: Option<u32>,
    page_count: Option<u32>,
}

impl Pagination {
    /// The next page: a cursor when the server gives one, else the next page
    /// number as a string (see [`WebClient::search`]).
    fn next(self) -> Option<String> {
        if let Some(c) = self.next_cursor.filter(|c| !c.is_empty()) {
            return Some(c);
        }
        let (page, count) = (self.page?, self.page_count?);
        (page < count).then(|| (page + 1).to_string())
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ChannelRef {
    id: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MessageGroup {
    channel: ChannelRef,
    messages: Vec<Message>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MessagesResp {
    items: Vec<MessageGroup>,
    pagination: Pagination,
}

#[derive(Deserialize)]
struct FileItem {
    file: File,
    #[serde(default)]
    channel: Option<ChannelRef>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    ts: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FilesResp {
    items: Vec<FileItem>,
    pagination: Pagination,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PeopleResp {
    items: Vec<User>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ChannelsResp {
    items: Vec<Conversation>,
    pagination: Pagination,
}

/// Most results the server returns per call; larger requests are clamped.
pub const MAX_COUNT: u32 = 100;

impl WebClient {
    async fn search<R: serde::de::DeserializeOwned>(
        &self,
        m: Method,
        module: &str,
        query: &str,
        cursor: Option<&str>,
        count: u32,
        extra: &[(&str, &str)],
    ) -> Result<R, ApiError> {
        let count = count.clamp(1, MAX_COUNT).to_string();
        let mut params = vec![
            ("module", module),
            ("query", query),
            ("count", count.as_str()),
        ];
        params.extend_from_slice(extra);
        // A number is a page of the web client's numbered paging; anything
        // else is an opaque cursor.
        if let Some(c) = cursor {
            let numbered = !c.is_empty() && c.chars().all(|ch| ch.is_ascii_digit());
            params.push((if numbered { "page" } else { "cursor" }, c));
        }
        self.call(m, &params, Priority::User).await
    }

    /// Message search, newest first, with match highlights.
    pub async fn search_messages(
        &self,
        query: &str,
        cursor: Option<&str>,
        count: u32,
    ) -> Result<SearchResults<SearchMessage>, ApiError> {
        let r: MessagesResp = self
            .search(
                method::SEARCH_MODULES_MESSAGES,
                "messages",
                query,
                cursor,
                count,
                &[
                    ("highlight", "true"),
                    ("sort", "timestamp"),
                    ("sort_dir", "desc"),
                ],
            )
            .await?;
        let items = r
            .items
            .into_iter()
            .filter(|g| !g.channel.id.is_empty())
            .flat_map(|g| {
                let channel = g.channel.id;
                g.messages
                    .into_iter()
                    .filter(|m| !m.ts.is_empty())
                    .map(move |message| SearchMessage {
                        channel: channel.clone(),
                        message,
                    })
            })
            .collect();
        Ok(SearchResults {
            items,
            next_cursor: r.pagination.next(),
        })
    }

    pub async fn search_files(
        &self,
        query: &str,
        cursor: Option<&str>,
        count: u32,
    ) -> Result<SearchResults<SearchFile>, ApiError> {
        let r: FilesResp = self
            .search(
                method::SEARCH_MODULES_FILES,
                "files",
                query,
                cursor,
                count,
                &[],
            )
            .await?;
        Ok(SearchResults {
            items: r
                .items
                .into_iter()
                .map(|i| SearchFile {
                    file: i.file,
                    channel: i.channel.map(|c| c.id).filter(|c| !c.is_empty()),
                    user: i.user,
                    ts: i.ts,
                })
                .collect(),
            next_cursor: r.pagination.next(),
        })
    }

    pub async fn search_people(&self, query: &str, count: u32) -> Result<Vec<User>, ApiError> {
        let r: PeopleResp = self
            .search(
                method::SEARCH_MODULES_PEOPLE,
                "people",
                query,
                None,
                count,
                &[],
            )
            .await?;
        Ok(r.items)
    }

    pub async fn search_channels(
        &self,
        query: &str,
        count: u32,
    ) -> Result<Vec<Conversation>, ApiError> {
        let r: ChannelsResp = self
            .search(
                method::SEARCH_MODULES_CHANNELS,
                "channels",
                query,
                None,
                count,
                &[],
            )
            .await?;
        Ok(r.items)
    }

    /// The channel browser: every channel the user can see, members or not,
    /// by name, with the params the web client's browser sends (2026-11-24
    /// capture). A blank query lists them all. Pages are numbered from 1
    /// (`cursor` "2", "3", ...).
    pub async fn browse_channels(
        &self,
        query: &str,
        cursor: Option<&str>,
        count: u32,
    ) -> Result<SearchResults<Conversation>, ApiError> {
        let r: ChannelsResp = self
            .search(
                method::SEARCH_MODULES_CHANNELS,
                "channels",
                query,
                Some(cursor.unwrap_or("1")),
                count,
                &[
                    ("sort", "recommended"),
                    ("channel_type", "all"),
                    ("exclude_my_channels", "0"),
                    ("search_only_my_channels", "false"),
                    ("search_context", "desktop_channel_browser"),
                ],
            )
            .await?;
        Ok(SearchResults {
            items: r.items,
            next_cursor: r.pagination.next(),
        })
    }
}
