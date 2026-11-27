use slack_realtime::event::{Outgoing, Presence, SlackEvent, parse_frame};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spike")
}

/// The inner frame of a captured fixture (`data.json`), as text.
fn frame(name: &str) -> String {
    let text = std::fs::read_to_string(fixture_dir().join(name)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["data"]["json"].to_string()
}

fn received() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixture_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with("ws-") && !n.starts_with("ws-send-") && n.ends_with(".json"))
        .collect();
    names.sort();
    names
}

#[test]
fn parses_every_fixture() {
    let names = received();
    assert!(names.len() > 20);
    // Explicitly ignorable in M1: they must land in Unknown, never panic or error.
    let ignorable = [
        "activity",
        "badge_counts_updated",
        "draft_create",
        "draft_update",
        "pref_change",
        "subscription_update",
        "thread_subscribed",
        "update_global_thread_state",
        "user_interaction_changed",
        "user_invalidated",
        "visits_changed",
    ];
    for n in names {
        let ev = parse_frame(&frame(&n)).unwrap_or_else(|e| panic!("{n}: {e}"));
        let stem = n.trim_start_matches("ws-").trim_end_matches(".json");
        if let SlackEvent::Unknown { kind, .. } = &ev {
            assert!(ignorable.contains(&stem), "{n} fell into Unknown({kind})");
        }
    }
}

#[test]
fn fixtures_map_to_expected_variants() {
    let p = |n: &str| parse_frame(&frame(n)).unwrap();
    assert!(matches!(p("ws-pong.json"), SlackEvent::Pong { .. }));
    assert!(matches!(
        p("ws-reconnect_url.json"),
        SlackEvent::ReconnectUrl { .. }
    ));
    assert!(matches!(
        p("ws-channel_marked.json"),
        SlackEvent::ChannelMarked { .. }
    ));
    assert!(matches!(
        p("ws-user_change.json"),
        SlackEvent::UserChange { .. }
    ));
    assert!(matches!(
        p("ws-message-message_replied.json"),
        SlackEvent::MessageReplied { .. }
    ));
    assert!(matches!(
        p("ws-message-thread_broadcast.json"),
        SlackEvent::Message { .. }
    ));
    assert!(matches!(
        p("ws-message-huddle_thread.json"),
        SlackEvent::Message { .. }
    ));
    for n in [
        "ws-sh_room_join.json",
        "ws-sh_room_update.json",
        "ws-sh_room_leave.json",
        "ws-user_huddle_changed.json",
        "ws-huddle_invite_cancel.json",
    ] {
        assert!(matches!(p(n), SlackEvent::Huddle { .. }), "{n}");
    }
}

