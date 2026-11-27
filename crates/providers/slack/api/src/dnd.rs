//! Do not disturb (`dnd.*`). Unverified against a capture; fields tolerate `null` and absence.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::people::null_default;
use crate::{ApiError, Priority, WebClient, method};

/// Most users one `dnd.teamInfo` call is asked about.
pub const DND_TEAM_BATCH: usize = 50;

/// One user's DND state as the server reports it (unix seconds, `0` for none).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct DndRaw {
    #[serde(deserialize_with = "null_default")]
    pub dnd_enabled: bool,
    #[serde(deserialize_with = "null_default")]
    pub next_dnd_start_ts: i64,
    #[serde(deserialize_with = "null_default")]
    pub next_dnd_end_ts: i64,
    #[serde(deserialize_with = "null_default")]
    pub snooze_enabled: bool,
    #[serde(deserialize_with = "null_default")]
    pub snooze_endtime: i64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TeamInfo {
    #[serde(deserialize_with = "null_default")]
    users: BTreeMap<String, Option<DndRaw>>,
}

impl WebClient {
    /// `dnd.info` for the signed-in user.
    pub async fn dnd_info(&self) -> Result<DndRaw, ApiError> {
        self.call(method::DND_INFO, &[], Priority::Visible).await
    }

    /// `dnd.teamInfo` for other users, in batches of [`DND_TEAM_BATCH`]. Empty and repeated ids
    /// are not sent; empty ids and `null` entries in the answer are dropped. Sorted by id.
    pub async fn dnd_team_info(&self, users: &[String]) -> Result<Vec<(String, DndRaw)>, ApiError> {
        let mut ids: Vec<&str> = Vec::new();
        for u in users {
            if !u.is_empty() && !ids.contains(&u.as_str()) {
                ids.push(u);
            }
        }
        let mut out: BTreeMap<String, DndRaw> = BTreeMap::new();
        for batch in ids.chunks(DND_TEAM_BATCH) {
            let joined = batch.join(",");
            let r: TeamInfo = self
                .call(
                    method::DND_TEAM_INFO,
                    &[("users", joined.as_str())],
                    Priority::Visible,
                )
                .await?;
            for (u, d) in r.users {
                if let (false, Some(d)) = (u.is_empty(), d) {
                    out.insert(u, d);
                }
            }
        }
        Ok(out.into_iter().collect())
    }

    /// `dnd.setSnooze`; the answer carries the snooze fields.
    pub async fn dnd_set_snooze(&self, minutes: u32) -> Result<DndRaw, ApiError> {
        let m = minutes.to_string();
        self.call_write(
            method::DND_SET_SNOOZE,
            &[("num_minutes", m.as_str())],
            Priority::User,
        )
        .await
    }

    /// `dnd.endSnooze`. `snooze_not_active` stays an `ApiError::Slack`.
    pub async fn dnd_end_snooze(&self) -> Result<DndRaw, ApiError> {
        self.call_write(method::DND_END_SNOOZE, &[], Priority::User)
            .await
    }
}
