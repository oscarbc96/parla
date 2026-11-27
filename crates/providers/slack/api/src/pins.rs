//! Pins (`pins.*`). The endpoints are unverified against a capture.

use serde::Deserialize;

use crate::client::Retry;
use crate::{ApiError, Priority, WebClient, method};

/// One pinned message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinRaw {
    pub ts: String,
    pub by: Option<String>,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Item {
    r#type: String,
    created: i64,
    created_by: Option<String>,
    message: Option<Stamp>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Stamp {
    ts: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Listed {
    items: Vec<Item>,
}

impl WebClient {
    /// `pins.list`: the pinned messages of a conversation (file pins are skipped).
    pub async fn pins_list(&self, channel: &str) -> Result<Vec<PinRaw>, ApiError> {
        let r: Listed = self
            .call(
                method::PINS_LIST,
                &[("channel", channel)],
                Priority::Visible,
            )
            .await?;
        Ok(r.items
            .into_iter()
            .filter(|i| i.r#type == "message")
            .filter_map(|i| {
                let ts = i.message.map(|m| m.ts).filter(|t| !t.is_empty())?;
                Some(PinRaw {
                    ts,
                    by: i.created_by.filter(|u| !u.is_empty()),
                    at: i.created,
                })
            })
            .collect())
    }

    /// `pins.add`. `already_pinned` stays an `ApiError::Slack`.
    pub async fn pins_add(&self, channel: &str, ts: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::PINS_ADD,
            &[("channel", channel), ("timestamp", ts)],
            Priority::User,
            Retry::Write,
        )
        .await
    }

    /// `pins.remove`. `no_pin` stays an `ApiError::Slack`.
    pub async fn pins_remove(&self, channel: &str, ts: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::PINS_REMOVE,
            &[("channel", channel), ("timestamp", ts)],
            Priority::User,
            Retry::Write,
        )
        .await
    }
}
