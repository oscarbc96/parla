//! Huddles. `huddles.get` and `huddles.history` match the 2026-11-13 capture and the edge
//! calls `huddles/list` and `huddles/info` (the running huddles) the 2026-11-18 one; the decline call is
//! unverified (docs/providers/slack/web-client/huddles.md).

use serde::Deserialize;
use serde_json::Value;

use crate::client::Retry;
use crate::{ApiError, Priority, Secret, WebClient, method};

/// Rows `huddles.history` is asked for, the web client's own page size. The call has no
/// cursor in the capture, so a full page may hide older rows: see [`Huddles::truncated`].
pub const HISTORY_LIMIT: u32 = 30;

/// A huddle room. Every wire field may be missing or `null`; unknown fields are ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Room {
    pub id: String,
    pub channels: Vec<String>,
    pub participants: Vec<String>,
    pub created_by: Option<String>,
    pub date_start: i64,
    /// Unix seconds; `0` while the huddle runs.
    pub date_end: i64,
    pub has_ended: bool,
    pub is_dm_call: bool,
    pub huddle_link: Option<String>,
    /// The keys of the object form seen in captures (a plain list is accepted too).
    pub pending_invitees: Vec<String>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn keys_or_strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Object(o)) => o.keys().filter(|k| !k.is_empty()).cloned().collect(),
        other => strings(other),
    }
}

impl Room {
    /// `None` when the value is not an object or its id is missing or empty (such a room
    /// could never be matched to a later event).
    pub fn parse(v: &Value) -> Option<Room> {
        let o = v.as_object()?;
        let id = o.get("id")?.as_str().filter(|s| !s.is_empty())?.to_string();
        let str_of = |k: &str| {
            o.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let int_of = |k: &str| o.get(k).and_then(Value::as_i64).unwrap_or(0);
        Some(Room {
            id,
            channels: strings(o.get("channels")),
            participants: strings(o.get("participants")),
            created_by: str_of("created_by"),
            date_start: int_of("date_start"),
            date_end: int_of("date_end"),
            has_ended: o.get("has_ended").and_then(Value::as_bool).unwrap_or(false),
            is_dm_call: o
                .get("is_dm_call")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            huddle_link: str_of("huddle_link"),
            pending_invitees: keys_or_strings(o.get("pending_invitees")),
        })
    }

    /// Still running: not flagged ended and no end time.
    pub fn is_active(&self) -> bool {
        !self.has_ended && self.date_end <= 0
    }
}

/// Rooms of one `huddles.history` call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Huddles {
    pub rooms: Vec<Room>,
    /// The server returned a full page, so older rows may exist that were not read.
    pub truncated: bool,
}

/// The media region the web client asks `rooms.join` for in the capture.
const JOIN_REGION: &str = "eu-west-3";

/// What `rooms.join` returns for the media session (`call.free_willy`). `Debug` redacts the
/// join token.
#[derive(Debug)]
pub struct RoomsJoin {
    pub meeting_id: String,
    pub attendee_id: String,
    pub external_user_id: String,
    pub join_token: Secret,
    pub signaling_url: String,
    pub audio_host_url: String,
    pub turn_control_url: Option<String>,
}

impl RoomsJoin {
    /// `Err` (a decode error naming the missing path, never a value) when a required field is
    /// missing, not a string or empty.
    fn parse(v: &Value) -> Result<RoomsJoin, ApiError> {
        let miss = |p: &str| ApiError::Decode {
            method: method::ROOMS_JOIN.name,
            pointer: p.to_string(),
            message: "missing or empty".to_string(),
        };
        let at = |p: &str| -> Result<String, ApiError> {
            v.pointer(p)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .ok_or_else(|| miss(p))
        };
        let base = "/call/free_willy";
        Ok(RoomsJoin {
            meeting_id: at(&format!("{base}/meeting/MeetingId"))?,
            attendee_id: at(&format!("{base}/attendee/AttendeeId"))?,
            external_user_id: at(&format!("{base}/attendee/ExternalUserId"))?,
            join_token: Secret::new(at(&format!("{base}/attendee/JoinToken"))?),
            signaling_url: at(&format!("{base}/meeting/MediaPlacement/SignalingUrl"))?,
            audio_host_url: at(&format!("{base}/meeting/MediaPlacement/AudioHostUrl"))?,
            turn_control_url: at(&format!("{base}/meeting/MediaPlacement/TurnControlUrl")).ok(),
        })
    }
}

