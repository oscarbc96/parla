//! Typed frames of the session websocket
//! (`docs/providers/slack/web-client/realtime.md`).
//!
//! Parsing is total: a frame is a known variant, `Unknown` (kept byte-for-byte)
//! or a `FrameError`. It never panics. Message and user payloads are typed
//! with `slack_api`'s `Message` and `User`.

use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use slack_api::{DndRaw, Message, User};

use crate::huddle::HuddleEvent;

/// Most ids a single `presence_sub` frame carries.
pub const PRESENCE_SUB_MAX_IDS: usize = 750;

/// Preference names whose change means the notification prefs changed.
pub const NOTIFY_PREF_NAMES: [&str; 2] = ["all_notifications_prefs", "muted_channels"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Active,
    Away,
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame is not a JSON object with a string `type`")]
    NoType,
    #[error("`{kind}` frame lacks or mistypes `{field}`")]
    Field { kind: String, field: &'static str },
}

#[derive(Debug, Clone)]
pub enum SlackEvent {
    Hello,
    /// A new message; `message` is the whole frame (message fields at top level).
    Message {
        channel: String,
        message: Box<Message>,
    },
    /// `message` is the edited message (the frame's inner `message`).
    MessageChanged {
        channel: String,
        message: Box<Message>,
    },
    /// Subtype `message_replied`: the thread root with updated reply fields.
    MessageReplied {
        channel: String,
        message: Box<Message>,
    },
    MessageDeleted {
        channel: String,
        deleted_ts: String,
    },
    /// `channel_marked`, `im_marked`, `group_marked` and `mpim_marked`.
    ChannelMarked {
        channel: String,
        ts: String,
    },
    /// A thread's read position moved (`ts` is the last read reply).
    ThreadMarked {
        channel: String,
        thread_ts: String,
        ts: String,
    },
    /// `reaction` may carry a `::skin-tone-N` suffix.
    ReactionAdded {
        user: String,
        reaction: String,
        channel: String,
        ts: String,
    },
    ReactionRemoved {
        user: String,
        reaction: String,
        channel: String,
        ts: String,
    },
    FileShared {
        file_id: String,
    },
    FileChange {
        file_id: String,
    },
    FileDeleted {
        file_id: String,
    },
    EmojiChanged,
    /// A message was pinned (`pin_added`; UNVERIFIED: no capture).
    PinAdded {
        channel: String,
        ts: String,
    },
    PinRemoved {
        channel: String,
        ts: String,
    },
    /// `bookmark_added`, `bookmark_changed` and `bookmark_removed` (UNVERIFIED).
    BookmarksChanged {
        channel: String,
    },
    /// A message was saved or unsaved (`star_added`, `star_removed`; UNVERIFIED).
    SavedChanged,
    /// `update_global_thread_state` (seen in the spike), or a `thread_subscribed` without
    /// its subscription: the Threads view is stale.
    ThreadInboxChanged,
    /// `thread_subscribed` (captured): the thread's subscription, `active` when the user
    /// follows it. Also makes the Threads view stale.
    ThreadSubscribed {
        channel: String,
        thread_ts: String,
        active: bool,
    },
    /// Team renamed (UNVERIFIED: public docs shape, synthetic fixture).
    TeamRename {
        name: String,
    },
    UserTyping {
        channel: String,
        user: String,
        thread_ts: Option<String>,
    },
    PresenceChange {
        users: Vec<String>,
        presence: Presence,
    },
    /// `channel_rename` / `group_rename`: the channel object with its new name.
    ChannelRenamed {
        channel: String,
        name: String,
    },
    /// `channel_archive` / `group_archive` (`archived`) and the `unarchive` pair.
    ChannelArchived {
        channel: String,
        archived: bool,
    },
    /// `channel_left` / `group_left`: the signed-in user left (here or elsewhere).
    ChannelLeft {
        channel: String,
    },
    MemberJoined {
        channel: String,
        user: String,
    },
    MemberLeft {
        channel: String,
        user: String,
    },
    UserChange {
        user: Box<User>,
    },
    /// Own DND changed (`dnd_updated`). `user` is empty when the frame lacks it: the
    /// provider fills in the signed-in user.
    DndUpdated {
        user: String,
        dnd: DndRaw,
    },
    /// Another user's DND changed.
    DndUpdatedUser {
        user: String,
        dnd: DndRaw,
    },
    /// A user preference changed; `value` is the frame's raw value.
    PrefChange {
        name: String,
        value: Box<RawValue>,
    },
    /// `sh_room_*`, `ended_huddle_update`, `user_huddle_changed`, `huddle_invite_cancel`.
    Huddle {
        event: HuddleEvent,
    },
    Pong {
        reply_to: u64,
    },
    /// The URL is a credential: never log it.
    ReconnectUrl {
        url: String,
    },
    Goodbye,
    Error {
        msg: String,
    },
    Unknown {
        kind: String,
        raw: Box<RawValue>,
    },
}

fn field<'a>(kind: &'a str, name: &'static str) -> impl FnOnce() -> FrameError + 'a {
    move || FrameError::Field {
        kind: kind.to_string(),
        field: name,
    }
}

fn str_of(v: &Value, kind: &str, name: &'static str) -> Result<String, FrameError> {
    v.get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(field(kind, name))
}

/// A channel given as `"C1"` or `{"id":"C1"}`.
fn channel_of(v: &Value, kind: &str) -> Result<String, FrameError> {
    match v.get("channel") {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(o) => o
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(field(kind, "channel")),
        None => Err(field(kind, "channel")()),
    }
}

fn typed_of<T: serde::de::DeserializeOwned>(
    v: &Value,
    kind: &str,
    name: &'static str,
) -> Result<Box<T>, FrameError> {
    let inner = v.get(name).ok_or_else(field(kind, name))?;
    Ok(Box::new(T::deserialize(inner)?))
}

/// A frame's `item` when it is a message (not a file or other item).
fn message_item(v: &Value) -> Option<&Value> {
    let item = v.get("item")?;
    (item.get("type").and_then(Value::as_str) == Some("message")).then_some(item)
}

/// A reaction frame's target; `None` for non-message items (files).
fn reaction_target(v: &Value) -> Option<(String, String)> {
    let item = message_item(v)?;
    Some((
        item.get("channel")?.as_str()?.to_string(),
        item.get("ts")?.as_str()?.to_string(),
    ))
}

/// The message a pin or star frame's `item` points at; `None` for files and other items.
fn item_message(v: &Value) -> Option<(String, String)> {
    let item = message_item(v)?;
    let channel = item
        .get("channel")
        .or_else(|| v.get("channel_id"))?
        .as_str()?
        .to_string();
    let ts = item.get("message")?.get("ts")?.as_str()?.to_string();
    Some((channel, ts))
}

pub fn parse_frame(text: &str) -> Result<SlackEvent, FrameError> {
    let v: Value = serde_json::from_str(text)?;
    let kind = v
        .get("type")
        .and_then(Value::as_str)
        .ok_or(FrameError::NoType)?;
    let whole = || RawValue::from_string(text.to_string());
    let unknown = || -> Result<SlackEvent, FrameError> {
        Ok(SlackEvent::Unknown {
            kind: kind.to_string(),
            raw: whole()?,
        })
    };
    Ok(match kind {
        "hello" => SlackEvent::Hello,
        "goodbye" => SlackEvent::Goodbye,
        "pong" => SlackEvent::Pong {
            reply_to: v
                .get("reply_to")
                .and_then(Value::as_u64)
                .ok_or_else(field(kind, "reply_to"))?,
        },
        "reconnect_url" => SlackEvent::ReconnectUrl {
            url: str_of(&v, kind, "url")?,
        },
        "error" => {
            let e = v.get("error").unwrap_or(&v);
            SlackEvent::Error {
                msg: e
                    .get("msg")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            }
        }
        "message" => {
            let channel = str_of(&v, kind, "channel")?;
            match v.get("subtype").and_then(Value::as_str) {
                Some("message_changed") => SlackEvent::MessageChanged {
                    channel,
                    message: typed_of(&v, kind, "message")?,
                },
                Some("message_replied") => SlackEvent::MessageReplied {
                    channel,
                    message: typed_of(&v, kind, "message")?,
                },
                Some("message_deleted") => SlackEvent::MessageDeleted {
                    channel,
                    deleted_ts: str_of(&v, kind, "deleted_ts")?,
                },
                _ => SlackEvent::Message {
                    channel,
                    message: Box::new(Message::deserialize(&v)?),
                },
            }
        }
        "channel_marked" | "im_marked" | "group_marked" | "mpim_marked" => {
            SlackEvent::ChannelMarked {
                channel: str_of(&v, kind, "channel")?,
                ts: str_of(&v, kind, "ts")?,
            }
        }
        "thread_marked" => {
            let sub = v
                .get("subscription")
                .ok_or_else(field(kind, "subscription"))?;
            SlackEvent::ThreadMarked {
                channel: str_of(sub, kind, "channel")?,
                thread_ts: str_of(sub, kind, "thread_ts")?,
                ts: str_of(sub, kind, "last_read")?,
            }
        }
        "reaction_added" | "reaction_removed" => match reaction_target(&v) {
            Some((channel, ts)) => {
                let user = str_of(&v, kind, "user")?;
                let reaction = str_of(&v, kind, "reaction")?;
                if kind == "reaction_added" {
                    SlackEvent::ReactionAdded {
                        user,
                        reaction,
                        channel,
                        ts,
                    }
                } else {
                    SlackEvent::ReactionRemoved {
                        user,
                        reaction,
                        channel,
                        ts,
                    }
                }
            }
            None => unknown()?,
        },
        "file_shared" => SlackEvent::FileShared {
            file_id: str_of(&v, kind, "file_id")?,
        },
        "file_change" => SlackEvent::FileChange {
            file_id: str_of(&v, kind, "file_id")?,
        },
        "file_deleted" => SlackEvent::FileDeleted {
            file_id: str_of(&v, kind, "file_id")?,
        },
        "emoji_changed" => SlackEvent::EmojiChanged,
        "pin_added" | "pin_removed" => match item_message(&v) {
            Some((channel, ts)) => {
                if kind == "pin_added" {
                    SlackEvent::PinAdded { channel, ts }
                } else {
                    SlackEvent::PinRemoved { channel, ts }
                }
            }
            None => unknown()?,
        },
        "bookmark_added" | "bookmark_changed" | "bookmark_removed" => {
            SlackEvent::BookmarksChanged {
                channel: str_of(&v, kind, "channel_id")?,
            }
        }
        "star_added" | "star_removed" => match item_message(&v) {
            Some(_) => SlackEvent::SavedChanged,
            None => unknown()?,
        },
        "thread_subscribed" => match v.get("subscription") {
            Some(sub) => SlackEvent::ThreadSubscribed {
                channel: str_of(sub, kind, "channel")?,
                thread_ts: str_of(sub, kind, "thread_ts")?,
                // The captured frame carries `active: true`; a missing flag reads as followed.
                active: sub
                    .get("active")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
            },
            None => SlackEvent::ThreadInboxChanged,
        },
        "update_global_thread_state" => SlackEvent::ThreadInboxChanged,
        "team_rename" => SlackEvent::TeamRename {
            name: str_of(&v, kind, "name")?,
        },
        "user_typing" => SlackEvent::UserTyping {
            channel: str_of(&v, kind, "channel")?,
            user: str_of(&v, kind, "user")?,
            thread_ts: v
                .get("thread_ts")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        "presence_change" => {
            let presence = match v.get("presence").and_then(Value::as_str) {
                Some("active") => Presence::Active,
                Some("away") => Presence::Away,
                _ => return Err(field(kind, "presence")()),
            };
            let users = match (v.get("users"), v.get("user")) {
                (Some(Value::Array(a)), _) => a
                    .iter()
                    .map(|u| {
                        u.as_str()
                            .map(str::to_string)
                            .ok_or_else(field(kind, "users"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                (_, Some(Value::String(u))) => vec![u.clone()],
                _ => return Err(field(kind, "users")()),
            };
            SlackEvent::PresenceChange { users, presence }
        }
        "channel_rename" | "group_rename" => SlackEvent::ChannelRenamed {
            channel: channel_of(&v, kind)?,
            name: v
                .get("channel")
                .and_then(|c| c.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(field(kind, "channel.name"))?,
        },
        "channel_archive" | "group_archive" | "channel_unarchive" | "group_unarchive" => {
            SlackEvent::ChannelArchived {
                channel: channel_of(&v, kind)?,
                archived: kind.ends_with("_archive"),
            }
        }
        "channel_left" | "group_left" => SlackEvent::ChannelLeft {
            channel: channel_of(&v, kind)?,
        },
        "member_joined_channel" => SlackEvent::MemberJoined {
            channel: str_of(&v, kind, "channel")?,
            user: str_of(&v, kind, "user")?,
        },
        "member_left_channel" => SlackEvent::MemberLeft {
            channel: str_of(&v, kind, "channel")?,
            user: str_of(&v, kind, "user")?,
        },
        "user_change" => SlackEvent::UserChange {
            user: typed_of(&v, kind, "user")?,
        },
        "dnd_updated" | "dnd_updated_user" => {
            let user = v
                .get("user")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let status = v.get("dnd_status").or_else(|| v.get("dnd"));
            let dnd = match status {
                Some(Value::Null) | None => DndRaw::default(),
                Some(d) => DndRaw::deserialize(d)?,
            };
            if kind == "dnd_updated" {
                SlackEvent::DndUpdated { user, dnd }
            } else {
                SlackEvent::DndUpdatedUser { user, dnd }
            }
        }
        "pref_change" => SlackEvent::PrefChange {
            name: str_of(&v, kind, "name")?,
            value: RawValue::from_string(
                v.get("value")
                    .map_or_else(|| "null".to_string(), Value::to_string),
            )?,
        },
        "sh_room_join"
        | "sh_room_update"
        | "sh_room_leave"
        | "ended_huddle_update"
        | "user_huddle_changed"
        | "huddle_invite_cancel" => match crate::huddle::parse(kind, &v) {
            Some(event) => SlackEvent::Huddle { event },
            None => unknown()?,
        },
        _ => SlackEvent::Unknown {
            kind: kind.to_string(),
            raw: whole()?,
        },
    })
}

/// Frames the client sends. Ids come from one counter shared across all client frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    Ping {
        id: u64,
    },
    PresenceSub {
        id: u64,
        ids: Vec<String>,
    },
    PresenceQuery {
        id: u64,
        ids: Vec<String>,
    },
    Typing {
        id: u64,
        channel: String,
        thread_ts: Option<String>,
    },
}

impl Outgoing {
    /// The frame text; id lists are capped at [`PRESENCE_SUB_MAX_IDS`] (use
    /// [`Outgoing::to_frames`] to send every id).
    pub fn to_frame(&self) -> String {
        self.to_frames().into_iter().next().unwrap_or_default()
    }

    /// One frame, or several when a presence id list exceeds the cap.
    pub fn to_frames(&self) -> Vec<String> {
        use serde_json::json;
        match self {
            Outgoing::Ping { id } => vec![json!({"id": id, "type": "ping"}).to_string()],
            Outgoing::PresenceSub { id, ids } => chunked("presence_sub", *id, ids),
            Outgoing::PresenceQuery { id, ids } => chunked("presence_query", *id, ids),
            Outgoing::Typing {
                id,
                channel,
                thread_ts,
            } => {
                let mut v = json!({"id": id, "type": "user_typing", "channel": channel});
                if let Some(t) = thread_ts {
                    v["thread_ts"] = json!(t);
                }
                vec![v.to_string()]
            }
        }
    }
}

fn chunked(kind: &str, id: u64, ids: &[String]) -> Vec<String> {
    let frame = |c: &[String]| serde_json::json!({"id": id, "type": kind, "ids": c}).to_string();
    if ids.is_empty() {
        return vec![frame(&[])];
    }
    ids.chunks(PRESENCE_SUB_MAX_IDS).map(frame).collect()
}
