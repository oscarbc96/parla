use std::sync::Arc;

use domain::test_support::{conv, member, uid};
use domain::{
    Bootstrap, Input, PresenceSetting, RemindTarget, SlashCommand, SlashError, WorkspaceState,
    parse_slash, reduce, slash_to_input,
};
use settings::WorkspaceKey;

/// A fixed wall clock, so `/dnd 2h` maps to a stable deadline.
const NOW: i64 = 1_700_000_000;

fn ok(input: &str) -> SlashCommand {
    match parse_slash(input) {
        Some(Ok(c)) => c,
        other => panic!("{input:?} did not parse: {other:?}"),
    }
}

fn err(input: &str) -> SlashError {
    match parse_slash(input) {
        Some(Err(e)) => e,
        other => panic!("{input:?} did not fail: {other:?}"),
    }
}

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![conv("C1", "ops")],
            members: vec![member("U1", "Ana"), member("U2", "Bo")],
        }),
    );
    s
}

#[test]
fn remind_me_in_minutes() {
    assert_eq!(
        ok("/remind me stretch in 20 minutes"),
        SlashCommand::Remind {
            target: RemindTarget::Me,
            what: "stretch".into(),
            when: "in 20 minutes".into(),
        }
    );
    // The provider receives the when verbatim, short forms included.
    assert_eq!(
        ok("/remind me stretch in 20 min"),
        SlashCommand::Remind {
            target: RemindTarget::Me,
            what: "stretch".into(),
            when: "in 20 min".into(),
        }
    );
}

#[test]
fn remind_person_tomorrow_at() {
    assert_eq!(
        ok("/remind @ana send the report tomorrow at 09:00"),
        SlashCommand::Remind {
            target: RemindTarget::Person("ana".into()),
            what: "send the report".into(),
            when: "tomorrow at 09:00".into(),
        }
    );
    assert_eq!(
        ok("/remind @ana ping tomorrow"),
        SlashCommand::Remind {
            target: RemindTarget::Person("ana".into()),
            what: "ping".into(),
            when: "tomorrow".into(),
        }
    );
}

