//! Channel admin: create, join, leave, archive, rename, topic, purpose,
//! invite and stars, as the web client sends them (2026-11-24 capture, see
//! docs/providers/slack/web-client/conversations.md "Channel admin"). All run
//! at `Priority::User` and are never retried blindly (`call_write`).

use serde::Deserialize;

use crate::types::Conversation;
use crate::{ApiError, Priority, WebClient, method};

#[derive(Deserialize)]
struct ChannelResp {
    #[serde(default)]
    channel: Conversation,
}

#[derive(Deserialize)]
struct Done {}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MembersResp {
    members: Vec<String>,
    response_metadata: Option<crate::ResponseMetadata>,
}

impl WebClient {
    /// `conversations.create`; `validate_name=true` makes the server refuse a
    /// taken or invalid name (`name_taken`, `invalid_name_*`).
    pub async fn create_channel(
        &self,
        name: &str,
        private: bool,
    ) -> Result<Conversation, ApiError> {
        let mut params = vec![("name", name), ("validate_name", "true")];
        if private {
            params.push(("is_private", "true"));
        }
        let r: ChannelResp = self
            .call_write(method::CONVERSATIONS_CREATE, &params, Priority::User)
            .await?;
        Ok(r.channel)
    }

    /// `conversations.validateName`: whether `name` could be used now.
    pub async fn validate_channel_name(&self, name: &str) -> Result<(), ApiError> {
        let _: Done = self
            .call(
                method::CONVERSATIONS_VALIDATE_NAME,
                &[("name", name)],
                Priority::User,
            )
            .await?;
        Ok(())
    }

    /// `conversations.join` as the web client sends it.
    pub async fn join_channel(&self, channel: &str) -> Result<Conversation, ApiError> {
        let r: ChannelResp = self
            .call_write(
                method::CONVERSATIONS_JOIN,
                &[("channel", channel), ("request_in_background", "true")],
                Priority::User,
            )
            .await?;
        Ok(r.channel)
    }

    pub async fn leave_channel(&self, channel: &str) -> Result<(), ApiError> {
        self.channel_write(method::CONVERSATIONS_LEAVE, &[("channel", channel)])
            .await
    }

    pub async fn archive_channel(&self, channel: &str) -> Result<(), ApiError> {
        self.channel_write(method::CONVERSATIONS_ARCHIVE, &[("channel", channel)])
            .await
    }

    pub async fn rename_channel(&self, channel: &str, name: &str) -> Result<(), ApiError> {
        self.channel_write(
            method::CONVERSATIONS_RENAME,
            &[("channel", channel), ("name", name), ("validate", "true")],
        )
        .await
    }

    pub async fn set_channel_topic(&self, channel: &str, topic: &str) -> Result<(), ApiError> {
        self.channel_write(
            method::CONVERSATIONS_SET_TOPIC,
            &[("channel", channel), ("topic", topic)],
        )
        .await
    }

    /// The description; `name_tagging=true` as the web client sends it.
    pub async fn set_channel_purpose(&self, channel: &str, purpose: &str) -> Result<(), ApiError> {
        self.channel_write(
            method::CONVERSATIONS_SET_PURPOSE,
            &[
                ("channel", channel),
                ("purpose", purpose),
                ("name_tagging", "true"),
            ],
        )
        .await
    }

    /// `conversations.invite` with comma-separated user ids (public Web API
    /// shape; not captured).
    pub async fn invite_to_channel(&self, channel: &str, users: &[&str]) -> Result<(), ApiError> {
        let users = users.join(",");
        self.channel_write(
            method::CONVERSATIONS_INVITE,
            &[("channel", channel), ("users", &users)],
        )
        .await
    }

    /// `stars.add` / `stars.remove` for a whole channel; a star that is
    /// already there (or already gone) counts as done.
    pub async fn star_channel(&self, channel: &str, starred: bool) -> Result<(), ApiError> {
        let m = if starred {
            method::STARS_ADD
        } else {
            method::STARS_REMOVE
        };
        match self.channel_write(m, &[("channel", channel)]).await {
            Err(ApiError::Slack { error, .. })
                if error == "already_starred" || error == "not_starred" =>
            {
                Ok(())
            }
            r => r,
        }
    }

    /// `conversations.members`, every page (public Web API shape; not
    /// captured).
    pub async fn channel_members(&self, channel: &str) -> Result<Vec<String>, ApiError> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = vec![("channel", channel), ("limit", "1000")];
            if let Some(c) = cursor.as_deref() {
                params.push(("cursor", c));
            }
            let r: MembersResp = self
                .call(method::CONVERSATIONS_MEMBERS, &params, Priority::User)
                .await?;
            out.extend(r.members);
            match r
                .response_metadata
                .and_then(|m| m.next_cursor)
                .filter(|c| !c.is_empty())
            {
                Some(c) => cursor = Some(c),
                None => return Ok(out),
            }
        }
    }

    async fn channel_write(
        &self,
        m: crate::Method,
        params: &[(&str, &str)],
    ) -> Result<(), ApiError> {
        let _: Done = self.call_write(m, params, Priority::User).await?;
        Ok(())
    }
}
