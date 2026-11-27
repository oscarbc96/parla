//! Channel renames, archives and leaves (synthetic fixtures in the public
//! event shapes; see docs/providers/slack/web-client/conversations.md
//! "Channel admin").

use std::path::PathBuf;

use slack_realtime::event::{SlackEvent, parse_frame};

fn synthetic(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/events")
            .join(format!("{name}.json")),
    )
    .unwrap()
}

#[test]
fn renames_carry_the_new_name() {
    for (f, id, name) in [
        ("channel_rename", "C0000020", "launch-day"),
        ("group_rename", "C0000021", "launch-team"),
    ] {
        match parse_frame(&synthetic(f)).unwrap() {
            SlackEvent::ChannelRenamed { channel, name: n } => {
                assert_eq!((channel.as_str(), n.as_str()), (id, name), "{f}");
            }
            other => panic!("{f}: {other:?}"),
        }
    }
}

#[test]
fn archives_and_unarchives_say_which() {
    for (f, archived) in [
        ("channel_archive", true),
        ("group_archive", true),
        ("channel_unarchive", false),
    ] {
        match parse_frame(&synthetic(f)).unwrap() {
            SlackEvent::ChannelArchived { archived: a, .. } => assert_eq!(a, archived, "{f}"),
            other => panic!("{f}: {other:?}"),
        }
    }
}

#[test]
fn leaving_names_the_channel() {
    for (f, id) in [("channel_left", "C0000020"), ("group_left", "C0000021")] {
        match parse_frame(&synthetic(f)).unwrap() {
            SlackEvent::ChannelLeft { channel } => assert_eq!(channel, id, "{f}"),
            other => panic!("{f}: {other:?}"),
        }
    }
}
