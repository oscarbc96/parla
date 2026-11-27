//! Per-conversation notification preferences (`users.prefs.get`/`users.prefs.set` of
//! `all_notifications_prefs` and `muted_channels`). Unverified against a capture: the JSON
//! string is read with channel entries either under `channels` or at the top level, and
//! written back the same way, keeping every key it does not own.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::client::Retry;
use crate::people::null_default;
use crate::{ApiError, Priority, WebClient, method};

/// One conversation's stored preference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelPrefRaw {
    pub id: String,
    /// `everything`, `mentions`, `nothing`, or `None` when unset.
    pub desktop: Option<String>,
    pub muted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotifyPrefsRaw {
    pub channels: Vec<ChannelPrefRaw>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PrefsEnvelope {
    #[serde(deserialize_with = "null_default")]
    prefs: PrefsBody,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PrefsBody {
    all_notifications_prefs: Option<String>,
    muted_channels: Option<String>,
}

/// What was stored. `all` is `None` when the string is absent, null, empty or not a JSON
/// object, and `muted` is `None` when the list is absent or null: reading treats both as empty,
/// but a write is refused rather than built on what was never read.
struct Stored {
    all: Option<Map<String, Value>>,
    muted: Option<Vec<String>>,
}

fn stored(b: PrefsBody) -> Stored {
    let all = match b
        .all_notifications_prefs
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        Some(Value::Object(o)) => Some(o),
        _ => None,
    };
    let muted = b.muted_channels.map(|m| split_muted(&m));
    Stored { all, muted }
}

fn split_muted(list: &str) -> Vec<String> {
    let mut muted: Vec<String> = Vec::new();
    {
        for id in list.split(',') {
            let id = id.trim();
            if !id.is_empty() && !muted.iter().any(|m| m == id) {
                muted.push(id.to_string());
            }
        }
    }
    muted
}

/// Where channel entries live: `channels` when present as an object, else the top level.
fn entries(all: &Map<String, Value>) -> impl Iterator<Item = (&String, &Value)> {
    let nested = all.get("channels").and_then(Value::as_object);
    nested.into_iter().flatten().chain(
        all.iter()
            .filter(move |(k, _)| nested.is_none() && *k != "global" && *k != "channels"),
    )
}

fn parse(s: Stored) -> NotifyPrefsRaw {
    let all = s.all.unwrap_or_default();
    let mut channels: Vec<ChannelPrefRaw> = Vec::new();
    for (id, v) in entries(&all) {
        let Some(o) = v.as_object() else { continue };
        if id.is_empty() {
            continue;
        }
        channels.push(ChannelPrefRaw {
            id: id.clone(),
            desktop: o
                .get("desktop")
                .and_then(Value::as_str)
                .filter(|d| !d.is_empty())
                .map(str::to_string),
            muted: o.get("muted").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    for id in s.muted.unwrap_or_default() {
        match channels.iter_mut().find(|c| c.id == id) {
            Some(c) => c.muted = true,
            None => channels.push(ChannelPrefRaw {
                id,
                desktop: None,
                muted: true,
            }),
        }
    }
    channels.sort_by(|a, b| a.id.cmp(&b.id));
    NotifyPrefsRaw { channels }
}

const PREF_ALL: &str = "all_notifications_prefs";
const PREF_MUTED: &str = "muted_channels";

impl WebClient {
    async fn prefs_set(&self, name: &str, value: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::USERS_PREFS_SET,
            &[("name", name), ("value", value)],
            Priority::User,
            Retry::Write,
        )
        .await
    }

    async fn prefs_stored(&self) -> Result<Stored, ApiError> {
        let r: PrefsEnvelope = self
            .call(method::USERS_PREFS_GET, &[], Priority::Visible)
            .await?;
        Ok(stored(r.prefs))
    }

    /// [`prefs_get`](Self::prefs_get) for a background refresh: one attempt, background priority.
    pub async fn prefs_get_once(&self) -> Result<NotifyPrefsRaw, ApiError> {
        let r: PrefsEnvelope = self
            .call_once(method::USERS_PREFS_GET, &[], Priority::Background)
            .await?;
        Ok(parse(stored(r.prefs)))
    }

    /// The notification prefs, parsed. Empty ids and non-object entries are dropped.
    pub async fn prefs_get(&self) -> Result<NotifyPrefsRaw, ApiError> {
        Ok(parse(self.prefs_stored().await?))
    }

    /// Set one conversation's level (`None` removes the stored level) and mute. Reads the
    /// current prefs, changes only this conversation and writes the strings back. Only the
    /// `desktop` level is written (`mobile` is left as it is). Refused with a decode error,
    /// before anything is sent, when `all_notifications_prefs` did not parse as an object or
    /// `muted_channels` is absent, so a failed read never overwrites the stored prefs. The
    /// read-modify-write is not serialised here: the caller must not run two at once.
    pub async fn prefs_set_conversation(
        &self,
        channel: &str,
        desktop: Option<&str>,
        muted_flag: bool,
    ) -> Result<(), ApiError> {
        let Stored { all, muted } = self.prefs_stored().await?;
        let unreadable = |pointer: &str, message: &str| ApiError::Decode {
            method: method::USERS_PREFS_GET.name,
            pointer: pointer.to_string(),
            message: message.to_string(),
        };
        let mut all =
            all.ok_or_else(|| unreadable("/prefs/all_notifications_prefs", "not a JSON object"))?;
        let mut muted = muted.ok_or_else(|| unreadable("/prefs/muted_channels", "missing"))?;
        let nested = all.get("channels").is_some_and(Value::is_object);
        let map = if nested {
            all.get_mut("channels").and_then(Value::as_object_mut)
        } else {
            Some(&mut all)
        };
        if let Some(map) = map {
            let entry = map
                .entry(channel.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            if let Some(o) = entry.as_object_mut() {
                match desktop {
                    Some(d) => {
                        o.insert("desktop".into(), Value::String(d.to_string()));
                    }
                    None => {
                        o.remove("desktop");
                    }
                }
                o.insert("muted".into(), Value::Bool(muted_flag));
            }
        }
        let was_muted = muted.iter().any(|m| m == channel);
        let all_json = Value::Object(all).to_string();
        self.prefs_set(PREF_ALL, &all_json).await?;
        if was_muted != muted_flag {
            if muted_flag {
                muted.push(channel.to_string());
            } else {
                muted.retain(|m| m != channel);
            }
            self.prefs_set(PREF_MUTED, &muted.join(",")).await?;
        }
        Ok(())
    }
}
