//! `team.info`: workspace name, icon and Enterprise Grid organisation.

use serde::Deserialize;

use crate::{ApiError, Priority, WebClient, method};

/// What the rail needs about a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamInfo {
    pub id: String,
    pub name: String,
    /// URL of the largest custom icon; `None` when the team has none.
    pub icon: Option<String>,
    pub enterprise_id: Option<String>,
    pub enterprise_name: Option<String>,
}

#[derive(Deserialize)]
struct Envelope {
    team: Team,
}

#[derive(Deserialize)]
struct Team {
    id: String,
    name: String,
    #[serde(default)]
    icon: Option<Icon>,
    #[serde(default)]
    enterprise_id: Option<String>,
    #[serde(default)]
    enterprise_name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Icon {
    image_default: bool,
    image_132: Option<String>,
    image_102: Option<String>,
    image_88: Option<String>,
    image_68: Option<String>,
    image_44: Option<String>,
    image_34: Option<String>,
}

impl Icon {
    pub(crate) fn best(self) -> Option<String> {
        if self.image_default {
            return None;
        }
        self.image_132
            .or(self.image_102)
            .or(self.image_88)
            .or(self.image_68)
            .or(self.image_44)
            .or(self.image_34)
            .filter(|u| u.starts_with("https://"))
    }
}

impl WebClient {
    /// One attempt per workspace start (no retry); refreshes the cached name, icon and org.
    pub async fn team_info(&self) -> Result<TeamInfo, ApiError> {
        let e: Envelope = self
            .call_once(method::TEAM_INFO, &[], Priority::Background)
            .await?;
        let t = e.team;
        Ok(TeamInfo {
            id: t.id,
            name: t.name,
            icon: t.icon.and_then(Icon::best),
            enterprise_id: t.enterprise_id.filter(|s| !s.is_empty()),
            enterprise_name: t.enterprise_name.filter(|s| !s.is_empty()),
        })
    }
}