#[test]
fn remind_quoted_what_first() {
    assert_eq!(
        ok(r#"/remind me "call the bank" in 2 hours"#),
        SlashCommand::Remind {
            target: RemindTarget::Me,
            what: "call the bank".into(),
            when: "in 2 hours".into(),
        }
    );
}

#[test]
fn remind_channel_and_every() {
    assert_eq!(
        ok("/remind #ops water the plants on monday at 09:00"),
        SlashCommand::Remind {
            target: RemindTarget::Conversation("ops".into()),
            what: "water the plants".into(),
            when: "on monday at 09:00".into(),
        }
    );
    assert_eq!(
        ok("/remind me water the plants every monday"),
        SlashCommand::Remind {
            target: RemindTarget::Me,
            what: "water the plants".into(),
            when: "every monday".into(),
        }
    );
    assert_eq!(
        ok("/remind me pay rent on 2026-12-01 at 10:00"),
        SlashCommand::Remind {
            target: RemindTarget::Me,
            what: "pay rent".into(),
            when: "on 2026-12-01 at 10:00".into(),
        }
    );
}

#[test]
fn remind_without_when_is_usage() {
    assert!(matches!(err("/remind me stretch"), SlashError::Usage(_)));
    assert!(matches!(err("/remind me"), SlashError::Usage(_)));
    assert!(matches!(err("/remind"), SlashError::Usage(_)));
    // No target: a reminder always names me, a person or a conversation.
    assert!(matches!(
        err("/remind stretch in 2 minutes"),
        SlashError::Usage(_)
    ));
    // An empty quoted what is not a reminder either.
    assert!(matches!(
        err(r#"/remind me "   " in 2 hours"#),
        SlashError::Usage(_)
    ));
}

#[test]
fn dnd_forms() {
    assert_eq!(ok("/dnd"), SlashCommand::Dnd(Some(60)));
    assert_eq!(ok("/dnd 30"), SlashCommand::Dnd(Some(30)));
    assert_eq!(ok("/dnd 45m"), SlashCommand::Dnd(Some(45)));
    assert_eq!(ok("/dnd 2h"), SlashCommand::Dnd(Some(120)));
    assert_eq!(ok("/dnd off"), SlashCommand::DndOff);
    assert_eq!(ok("/DND Off"), SlashCommand::DndOff);
    assert!(matches!(err("/dnd 0m"), SlashError::Usage(_)));
    assert!(matches!(err("/dnd 2 weeks"), SlashError::Usage(_)));
    assert!(matches!(err("/dnd soon"), SlashError::Usage(_)));
}

#[test]
fn status_with_emoji_and_clear() {
    assert_eq!(
        ok("/status :palm_tree: On vacation"),
        SlashCommand::Status {
            emoji: Some("palm_tree".into()),
            text: "On vacation".into(),
        }
    );
    assert_eq!(
        ok("/status :coffee:"),
        SlashCommand::Status {
            emoji: Some("coffee".into()),
            text: String::new(),
        }
    );
    assert_eq!(
        ok("/status in a meeting"),
        SlashCommand::Status {
            emoji: None,
            text: "in a meeting".into(),
        }
    );
    assert_eq!(ok("/status clear"), SlashCommand::ClearStatus);
    assert_eq!(ok("/status"), SlashCommand::ClearStatus);
}

#[test]
fn away_and_active() {
    assert_eq!(ok("/away"), SlashCommand::Away);
    assert_eq!(ok("/active"), SlashCommand::Active);
    assert_eq!(ok("/Away"), SlashCommand::Away);
}

#[test]
fn unknown_command() {
    assert_eq!(err("/giphy cats"), SlashError::Unknown("giphy".into()));
    assert_eq!(err("/"), SlashError::Unknown(String::new()));
}

#[test]
fn not_a_command() {
    assert!(parse_slash("hello /remind").is_none());
    assert!(parse_slash("").is_none());
    assert!(parse_slash("   ").is_none());
    // A command may be indented.
    assert!(parse_slash("  /away").is_some());
}

#[test]
fn resolves_person_and_conversation_names() {
    let s = state();
    match slash_to_input(ok("/remind @ana stretch in 20 minutes"), &s, NOW) {
        Ok(Input::AddReminder { text, when, user }) => {
            assert_eq!(&*text, "stretch");
            assert_eq!(&*when, "in 20 minutes");
            assert_eq!(user, Some(uid("U1")));
        }
        other => panic!("{other:?}"),
    }
    // Case-insensitive, by display name.
    assert!(matches!(
        slash_to_input(ok("/remind @ANA stretch in 20 minutes"), &s, NOW),
        Ok(Input::AddReminder {
            user: Some(u),
            ..
        }) if u == uid("U1")
    ));
    // A conversation is validated by name.
    assert!(matches!(
        slash_to_input(ok("/remind #ops water in 2 hours"), &s, NOW),
        Ok(Input::AddReminder { user: None, .. })
    ));
    assert_eq!(
        slash_to_input(ok("/remind @nobody x in 2 hours"), &s, NOW).unwrap_err(),
        SlashError::UnknownName("nobody".into())
    );
    assert!(matches!(
        slash_to_input(ok("/remind #nope x in 2 hours"), &s, NOW),
        Err(SlashError::UnknownName(n)) if n == "nope"
    ));
    // A handle wins over another member's display name.
    let mut s2 = s.clone();
    let mut bo = member("U3", "Someone");
    bo.handle = Arc::from("ana");
    s2.members.insert(bo.id.clone(), bo);
    assert!(matches!(
        slash_to_input(ok("/remind @ana x in 2 hours"), &s2, NOW),
        Ok(Input::AddReminder { user: Some(u), .. }) if u == uid("U3")
    ));
    // Me needs no lookup.
    assert!(matches!(
        slash_to_input(ok("/remind me x in 2 hours"), &s, NOW),
        Ok(Input::AddReminder { user: None, .. })
    ));
}

#[test]
fn maps_dnd_status_and_presence_to_inputs() {
    let s = state();
    assert!(matches!(
        slash_to_input(ok("/dnd 2h"), &s, NOW),
        Ok(Input::Snooze {
            minutes: 120,
            now_unix
        }) if now_unix == NOW
    ));
    assert!(matches!(
        slash_to_input(ok("/dnd"), &s, NOW),
        Ok(Input::Snooze {
            minutes: 60,
            now_unix
        }) if now_unix == NOW
    ));
    assert!(matches!(
        slash_to_input(ok("/dnd off"), &s, NOW),
        Ok(Input::EndSnooze)
    ));
    assert!(matches!(
        slash_to_input(ok("/away"), &s, NOW),
        Ok(Input::SetPresence(PresenceSetting::Away))
    ));
    assert!(matches!(
        slash_to_input(ok("/active"), &s, NOW),
        Ok(Input::SetPresence(PresenceSetting::Auto))
    ));
    assert!(matches!(
        slash_to_input(ok("/status clear"), &s, NOW),
        Ok(Input::SetStatus(None))
    ));
    match slash_to_input(ok("/status :coffee: brewing"), &s, NOW) {
        Ok(Input::SetStatus(Some(st))) => {
            assert_eq!(&*st.text, "brewing");
            assert_eq!(st.emoji.as_ref().map(|e| &*e.shortcode), Some("coffee"));
        }
        other => panic!("{other:?}"),
    }
}
