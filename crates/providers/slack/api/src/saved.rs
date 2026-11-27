//! Saved items (`saved.*`, the "Later" list). The endpoints are unverified against a capture.

use serde::Deserialize;

use crate::client::Retry;
use crate::{ApiError, Priority, WebClient, method};

/// One saved message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedRaw {
    pub channel: String,
    pub ts: String,
    /// Unix seconds.
    pub saved_at: i64,
    /// Unix seconds; `None` without a reminder (the server sends `0` or `null`).
    pub due: Option<i64>,
    pub done: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedPage {
    pub items: Vec<SavedRaw>,
    pub next: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Wire {
    item_type: String,
    item_id: String,
    ts: String,
    date_created: i64,
    date_due: Option<i64>,
    state: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Listed {
    saved_items: Vec<Wire>,
    response_metadata: Option<crate::ResponseMetadata>,
}

impl WebClient {
    /// One page of `saved.list`; `next` is `None` on the last page.
    pub async fn saved_list(&self, cursor: Option<&str>) -> Result<SavedPage, ApiError> {
        let mut params = vec![("filter", "saved"), ("limit", "50")];
        if let Some(c) = cursor {
            params.push(("cursor", c));
        }
        let r: Listed = self
            .call(method::SAVED_LIST, &params, Priority::Visible)
            .await?;
        Ok(SavedPage {
            items: r
                .saved_items
                .into_iter()
                .filter(|w| w.item_type == "message" && !w.item_id.is_empty() && !w.ts.is_empty())
                .map(|w| SavedRaw {
                    channel: w.item_id,
                    ts: w.ts,
                    saved_at: w.date_created,
                    due: w.date_due.filter(|d| *d > 0),
                    done: w.state == "completed",
                })
                .collect(),
            next: r
                .response_metadata
                .and_then(|m| m.next_cursor)
                .filter(|c| !c.is_empty()),
        })
    }

    pub async fn saved_add(&self, channel: &str, ts: &str) -> Result<(), ApiError> {
        self.saved_write(method::SAVED_ADD, channel, ts).await
    }

    pub async fn saved_remove(&self, channel: &str, ts: &str) -> Result<(), ApiError> {
        self.saved_write(method::SAVED_DELETE, channel, ts).await
    }

    async fn saved_write(&self, m: crate::Method, channel: &str, ts: &str) -> Result<(), ApiError> {
        self.call_ok(
            m,
            &[("item_type", "message"), ("item_id", channel), ("ts", ts)],
            Priority::User,
            Retry::Write,
        )
        .await
    }
}