#[derive(Deserialize, Default)]
struct One {
    huddle: Option<Value>,
}

impl Room {
    /// One entry of a `huddles/list` (or `huddles/info`) response. Its `call_id` is the room id, its
    /// `active_members` the participants; `None` without a call or channel id.
    fn parse_edge(v: &Value) -> Option<Room> {
        let o = v.as_object()?;
        let str_of = |k: &str| {
            o.get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let id = str_of("call_id")?;
        let channel = str_of("channel_id")?;
        Some(Room {
            id,
            is_dm_call: channel.starts_with('D'),
            channels: vec![channel],
            participants: strings(o.get("active_members")),
            created_by: str_of("created_by"),
            date_start: o.get("start_date").and_then(Value::as_i64).unwrap_or(0),
            date_end: 0,
            has_ended: false,
            huddle_link: None,
            pending_invitees: keys_or_strings(o.get("pending_invitees")),
        })
    }
}

#[derive(Deserialize, Default)]
struct Many {
    huddles: Option<Vec<Value>>,
}

impl WebClient {
    /// `huddles.get`: one room by id. `Ok(None)` when the response holds no usable room.
    pub async fn huddles_get(&self, id: &str) -> Result<Option<Room>, ApiError> {
        let r: One = self
            .call(method::HUDDLES_GET, &[("huddle_id", id)], Priority::User)
            .await?;
        Ok(r.huddle.as_ref().and_then(Room::parse))
    }

    /// `huddles.history`: the newest rooms, running and ended. Entries without an id are
    /// dropped. Active huddles are the ones with [`Room::is_active`].
    pub async fn huddles_history(&self, prio: Priority) -> Result<Huddles, ApiError> {
        let limit = HISTORY_LIMIT.to_string();
        let r: Many = self
            .call(method::HUDDLES_HISTORY, &[("limit", limit.as_str())], prio)
            .await?;
        let raw = r.huddles.unwrap_or_default();
        let truncated = raw.len() >= HISTORY_LIMIT as usize;
        Ok(Huddles {
            rooms: raw.iter().filter_map(Room::parse).collect(),
            truncated,
        })
    }

    /// `huddles/list` on the edge API: every huddle running now that the user can see, the
    /// web client's page-load call (`huddles.history` lists only ended rooms). The body is
    /// only the token. Entries without a call or channel id are dropped.
    pub async fn huddles_list(&self, team: &str, prio: Priority) -> Result<Vec<Room>, ApiError> {
        let r: Many = self
            .edge_call(
                method::HUDDLES_LIST,
                team,
                "huddles/list",
                serde_json::json!({}),
                prio,
            )
            .await?;
        Ok(r.huddles
            .unwrap_or_default()
            .iter()
            .filter_map(Room::parse_edge)
            .collect())
    }

    /// `rooms.join` for a channel's huddle with the params the web client sends (2026-11-21
    /// capture). A write (it joins, or starts, the huddle): only a 429 is retried. The caller
    /// must keep the result secret.
    pub async fn rooms_join(&self, channel: &str) -> Result<RoomsJoin, ApiError> {
        self.rooms_join_with(channel, false).await
    }

    /// `rooms.join` with the `multidevice` flag set explicitly (the web client sends `false`;
    /// `true` is for a second attendee of the same user, see the native calls spike note).
    pub async fn rooms_join_with(
        &self,
        channel: &str,
        multidevice: bool,
    ) -> Result<RoomsJoin, ApiError> {
        let r: Value = self
            .call_write(
                method::ROOMS_JOIN,
                &[
                    ("channel_id", channel),
                    ("regions", JOIN_REGION),
                    ("multidevice", if multidevice { "true" } else { "false" }),
                    ("_x_reason", "rooms-join"),
                    ("_x_mode", "online"),
                    ("_x_app_name", "client"),
                    ("_x_sonic", "true"),
                ],
                Priority::User,
            )
            .await?;
        RoomsJoin::parse(&r)
    }

    /// `huddles.decline` (unverified). Never resent after a timeout or 5xx; only a 429 is
    /// retried.
    pub async fn huddles_decline(&self, channel: &str, id: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::HUDDLES_DECLINE,
            &[("huddle_id", id), ("channel_id", channel)],
            Priority::User,
            Retry::Write,
        )
        .await
    }
}
