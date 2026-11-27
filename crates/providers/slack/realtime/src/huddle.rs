//! Huddle frames: `sh_room_join`, `sh_room_update`, `sh_room_leave`, `ended_huddle_update`,
//! `user_huddle_changed` and `huddle_invite_cancel`. The channel case is seen in the
//! 2026-11-13 capture, the DM ring and the end of a huddle in the 2026-11-18 one
//! (docs/providers/slack/web-client/huddles.md).
//! A frame that lacks what its variant needs is not a huddle event: the caller keeps it `Unknown`.

use serde_json::Value;
pub use slack_api::Room;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HuddleEvent {
    RoomJoin {
        room: Room,
        user: String,
    },
    RoomUpdate {
        room: Room,
    },
    RoomLeave {
        room: Room,
        user: String,
    },
    /// `ended_huddle_update`: the final room, carried under `huddle` (not `room`).
    Ended {
        room: Room,
    },
    UserHuddleChanged {
        user: String,
        in_huddle: bool,
    },
    InviteCancel {
        channel: String,
    },
}

fn nonempty(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn room(v: &Value) -> Option<Room> {
    let mut r = Room::parse(v.get("room")?)?;
    if r.channels.is_empty() {
        let c = nonempty(v.get("huddle").and_then(|h| h.get("channel_id")))?;
        r.channels.push(c.to_string());
    }
    Some(r)
}

/// `None` for a frame that is not a well-formed huddle frame.
pub fn parse(kind: &str, v: &Value) -> Option<HuddleEvent> {
    Some(match kind {
        "sh_room_join" => HuddleEvent::RoomJoin {
            room: room(v)?,
            user: nonempty(v.get("user"))?.to_string(),
        },
        "sh_room_update" => HuddleEvent::RoomUpdate { room: room(v)? },
        "sh_room_leave" => HuddleEvent::RoomLeave {
            room: room(v)?,
            user: nonempty(v.get("user"))?.to_string(),
        },
        "ended_huddle_update" => HuddleEvent::Ended {
            room: Room::parse(v.get("huddle")?)?,
        },
        "user_huddle_changed" => {
            let u = v.get("user")?;
            let state = u.get("profile")?.get("huddle_state")?.as_str()?;
            HuddleEvent::UserHuddleChanged {
                user: nonempty(u.get("id"))?.to_string(),
                in_huddle: state == "in_a_huddle",
            }
        }
        "huddle_invite_cancel" => HuddleEvent::InviteCancel {
            channel: nonempty(v.get("channel_id"))?.to_string(),
        },
        _ => return None,
    })
}
