//! Write endpoints: post, edit, delete and mark read. All run at `Priority::User`.

use serde::Deserialize;

use crate::client::Retry;
use crate::types::Message;
use crate::{ApiError, Priority, WebClient, method};

/// The server's answer to `chat.postMessage`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PostedMessage {
    pub channel: String,
    pub ts: String,
    pub message: Message,
}

impl WebClient {
    /// Posts a message; `client_msg_id` lets the caller match the echo and makes retries safe.
    pub async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
        client_msg_id: &str,
    ) -> Result<PostedMessage, ApiError> {
        let mut params = vec![
            ("channel", channel),
            ("text", text),
            ("client_msg_id", client_msg_id),
        ];
        if let Some(t) = thread_ts {
            params.push(("thread_ts", t));
        }
        self.call_write(method::CHAT_POST_MESSAGE, &params, Priority::User)
            .await
    }

    pub async fn update_message(
        &self,
        channel: &str,
        ts: &str,
        text: &str,
    ) -> Result<(), ApiError> {
        self.call_ok(
            method::CHAT_UPDATE,
            &[("channel", channel), ("ts", ts), ("text", text)],
            Priority::User,
            Retry::Idempotent,
        )
        .await
    }

    pub async fn delete_message(&self, channel: &str, ts: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::CHAT_DELETE,
            &[("channel", channel), ("ts", ts)],
            Priority::User,
            Retry::Idempotent,
        )
        .await
    }

    /// `conversations.mark`: moves the read cursor to `ts`.
    pub async fn mark_read(&self, channel: &str, ts: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::CONVERSATIONS_MARK,
            &[("channel", channel), ("ts", ts)],
            Priority::User,
            Retry::Idempotent,
        )
        .await
    }
}
