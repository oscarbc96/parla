//! Add and remove reactions.

use crate::client::Retry;
use crate::{ApiError, Method, Priority, WebClient, method};

impl WebClient {
    async fn react(
        &self,
        m: Method,
        channel: &str,
        ts: &str,
        name: &str,
        benign: &str,
    ) -> Result<(), ApiError> {
        let r = self
            .call_ok(
                m,
                &[("channel", channel), ("timestamp", ts), ("name", name)],
                Priority::User,
                Retry::Idempotent,
            )
            .await;
        match r {
            Err(ApiError::Slack { error, .. }) if error == benign => Ok(()),
            other => other,
        }
    }

    /// `already_reacted` counts as success.
    pub async fn add_reaction(&self, channel: &str, ts: &str, name: &str) -> Result<(), ApiError> {
        self.react(method::REACTIONS_ADD, channel, ts, name, "already_reacted")
            .await
    }

    /// `no_reaction` counts as success.
    pub async fn remove_reaction(
        &self,
        channel: &str,
        ts: &str,
        name: &str,
    ) -> Result<(), ApiError> {
        self.react(method::REACTIONS_REMOVE, channel, ts, name, "no_reaction")
            .await
    }
}
