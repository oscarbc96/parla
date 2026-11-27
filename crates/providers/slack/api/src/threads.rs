//! Thread replies, threaded posts, thread read marks and thread follow.

use serde::Deserialize;

use crate::client::Retry;
use crate::types::HistoryPage;
use crate::writes::PostedMessage;
use crate::{ApiError, Priority, WebClient, method};

impl WebClient {
    /// One page of a thread, root first. `oldest` is exclusive; without it the root is included.
    pub async fn conversations_replies(
        &self,
        channel: &str,
        ts: &str,
        oldest: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
        prio: Priority,
    ) -> Result<HistoryPage, ApiError> {
        let limit = limit.to_string();
        let mut params = vec![
            ("channel", channel),
            ("ts", ts),
            ("limit", limit.as_str()),
            ("inclusive", if oldest.is_none() { "true" } else { "false" }),
        ];
        if let Some(o) = oldest {
            params.push(("oldest", o));
        }
        if let Some(c) = cursor {
            params.push(("cursor", c));
        }
        self.call(method::CONVERSATIONS_REPLIES, &params, prio)
            .await
    }

    /// Posts into a thread; `reply_broadcast=true` is sent only when `broadcast`.
    pub async fn post_reply(
        &self,
        channel: &str,
        thread_ts: &str,
        text: &str,
        broadcast: bool,
        client_msg_id: &str,
    ) -> Result<PostedMessage, ApiError> {
        let mut params = vec![
            ("channel", channel),
            ("text", text),
            ("client_msg_id", client_msg_id),
            ("thread_ts", thread_ts),
        ];
        if broadcast {
            params.push(("reply_broadcast", "true"));
        }
        self.call_write(method::CHAT_POST_MESSAGE, &params, Priority::User)
            .await
    }

    /// `subscriptions.thread.mark`: moves the thread read cursor to `ts`.
    pub async fn mark_thread(
        &self,
        channel: &str,
        thread_ts: &str,
        ts: &str,
    ) -> Result<(), ApiError> {
        self.call_ok(
            method::SUBSCRIPTIONS_THREAD_MARK,
            &[("channel", channel), ("thread_ts", thread_ts), ("ts", ts)],
            Priority::User,
            Retry::Idempotent,
        )
        .await
    }

    /// `subscriptions.thread.get`: the user's read cursor in the thread when they follow it,
    /// `None` when they do not (`subscriptions: []`).
    pub async fn thread_subscription(
        &self,
        channel: &str,
        thread_ts: &str,
    ) -> Result<Option<String>, ApiError> {
        let r: ThreadSubscriptions = self
            .call(
                method::SUBSCRIPTIONS_THREAD_GET,
                &[("channel", channel), ("thread_ts", thread_ts)],
                Priority::User,
            )
            .await?;
        Ok(r.subscriptions.into_iter().find_map(|s| s.last_read))
    }

    /// `subscriptions.thread.add`: follows the thread; `last_read` is the read cursor.
    pub async fn follow_thread(
        &self,
        channel: &str,
        thread_ts: &str,
        last_read: &str,
    ) -> Result<(), ApiError> {
        self.thread_subscription_write(
            method::SUBSCRIPTIONS_THREAD_ADD,
            channel,
            thread_ts,
            last_read,
        )
        .await
    }

    /// `subscriptions.thread.remove`: unfollows the thread.
    pub async fn unfollow_thread(
        &self,
        channel: &str,
        thread_ts: &str,
        last_read: &str,
    ) -> Result<(), ApiError> {
        self.thread_subscription_write(
            method::SUBSCRIPTIONS_THREAD_REMOVE,
            channel,
            thread_ts,
            last_read,
        )
        .await
    }

    async fn thread_subscription_write(
        &self,
        m: method::Method,
        channel: &str,
        thread_ts: &str,
        last_read: &str,
    ) -> Result<(), ApiError> {
        self.call_ok(
            m,
            &[
                ("channel", channel),
                ("thread_ts", thread_ts),
                ("last_read", last_read),
            ],
            Priority::User,
            Retry::Idempotent,
        )
        .await
    }
}

#[derive(Deserialize)]
struct ThreadSubscriptions {
    #[serde(default)]
    subscriptions: Vec<ThreadSubscription>,
}

#[derive(Deserialize)]
struct ThreadSubscription {
    last_read: Option<String>,
}
