//! Workspace boot, unread counts and the web build version.

use crate::types::boot::{Boot, Counts, ShouldReload};
use crate::{ApiError, Priority, WebClient, method};

impl WebClient {
    /// `client.userBoot`: the user, team and conversation lists in one call.
    pub async fn user_boot(&self) -> Result<Boot, ApiError> {
        self.call(
            method::CLIENT_USER_BOOT,
            &[("min_channel_updated", "0"), ("_x_reason", "initial-data")],
            Priority::User,
        )
        .await
    }

    /// `client.counts`: unread and mention counts, same params as the web client.
    pub async fn counts(&self) -> Result<Counts, ApiError> {
        self.call(
            method::CLIENT_COUNTS,
            &[
                ("thread_counts_by_channel", "true"),
                ("org_wide_aware", "true"),
                ("include_file_channels", "true"),
                ("include_all_unreads", "true"),
                ("_x_reason", "counts-initial"),
            ],
            Priority::Visible,
        )
        .await
    }

    /// `client.shouldReload`'s recommended build version (`_x_version_ts`, socket
    /// `agent_version`); `None` if the server sends none.
    pub async fn build_version(&self) -> Result<Option<String>, ApiError> {
        let r: ShouldReload = self
            .call(
                method::CLIENT_SHOULD_RELOAD,
                &[("_x_reason", "should-reload")],
                Priority::User,
            )
            .await?;
        Ok(r.recommended_version.filter(|v| !v.is_empty()))
    }
}
