//! Huddle frames from synthetic fixtures (the spike captures cover the channel
//! case; DM ring frames are unverified, see docs/providers/slack/web-client/huddles.md).

use slack_realtime::event::{SlackEvent, parse_frame};
use slack_realtime::huddle::HuddleEvent;

fn synth(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/huddles/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn spike(name: &str) -> String {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/spike/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["data"]["json"].to_string()
}

fn event(text: &str) -> HuddleEvent {
    match parse_frame(text).unwrap() {
        SlackEvent::Huddle { event, .. } => event,
        other => panic!("{other:?}"),
    }
}

#[test]
fn room_events_parse_from_spike_fixtures() {
    match event(&spike("ws-sh_room_join.json")) {
        HuddleEvent::RoomJoin { room, user } => {
            assert_eq!(user, "U0000001");
            assert_eq!(room.id, "R0000010");
            assert_eq!(room.channels, ["C0000005"]);
            assert_eq!(room.participants, ["U0000001"]);
            assert_eq!(room.created_by.as_deref(), Some("U0000001"));
            assert_eq!(room.date_start, 1_794_526_177);
            assert!(!room.has_ended && !room.is_dm_call);
            assert!(room.pending_invitees.is_empty());
            assert!(room.huddle_link.is_some());
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        event(&spike("ws-sh_room_update.json")),
        HuddleEvent::RoomUpdate { .. }
    ));
    assert!(matches!(
        event(&spike("ws-sh_room_leave.json")),
        HuddleEvent::RoomLeave { .. }
    ));
    match event(&spike("ws-huddle_invite_cancel.json")) {
        HuddleEvent::InviteCancel { channel } => assert_eq!(channel, "C0000005"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn pending_invitees_come_from_object_keys_and_nulls_are_tolerated() {
    match event(&synth("room_join_ring.json")) {
        HuddleEvent::RoomJoin { room, .. } => {
            assert_eq!(room.pending_invitees, ["U0000001"]);
            assert!(room.is_dm_call);
        }
        other => panic!("{other:?}"),
    }
    match event(&synth("room_leave.json")) {
        HuddleEvent::RoomLeave { room, user } => {
            assert_eq!(user, "U0000003");
            assert!(room.created_by.is_none());
            assert!(room.pending_invitees.is_empty());
            assert_eq!(room.date_end, 0);
        }
        other => panic!("{other:?}"),
    }
    match event(&synth("room_update_final.json")) {
        HuddleEvent::RoomUpdate { room } => assert!(room.has_ended && room.date_end > 0),
        other => panic!("{other:?}"),
    }
}

#[test]
fn user_huddle_changed_parses_state() {
    assert!(matches!(
        event(&synth("user_in_huddle.json")),
        HuddleEvent::UserHuddleChanged { in_huddle: true, user } if user == "U0000002"
    ));
    assert!(matches!(
        event(&synth("user_left_huddle.json")),
        HuddleEvent::UserHuddleChanged {
            in_huddle: false,
            ..
        }
    ));
    match event(&spike("ws-user_huddle_changed.json")) {
        HuddleEvent::UserHuddleChanged { in_huddle, .. } => assert!(!in_huddle),
        other => panic!("{other:?}"),
    }
}

#[test]
fn invite_cancel_parses_top_level_channel() {
    assert!(matches!(
        event(&synth("invite_cancel.json")),
        HuddleEvent::InviteCancel { channel } if channel == "D0000007"
    ));
}

#[test]
fn malformed_room_is_unknown() {
    for text in [
        synth("room_no_id.json"),
        r#"{"type":"sh_room_join","user":"U1"}"#.to_string(),
        r#"{"type":"sh_room_update","room":"x"}"#.to_string(),
        r#"{"type":"sh_room_leave","room":{"id":"R1"}}"#.to_string(),
        r#"{"type":"user_huddle_changed","user":{"id":"","profile":{"huddle_state":"in_a_huddle"}}}"#
            .to_string(),
        r#"{"type":"user_huddle_changed","user":{"id":"U1"}}"#.to_string(),
        r#"{"type":"huddle_invite_cancel"}"#.to_string(),
    ] {
        match parse_frame(&text).unwrap() {
            SlackEvent::Unknown { .. } => {}
            other => panic!("{text}: {other:?}"),
        }
    }
}

fn live(name: &str) -> String {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/huddles-live/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["data"]["json"].to_string()
}

/// 2026-11-18 capture: a DM huddle rings (join with a pending invitee), updates while it
/// runs, loses its last participant, and ends with `sh_room_update` plus `ended_huddle_update`.
#[test]
fn live_dm_huddle_frames_parse() {
    match event(&live("ws-7087-sh_room_join.json")) {
        HuddleEvent::RoomJoin { room, user } => {
            assert_eq!(user, "U0000013");
            assert_eq!(room.id, "R0000016");
            assert_eq!(room.channels, ["D0000012"]);
            assert!(room.is_dm_call && room.is_active());
            assert_eq!(room.pending_invitees, ["U0000014"]);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        event(&live("ws-7080-sh_room_update.json")),
        HuddleEvent::RoomUpdate { room } if room.is_active()
    ));
    assert!(matches!(
        event(&live("ws-7101-sh_room_leave.json")),
        HuddleEvent::RoomLeave { room, user } if user == "U0000013" && room.id == "R0000016"
    ));
}

#[test]
fn live_channel_huddle_ends_by_update_and_ended_huddle_update() {
    match event(&live("ws-7069-sh_room_update.json")) {
        HuddleEvent::RoomUpdate { room } => {
            assert!(!room.is_active());
            assert_eq!(room.date_end, 1_794_996_137);
        }
        other => panic!("{other:?}"),
    }
    // `ended_huddle_update` carries the room under `huddle`, not `room`.
    match event(&live("ws-7072-ended_huddle_update.json")) {
        HuddleEvent::Ended { room } => {
            assert_eq!(room.id, "R0000014");
            assert_eq!(room.channels, ["C0000010"]);
            assert!(room.has_ended && !room.is_active());
            assert_eq!(room.date_start, 1_794_995_685);
        }
        other => panic!("{other:?}"),
    }
}
