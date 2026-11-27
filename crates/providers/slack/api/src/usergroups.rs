//! User groups (`usergroups.list`). Shaped after the public method; the web
//! client's own call is not captured yet. Every field tolerates absence.

use serde::Deserialize;
use serde_json::Value;

use crate::people::null_default;
use crate::{ApiError, Priority, WebClient, method};

/// One live user group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserGroupRaw {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub user_count: u32,
    /// Member ids (`include_users`); may be empty when the server leaves them out.
    pub users: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Group {
    #[serde(deserialize_with = "null_default")]
    id: String,
    #[serde(deserialize_with = "null_default")]
    handle: String,
    #[serde(deserialize_with = "null_default")]
    name: String,
    date_delete: Value,
    user_count: Value,
    #[serde(deserialize_with = "null_default")]
    users: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Listed {
    #[serde(deserialize_with = "null_default")]
    usergroups: Vec<Group>,
}

/// A number or a numeric string, else 0.
fn number(v: &Value) -> u64 {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}

impl WebClient {
    /// `usergroups.list`: the workspace's live groups with their member count and ids.
    pub async fn usergroups_list(&self) -> Result<Vec<UserGroupRaw>, ApiError> {
        let r: Listed = self
            .call(
                method::USERGROUPS_LIST,
                &[
                    ("include_count", "true"),
                    ("include_users", "true"),
                    ("include_disabled", "false"),
                ],
                Priority::Background,
            )
            .await?;
        Ok(r.usergroups
            .into_iter()
            .filter(|g| !g.id.is_empty() && !g.handle.is_empty() && number(&g.date_delete) == 0)
            .map(|g| UserGroupRaw {
                user_count: u32::try_from(number(&g.user_count))
                    .unwrap_or(u32::MAX)
                    .max(g.users.len().try_into().unwrap_or(u32::MAX)),
                id: g.id,
                handle: g.handle,
                name: g.name,
                users: g.users,
            })
            .collect())
    }
}
