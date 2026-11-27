//! The Threads view and single-message fetch. `subscriptions.thread.getView` is unverified
//! against a capture; `message_get` uses the verified history and replies methods.

use serde::Deserialize;

use crate::types::Message;
use crate::{ApiError, Priority, WebClient, method};

/// One thread of the Threads view.
#[derive(Debug, Clone)]
pub struct InboxRaw {
    pub channel: String,
    pub root_ts: String,
    /// The newest reply (the root's `latest_reply`, else the newest unread reply, else the root).
    pub latest: String,
    pub unread_replies: Vec<Message>,
}

#[derive(Debug, Clone, Default)]
pub struct InboxPage {
    pub threads: Vec<InboxRaw>,
    pub next: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Root {
    channel: String,
    ts: String,
    latest_reply: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Wire {
    root_msg: Root,
    unread_replies: Vec<Message>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Viewed {
    threads: Vec<Wire>,
    response_metadata: Option<crate::ResponseMetadata>,
}

impl WebClient {
    /// One page of the Threads view (`subscriptions.thread.getView`).
    pub async fn inbox_list(&self, cursor: Option<&str>) -> Result<InboxPage, ApiError> {
        let mut params = vec![("limit", "20")];
        if let Some(c) = cursor {
            params.push(("cursor", c));
        }
        let r: Viewed = self
            .call(
                method::SUBSCRIPTIONS_THREAD_GET_VIEW,
                &params,
                Priority::Visible,
            )
            .await?;
        Ok(InboxPage {
            threads: r
                .threads
                .into_iter()
                .filter(|w| !w.root_msg.channel.is_empty() && !w.root_msg.ts.is_empty())
                .map(|w| {
                    let latest = w
                        .root_msg
                        .latest_reply
                        .filter(|l| !l.is_empty())
                        .or_else(|| w.unread_replies.iter().map(|m| m.ts.clone()).max())
                        .unwrap_or_else(|| w.root_msg.ts.clone());
                    InboxRaw {
                        channel: w.root_msg.channel,
                        root_ts: w.root_msg.ts,
                        latest,
                        unread_replies: w.unread_replies,
                    }
                })
                .collect(),
            next: r
                .response_metadata
                .and_then(|m| m.next_cursor)
                .filter(|c| !c.is_empty()),
        })
    }

    /// One message by timestamp: `conversations.history` with `latest=ts`, `inclusive=true`,
    /// `limit=1` answers a root; anything else (a reply is not in history, so an older root
    /// comes back) goes to `conversations.replies`. `None` when the message is gone or not
    /// visible (`message_not_found`, `thread_not_found`, `channel_not_found`).
    pub async fn message_get(&self, channel: &str, ts: &str) -> Result<Option<Message>, ApiError> {
        let gone = |e: &ApiError| {
            matches!(e, ApiError::Slack { error, .. } if matches!(
                error.as_str(),
                "thread_not_found" | "message_not_found" | "channel_not_found"
            ))
        };
        let hist: crate::types::HistoryPage = match self
            .call(
                method::CONVERSATIONS_HISTORY,
                &[
                    ("channel", channel),
                    ("latest", ts),
                    ("inclusive", "true"),
                    ("limit", "1"),
                ],
                Priority::Visible,
            )
            .await
        {
            Ok(h) => h,
            Err(e) if gone(&e) => return Ok(None),
            Err(e) => return Err(e),
        };
        if let Some(m) = hist.messages.into_iter().find(|m| m.ts == ts) {
            return Ok(Some(m));
        }
        match self
            .call::<crate::types::HistoryPage>(
                method::CONVERSATIONS_REPLIES,
                &[
                    ("channel", channel),
                    ("ts", ts),
                    ("inclusive", "true"),
                    ("limit", "1"),
                ],
                Priority::Visible,
            )
            .await
        {
            Ok(p) => Ok(p.messages.into_iter().find(|m| m.ts == ts)),
            Err(e) if gone(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
