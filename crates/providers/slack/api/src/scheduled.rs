//! Server-side scheduled messages (`chat.scheduleMessage` and friends). All run at
//! `Priority::User`; the endpoints are unverified against a capture.

use serde::Deserialize;

use crate::client::Retry;
use crate::{ApiError, Priority, WebClient, method};

/// One scheduled message as the server reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ScheduledRaw {
    pub id: String,
    pub channel_id: String,
    pub post_at: i64,
    #[serde(deserialize_with = "null_default")]
    pub text: String,
    pub thread_ts: Option<String>,
}

/// `null` counts as the default (the server may send `"text": null`).
fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Scheduled {
    channel: String,
    scheduled_message_id: String,
    post_at: i64,
    message: ScheduledBody,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ScheduledBody {
    #[serde(deserialize_with = "null_default")]
    text: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ScheduledList {
    #[serde(deserialize_with = "null_default")]
    scheduled_messages: Vec<ScheduledRaw>,
    #[serde(deserialize_with = "null_default")]
    response_metadata: crate::ResponseMetadata,
}

impl WebClient {
    /// `chat.scheduleMessage`. `time_too_far` and `time_in_past` surface as `ApiError::Slack`.
    pub async fn schedule_message(
        &self,
        channel: &str,
        text: &str,
        post_at: i64,
        thread_ts: Option<&str>,
    ) -> Result<ScheduledRaw, ApiError> {
        let post_at_s = post_at.to_string();
        let mut params = vec![
            ("channel", channel),
            ("text", text),
            ("post_at", post_at_s.as_str()),
        ];
        if let Some(t) = thread_ts {
            params.push(("thread_ts", t));
        }
        let r: Scheduled = self
            .call_write(method::CHAT_SCHEDULE_MESSAGE, &params, Priority::User)
            .await?;
        Ok(ScheduledRaw {
            id: r.scheduled_message_id,
            channel_id: if r.channel.is_empty() {
                channel.to_string()
            } else {
                r.channel
            },
            post_at: if r.post_at == 0 { post_at } else { r.post_at },
            text: if r.message.text.is_empty() {
                text.to_string()
            } else {
                r.message.text
            },
            thread_ts: thread_ts.map(str::to_string),
        })
    }

    /// One page of `chat.scheduledMessages.list` and the next cursor (`None` when done).
    pub async fn scheduled_list(
        &self,
        cursor: Option<&str>,
    ) -> Result<(Vec<ScheduledRaw>, Option<String>), ApiError> {
        let mut params = Vec::new();
        if let Some(c) = cursor {
            params.push(("cursor", c));
        }
        let r: ScheduledList = self
            .call(
                method::CHAT_SCHEDULED_MESSAGES_LIST,
                &params,
                Priority::User,
            )
            .await?;
        let next = r.response_metadata.next_cursor.filter(|c| !c.is_empty());
        Ok((r.scheduled_messages, next))
    }

    /// `chat.deleteScheduledMessage`. `invalid_scheduled_message_id` (already sent or gone) stays an
    /// `ApiError::Slack`; the provider maps it to a not-allowed error so a reschedule does not
    /// post a second copy.
    pub async fn delete_scheduled(&self, channel: &str, id: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::CHAT_DELETE_SCHEDULED_MESSAGE,
            &[("channel", channel), ("scheduled_message_id", id)],
            Priority::User,
            Retry::Write,
        )
        .await
    }
}
