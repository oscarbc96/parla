//! Profiles, own status and presence (`users.profile.*`, `users.info`, `users.setPresence`).
//! All endpoints are unverified against a capture; every field tolerates `null` and absence.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::client::Retry;
use crate::{ApiError, Priority, WebClient, method};

/// `null` counts as the default (the server may send `"title": null`).
pub(crate) fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// Own presence setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceMode {
    Auto,
    Away,
}

/// One person's profile: `users.profile.get` (fields, status) joined with `users.info` (zone,
/// bot and deleted flags).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileRaw {
    pub id: String,
    pub display_name: String,
    pub real_name: String,
    pub title: String,
    pub pronouns: String,
    pub tz: Option<String>,
    pub status_text: String,
    pub status_emoji: String,
    /// Unix seconds, `0` for none.
    pub status_expiration: i64,
    pub email: String,
    pub phone: String,
    /// Custom profile fields as `(label, value)`, empty values dropped, sorted by label.
    pub fields: Vec<(String, String)>,
    /// URL of the largest avatar served over https.
    pub image: Option<String>,
    pub is_bot: bool,
    pub deleted: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ProfileEnvelope {
    #[serde(deserialize_with = "null_default")]
    profile: ProfileBody,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ProfileBody {
    #[serde(deserialize_with = "null_default")]
    display_name: String,
    #[serde(deserialize_with = "null_default")]
    real_name: String,
    #[serde(deserialize_with = "null_default")]
    title: String,
    #[serde(deserialize_with = "null_default")]
    pronouns: String,
    #[serde(deserialize_with = "null_default")]
    email: String,
    #[serde(deserialize_with = "null_default")]
    phone: String,
    #[serde(deserialize_with = "null_default")]
    status_text: String,
    #[serde(deserialize_with = "null_default")]
    status_emoji: String,
    #[serde(deserialize_with = "null_default")]
    status_expiration: i64,
    image_192: Option<String>,
    image_512: Option<String>,
    image_72: Option<String>,
    image_48: Option<String>,
    /// An object keyed by field id, or `[]` when the person has none.
    fields: Value,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct InfoEnvelope {
    #[serde(deserialize_with = "null_default")]
    user: InfoUser,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct InfoUser {
    #[serde(deserialize_with = "null_default")]
    deleted: bool,
    #[serde(deserialize_with = "null_default")]
    is_bot: bool,
    tz: Option<String>,
}

fn fields_of(v: &Value) -> Vec<(String, String)> {
    let Some(o) = v.as_object() else {
        return Vec::new();
    };
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for f in o.values() {
        let label = f.get("label").and_then(Value::as_str).unwrap_or_default();
        let value = f.get("value").and_then(Value::as_str).unwrap_or_default();
        if !label.is_empty() && !value.is_empty() {
            out.insert(label.to_string(), value.to_string());
        }
    }
    out.into_iter().collect()
}

impl ProfileBody {
    fn image(&mut self) -> Option<String> {
        [
            self.image_192.take(),
            self.image_512.take(),
            self.image_72.take(),
            self.image_48.take(),
        ]
        .into_iter()
        .flatten()
        .find(|u| u.starts_with("https://"))
    }
}

impl WebClient {
    /// `users.profile.get` (with labels) and `users.info`, joined. Either failing fails the
    /// call; `user_not_found` stays an `ApiError::Slack`.
    pub async fn profile_get(&self, user: &str) -> Result<ProfileRaw, ApiError> {
        let profile_params = [("user", user), ("include_labels", "true")];
        let info_params = [("user", user)];
        let profile = self.call::<ProfileEnvelope>(
            method::USERS_PROFILE_GET,
            &profile_params,
            Priority::Visible,
        );
        let info = self.call::<InfoEnvelope>(method::USERS_INFO, &info_params, Priority::Visible);
        let (p, i) = futures::future::try_join(profile, info).await?;
        let mut b = p.profile;
        let image = b.image();
        Ok(ProfileRaw {
            id: user.to_string(),
            fields: fields_of(&b.fields),
            display_name: b.display_name,
            real_name: b.real_name,
            title: b.title,
            pronouns: b.pronouns,
            tz: i.user.tz.filter(|t| !t.is_empty()),
            status_text: b.status_text,
            status_emoji: b.status_emoji,
            status_expiration: b.status_expiration,
            email: b.email,
            phone: b.phone,
            image,
            is_bot: i.user.is_bot,
            deleted: i.user.deleted,
        })
    }

    /// `users.profile.set` of the status fields. Empty `text` and `emoji` with `0` clear it.
    /// `expiration` is unix seconds, `0` for none.
    pub async fn profile_set_status(
        &self,
        text: &str,
        emoji: &str,
        expiration: i64,
    ) -> Result<(), ApiError> {
        let profile = serde_json::json!({
            "status_text": text,
            "status_emoji": emoji,
            "status_expiration": expiration,
        })
        .to_string();
        self.call_ok(
            method::USERS_PROFILE_SET,
            &[("profile", profile.as_str())],
            Priority::User,
            Retry::Write,
        )
        .await
    }

    /// `users.setPresence`.
    pub async fn set_presence(&self, mode: PresenceMode) -> Result<(), ApiError> {
        let presence = match mode {
            PresenceMode::Auto => "auto",
            PresenceMode::Away => "away",
        };
        self.call_ok(
            method::USERS_SET_PRESENCE,
            &[("presence", presence)],
            Priority::User,
            Retry::Write,
        )
        .await
    }
}