#[test]
fn unknown_event_kept_raw() {
    let text = r#"{"type":"brand_new",  "x":[1,2]}"#;
    match parse_frame(text).unwrap() {
        SlackEvent::Unknown { kind, raw } => {
            assert_eq!(kind, "brand_new");
            assert_eq!(raw.get(), text);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn message_changed_is_its_own_variant() {
    match parse_frame(&frame("ws-message-message_changed.json")).unwrap() {
        SlackEvent::MessageChanged { channel, message } => {
            assert_eq!(channel, "C0000001");
            assert!(!message.ts.is_empty());
        }
        other => panic!("{other:?}"),
    }
    match parse_frame(&frame("ws-message-message_deleted.json")).unwrap() {
        SlackEvent::MessageDeleted {
            channel,
            deleted_ts,
        } => {
            assert_eq!(channel, "C0000005");
            assert_eq!(deleted_ts, "1794526178.065219");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn presence_batched_and_single() {
    match parse_frame(&frame("ws-presence_change.json")).unwrap() {
        SlackEvent::PresenceChange { users, presence } => {
            assert_eq!(users, vec!["U0000001"]);
            assert_eq!(presence, Presence::Active);
        }
        other => panic!("{other:?}"),
    }
    let single = r#"{"type":"presence_change","user":"U1","presence":"away"}"#;
    match parse_frame(single).unwrap() {
        SlackEvent::PresenceChange { users, presence } => {
            assert_eq!(users, vec!["U1"]);
            assert_eq!(presence, Presence::Away);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn plain_message_hello_goodbye_error_typing() {
    let m = r#"{"type":"message","channel":"C1","user":"U1","text":"hi","ts":"1.2"}"#;
    assert!(matches!(
        parse_frame(m).unwrap(),
        SlackEvent::Message { .. }
    ));
    assert!(matches!(
        parse_frame(r#"{"type":"hello"}"#).unwrap(),
        SlackEvent::Hello
    ));
    assert!(matches!(
        parse_frame(r#"{"type":"goodbye"}"#).unwrap(),
        SlackEvent::Goodbye
    ));
    match parse_frame(r#"{"type":"error","error":{"code":1,"msg":"bad"}}"#).unwrap() {
        SlackEvent::Error { msg } => {
            assert_eq!(msg, "bad");
        }
        other => panic!("{other:?}"),
    }
    match parse_frame(r#"{"type":"user_typing","channel":"C1","user":"U2","thread_ts":"9.9"}"#)
        .unwrap()
    {
        SlackEvent::UserTyping {
            channel,
            user,
            thread_ts,
        } => {
            assert_eq!((channel.as_str(), user.as_str()), ("C1", "U2"));
            assert_eq!(thread_ts.as_deref(), Some("9.9"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn outgoing_frames_match_capture() {
    // The capture's sent frames, compared as JSON (key order is not significant).
    let sent = |n: &str| -> serde_json::Value {
        let t = std::fs::read_to_string(fixture_dir().join(n)).unwrap();
        serde_json::from_str::<serde_json::Value>(&t).unwrap()["data"]["json"].clone()
    };
    let to_v = |o: Outgoing| serde_json::from_str::<serde_json::Value>(&o.to_frame()).unwrap();
    assert_eq!(
        to_v(Outgoing::Ping { id: 16391 }),
        sent("ws-send-ping.json")
    );
    assert_eq!(
        to_v(Outgoing::PresenceSub {
            id: 16397,
            ids: vec!["U0000001".into(), "U0000002".into(), "U0000003".into()]
        }),
        sent("ws-send-presence_sub.json")
    );
    assert_eq!(
        to_v(Outgoing::Typing {
            id: 16406,
            channel: "D0000006".into(),
            thread_ts: None
        }),
        sent("ws-send-user_typing.json")
    );
    assert_eq!(
        Outgoing::Ping { id: 1 }.to_frame(),
        r#"{"id":1,"type":"ping"}"#
    );
}

#[test]
fn presence_sub_is_capped_and_split() {
    let ids: Vec<String> = (0..1600).map(|i| format!("U{i}")).collect();
    let frames = Outgoing::PresenceSub { id: 5, ids }.to_frames();
    assert_eq!(frames.len(), 3);
    let v: serde_json::Value = serde_json::from_str(&frames[0]).unwrap();
    assert_eq!(v["ids"].as_array().unwrap().len(), 750);
}

#[test]
fn malformed_frame_is_error_not_panic() {
    for bad in [
        "",
        "not json",
        "[]",
        "{}",
        r#"{"type":5}"#,
        r#"{"type":"message"}"#,
        r#"{"type":"pong"}"#,
    ] {
        assert!(parse_frame(bad).is_err(), "{bad}");
    }
}

fn synthetic(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/events")
        .join(format!("{name}.json"));
    std::fs::read_to_string(p).unwrap().trim().to_string()
}

#[test]
fn reaction_events_parse() {
    match parse_frame(&synthetic("reaction_added")).unwrap() {
        SlackEvent::ReactionAdded {
            user,
            reaction,
            channel,
            ts,
        } => {
            assert_eq!((user.as_str(), reaction.as_str()), ("U0000002", "thumbsup"));
            assert_eq!(
                (channel.as_str(), ts.as_str()),
                ("C0000001", "1700000000.000100")
            );
        }
        other => panic!("{other:?}"),
    }
    match parse_frame(&synthetic("reaction_removed")).unwrap() {
        SlackEvent::ReactionRemoved { reaction, .. } => {
            assert_eq!(reaction, "thumbsup::skin-tone-3")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn reaction_on_file_is_unknown() {
    assert!(matches!(
        parse_frame(&synthetic("reaction_on_file")).unwrap(),
        SlackEvent::Unknown { .. }
    ));
}

#[test]
fn message_replied_is_its_own_variant() {
    match parse_frame(&frame("ws-message-message_replied.json")).unwrap() {
        SlackEvent::MessageReplied { channel, message } => {
            assert_eq!(channel, "C0000001");
            assert_eq!(message.ts, "1794525536.639029");
            assert_eq!(message.reply_count, Some(1));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn thread_marked_from_capture() {
    match parse_frame(&frame("ws-thread_marked.json")).unwrap() {
        SlackEvent::ThreadMarked {
            channel,
            thread_ts,
            ts,
        } => {
            assert_eq!(channel, "C0000005");
            assert_eq!(thread_ts, "1794526178.065219");
            assert_eq!(ts, "1794526178.065219");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn all_marked_kinds_map_to_channel_marked() {
    for n in ["im_marked", "group_marked", "mpim_marked"] {
        match parse_frame(&synthetic(n)).unwrap() {
            SlackEvent::ChannelMarked { ts, .. } => assert_eq!(ts, "1700000000.000100"),
            other => panic!("{n}: {other:?}"),
        }
    }
}

#[test]
fn file_events_parse() {
    match parse_frame(&synthetic("file_shared")).unwrap() {
        SlackEvent::FileShared { file_id } => {
            assert_eq!(file_id, "F0000001");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        parse_frame(&synthetic("file_change")).unwrap(),
        SlackEvent::FileChange { file_id } if file_id == "F0000001"
    ));
    assert!(matches!(
        parse_frame(&synthetic("file_deleted")).unwrap(),
        SlackEvent::FileDeleted { file_id } if file_id == "F0000001"
    ));
    assert!(matches!(
        parse_frame(&synthetic("emoji_changed")).unwrap(),
        SlackEvent::EmojiChanged
    ));
}

#[test]
fn synthetic_fixtures_all_parse() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/events");
    for e in std::fs::read_dir(dir).unwrap() {
        let text = std::fs::read_to_string(e.unwrap().path()).unwrap();
        parse_frame(&text).unwrap();
    }
}

#[test]
fn dnd_events_parse() {
    match parse_frame(&synthetic("dnd_updated")).unwrap() {
        SlackEvent::DndUpdated { user, dnd } => {
            assert_eq!(user, "U0000001");
            assert!(dnd.dnd_enabled && dnd.snooze_enabled);
            assert_eq!(dnd.snooze_endtime, 1_700_001_800);
        }
        other => panic!("{other:?}"),
    }
    match parse_frame(&synthetic("dnd_updated_user")).unwrap() {
        SlackEvent::DndUpdatedUser { user, dnd } => {
            assert_eq!(user, "U0000002");
            assert_eq!(dnd.next_dnd_end_ts, 1_700_030_000);
            assert!(!dnd.snooze_enabled);
        }
        other => panic!("{other:?}"),
    }
    // A missing or null status is an empty one, never an error.
    match parse_frame(&synthetic("dnd_updated_nulls")).unwrap() {
        SlackEvent::DndUpdated { user, dnd } => {
            assert!(user.is_empty());
            assert!(!dnd.dnd_enabled);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn pref_change_parses_and_names_the_notification_prefs() {
    use slack_realtime::event::NOTIFY_PREF_NAMES;
    let name_of = |n: &str| match parse_frame(&synthetic(n)).unwrap() {
        SlackEvent::PrefChange { name, value } => (name, value.get().to_string()),
        other => panic!("{other:?}"),
    };
    let (n, v) = name_of("pref_change_notify");
    assert!(NOTIFY_PREF_NAMES.contains(&n.as_str()));
    assert!(v.starts_with('"'));
    assert!(NOTIFY_PREF_NAMES.contains(&name_of("pref_change_muted").0.as_str()));
    assert!(!NOTIFY_PREF_NAMES.contains(&name_of("pref_change_other").0.as_str()));
    // The captured spike frame parses too, with a boolean value.
    match parse_frame(&frame("ws-pref_change.json")).unwrap() {
        SlackEvent::PrefChange { value, .. } => assert_eq!(value.get(), "true"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn user_change_with_status_parses_with_nulls() {
    match parse_frame(&synthetic("user_change_status")).unwrap() {
        SlackEvent::UserChange { user } => {
            assert_eq!(user.profile.status_text, "Lunch");
            assert_eq!(user.profile.status_emoji, ":taco:");
            assert_eq!(user.profile.status_expiration, 1_800_000_000);
            assert!(!user.deleted, "null counts as false");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn pin_events_parse() {
    match parse_frame(&synthetic("pin_added")).unwrap() {
        SlackEvent::PinAdded { channel, ts } => {
            assert_eq!(
                (channel.as_str(), ts.as_str()),
                ("C0000001", "1800000000.000100")
            );
        }
        other => panic!("{other:?}"),
    }
    match parse_frame(&synthetic("pin_removed")).unwrap() {
        SlackEvent::PinRemoved { channel, ts } => {
            assert_eq!(
                (channel.as_str(), ts.as_str()),
                ("C0000001", "1800000000.000100")
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(
            parse_frame(&synthetic("pin_added_file")).unwrap(),
            SlackEvent::Unknown { .. }
        ),
        "a file pin is not a message pin"
    );
}

#[test]
fn bookmark_events_parse() {
    for n in ["bookmark_added", "bookmark_changed", "bookmark_removed"] {
        match parse_frame(&synthetic(n)).unwrap() {
            SlackEvent::BookmarksChanged { channel } => assert_eq!(channel, "C0000001"),
            other => panic!("{n}: {other:?}"),
        }
    }
}

#[test]
fn saved_events_parse() {
    for n in ["saved_added", "saved_removed"] {
        assert!(
            matches!(
                parse_frame(&synthetic(n)).unwrap(),
                SlackEvent::SavedChanged
            ),
            "{n}"
        );
    }
}

#[test]
fn thread_inbox_frames_from_capture() {
    // The captured `thread_subscribed` names the thread and whether it is followed.
    assert!(matches!(
        parse_frame(&frame("ws-thread_subscribed.json")).unwrap(),
        SlackEvent::ThreadSubscribed { channel, thread_ts, active: true }
            if channel == "C0000005" && thread_ts == "1794526178.065219"
    ));
    let n = "ws-update_global_thread_state.json";
    assert!(
        matches!(
            parse_frame(&frame(n)).unwrap(),
            SlackEvent::ThreadInboxChanged
        ),
        "{n}"
    );
}

#[test]
fn team_rename_parses() {
    match parse_frame(&synthetic("team_rename")).unwrap() {
        SlackEvent::TeamRename { name } => assert_eq!(name, "New Name"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn member_frames_parse_for_anyone() {
    match parse_frame(&synthetic("member_joined_channel")).unwrap() {
        SlackEvent::MemberJoined { channel, user } => {
            assert_eq!((channel.as_str(), user.as_str()), ("C0000001", "U0000002"));
        }
        other => panic!("{other:?}"),
    }
    match parse_frame(&synthetic("member_left_channel")).unwrap() {
        SlackEvent::MemberLeft { channel, user } => {
            assert_eq!((channel.as_str(), user.as_str()), ("C0000001", "U0000002"));
        }
        other => panic!("{other:?}"),
    }
}
