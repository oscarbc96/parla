//! The write confirmation prompt for a plugin:
//! the plugin's name and source, the extra write actions, and the "Allow
//! without asking" check box, hidden for MCP and for a delete.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use components::confirm_write::{ConfirmButton, Row};
use components::{ConfirmWriteEvent, ConfirmWriteView};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use settings::WorkspaceKey;
use write_confirm::{
    ClientId, ClientInfo, PluginSource, ReminderKind, Requester, Workspace, WriteAction,
    WriteRequest,
};

fn plugin() -> Requester {
    Requester::Plugin {
        id: Arc::from("standup-bot"),
        name: Arc::from("Standup Bot"),
        source: PluginSource::Registry,
    }
}

fn request(requester: Requester, action: WriteAction) -> WriteRequest {
    WriteRequest::new(
        requester,
        Workspace {
            key: WorkspaceKey::new("fake", "W1"),
            name: Arc::from("Acme"),
        },
        Some(Arc::from("Marta Vidal")),
        action,
    )
}

fn send() -> WriteAction {
    WriteAction::Send {
        conversation_name: Arc::from("general"),
        thread: None,
        broadcast: false,
        text: Arc::from("Standup in 5"),
    }
}

fn delete() -> WriteAction {
    WriteAction::Delete {
        conversation_name: Arc::from("general"),
        message_excerpt: Arc::from("Standup moved"),
    }
}

type Events = Rc<RefCell<Vec<ConfirmWriteEvent>>>;

fn open(
    cx: &mut TestAppContext,
    req: WriteRequest,
) -> (Entity<ConfirmWriteView>, &mut VisualTestContext, Events) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let events: Events = Rc::default();
    let (view, vcx) = cx.add_window_view(move |_, cx| ConfirmWriteView::new(req, cx));
    let ev = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &ConfirmWriteEvent, _| {
            ev.borrow_mut().push(*e)
        })
        .detach();
    });
    vcx.run_until_parked();
    (view, vcx, events)
}

type Text = (String, String, Option<String>);

fn row(label: &str, value: &str, note: Option<&str>) -> Text {
    (label.into(), value.into(), note.map(Into::into))
}

fn texts(rows: Vec<Row>) -> Vec<Text> {
    rows.into_iter()
        .map(|r| (r.label, r.value, r.note))
        .collect()
}

fn activate(vcx: &mut VisualTestContext) {
    vcx.update(|w, _| w.activate_window());
    vcx.run_until_parked();
}

fn arm(vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(Duration::from_millis(1100));
    vcx.run_until_parked();
}

fn click(vcx: &mut VisualTestContext, sel: &'static str) {
    let at = vcx.debug_bounds(sel).expect("drawn").center();
    vcx.simulate_click(at, Modifiers::none());
}

#[gpui::test]
fn plugin_request_shows_plugin_name(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, request(plugin(), send()));
    let l = common::en();
    view.read_with(vcx, |v, _| {
        assert_eq!(v.title(&l), "Standup Bot wants to send a message");
        assert_eq!(
            texts(v.rows(&l, &jiff::tz::TimeZone::UTC)),
            [
                row("Plugin", "Standup Bot", Some("registry")),
                row("Action", "send_message", None),
                row("Post to", "#general", Some("in Acme")),
                row("As", "Marta Vidal (you)", None),
            ]
        );
        assert_eq!(v.body(), "Standup in 5");
        assert!(v.details().contains("\"plugin\": \"standup-bot\""));
    });
    // Plugins answer with the check box, not "Always for this session".
    assert!(vcx.debug_bounds("confirm-write-allow-session").is_none());
    assert!(vcx.debug_bounds("confirm-write-allow-once").is_some());
}

#[gpui::test]
fn delete_never_offers_remember(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, request(plugin(), delete()));
    let l = common::en();
    view.read_with(vcx, |v, _| {
        assert_eq!(v.title(&l), "Standup Bot wants to delete a message");
        assert_eq!(v.remember_label(&l), None);
        assert_eq!(v.body(), "Standup moved");
    });
    assert!(vcx.debug_bounds("confirm-write-remember").is_none());
    activate(vcx);
    arm(vcx);
    click(vcx, "confirm-write-allow-once");
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::Approved]);
}

#[gpui::test]
fn remember_offered_in_conversation_for_send(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, request(plugin(), send()));
    let l = common::en();
    view.read_with(vcx, |v, _| {
        assert_eq!(
            v.remember_label(&l).as_deref(),
            Some("Allow without asking in #general")
        );
        assert!(!v.remember());
    });
    activate(vcx);
    click(vcx, "confirm-write-remember");
    assert!(view.read_with(vcx, |v, _| v.remember()));
    assert!(events.borrow().is_empty(), "ticking answers nothing");
    arm(vcx);
    click(vcx, "confirm-write-allow-once");
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::ApprovedAndRemember]);
}

#[gpui::test]
fn other_areas_offer_remember_for_the_plugin(cx: &mut TestAppContext) {
    let action = WriteAction::MarkRead {
        conversation_name: Arc::from("general"),
    };
    let (view, vcx, _) = open(cx, request(plugin(), action));
    let l = common::en();
    view.read_with(vcx, |v, _| {
        assert_eq!(
            v.remember_label(&l).as_deref(),
            Some("Allow without asking for this plugin")
        );
        assert_eq!(v.title(&l), "Standup Bot wants to mark a conversation read");
    });
}

#[gpui::test]
fn space_on_the_check_box_ticks_it_and_does_not_deny(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, request(plugin(), send()));
    activate(vcx);
    arm(vcx);
    assert_eq!(
        vcx.update(|w, cx| view.read(cx).focused(w)),
        Some(ConfirmButton::Deny)
    );
    // The check box row sits above the footer: it comes before Deny.
    vcx.simulate_keystrokes("shift-tab");
    assert_eq!(
        vcx.update(|w, cx| view.read(cx).focused(w)),
        Some(ConfirmButton::Remember)
    );
    vcx.simulate_keystrokes("space");
    assert!(view.read_with(vcx, |v, _| v.remember()));
    assert!(events.borrow().is_empty());
    // Space disarms Allow for a second, and an unarmed Allow is not a tab stop.
    arm(vcx);
    vcx.simulate_keystrokes("tab tab");
    assert_eq!(
        vcx.update(|w, cx| view.read(cx).focused(w)),
        Some(ConfirmButton::AllowOnce)
    );
}

#[gpui::test]
fn mcp_never_shows_the_check_box(cx: &mut TestAppContext) {
    let mcp = Requester::Mcp(ClientInfo {
        id: ClientId(1),
        name: Arc::from("claude-code"),
        peer_pid: None,
    });
    let (view, vcx, _) = open(cx, request(mcp, send()));
    let l = common::en();
    view.read_with(vcx, |v, _| assert_eq!(v.remember_label(&l), None));
    assert!(vcx.debug_bounds("confirm-write-remember").is_none());
    assert!(vcx.debug_bounds("confirm-write-allow-session").is_some());
}

#[gpui::test]
fn extra_actions_have_rows_and_text(cx: &mut TestAppContext) {
    let l = common::en();
    let epoch = l.date_time(
        jiff::Timestamp::UNIX_EPOCH,
        &jiff::tz::TimeZone::UTC,
        i18n::HourCycle::H24,
    );
    let cases: Vec<(WriteAction, &str, Vec<Text>, &str)> = vec![
        (
            WriteAction::Upload {
                conversation_name: Arc::from("general"),
                name: Arc::from("chart.png"),
                bytes: 2048,
            },
            "Standup Bot wants to upload a file",
            vec![row("File", "chart.png", Some("2.0 KB"))],
            "",
        ),
        (
            WriteAction::Reminder {
                kind: ReminderKind::Add,
                text: Arc::from("Ship it"),
                at: Some(jiff::Timestamp::UNIX_EPOCH),
            },
            "Standup Bot wants to add a reminder",
            vec![row("At", &epoch, None)],
            "Ship it",
        ),
        (
            WriteAction::Presence { away: true },
            "Standup Bot wants to change your presence",
            vec![row("Presence", "Away", None)],
            "",
        ),
        (
            WriteAction::Snooze { minutes: Some(30) },
            "Standup Bot wants to pause notifications",
            vec![row("Notifications", "Paused for 30 minutes", None)],
            "",
        ),
    ];
    for (action, title, tail, body) in cases {
        let (view, vcx, _) = open(cx, request(plugin(), action));
        view.read_with(vcx, |v, _| {
            assert_eq!(v.title(&l), title);
            let rows = texts(v.rows(&l, &jiff::tz::TimeZone::UTC));
            assert_eq!(rows[4..], tail[..], "{title}");
            assert_eq!(v.body(), body, "{title}");
        });
    }
}

fn mcp() -> Requester {
    Requester::Mcp(ClientInfo {
        id: ClientId(1),
        name: Arc::from("claude-code"),
        peer_pid: Some(4242),
    })
}

#[gpui::test]
fn mcp_reaction_with_empty_excerpt_keeps_the_text_block(cx: &mut TestAppContext) {
    let action = WriteAction::React {
        conversation_name: Arc::from("general"),
        message_excerpt: Arc::from(""),
        emoji: domain::EmojiRef {
            shortcode: Arc::from("tada"),
            unicode: Some(Arc::from("\u{1f389}")),
            skin_tone: None,
        },
    };
    let (_view, vcx, _) = open(cx, request(mcp(), action));
    assert!(vcx.debug_bounds("confirm-write-text").is_some());
}

#[gpui::test]
fn plugin_write_without_text_hides_the_text_block(cx: &mut TestAppContext) {
    let action = WriteAction::MarkRead {
        conversation_name: Arc::from("general"),
    };
    let (_view, vcx, _) = open(cx, request(plugin(), action));
    assert!(vcx.debug_bounds("confirm-write-text").is_none());
}

#[gpui::test]
fn space_on_the_check_box_disarms_allow_for_a_second(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, request(plugin(), send()));
    activate(vcx);
    arm(vcx);
    vcx.simulate_keystrokes("shift-tab space");
    assert!(view.read_with(vcx, |v, _| v.remember()));
    assert!(!view.read_with(vcx, |v, _| v.can_approve()));
    click(vcx, "confirm-write-allow-once");
    assert!(events.borrow().is_empty(), "locked after the keystroke");
    arm(vcx);
    click(vcx, "confirm-write-allow-once");
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::ApprovedAndRemember]);
}

fn excerpt_action(kind: &str) -> WriteAction {
    let c = Arc::from("general");
    let m = Arc::from("Standup moved");
    match kind {
        "delete" => WriteAction::Delete {
            conversation_name: c,
            message_excerpt: m,
        },
        "pin" => WriteAction::Pin {
            conversation_name: c,
            message_excerpt: m,
            on: true,
        },
        "unpin" => WriteAction::Pin {
            conversation_name: c,
            message_excerpt: m,
            on: false,
        },
        "save" => WriteAction::Save {
            conversation_name: c,
            message_excerpt: m,
            on: true,
        },
        "unsave" => WriteAction::Save {
            conversation_name: c,
            message_excerpt: m,
            on: false,
        },
        "edit" => WriteAction::Edit {
            conversation_name: c,
            message_excerpt: m,
            text: Arc::from("Standup moved to 10:30"),
        },
        "react" => WriteAction::React {
            conversation_name: c,
            message_excerpt: m,
            emoji: domain::EmojiRef {
                shortcode: Arc::from("tada"),
                unicode: Some(Arc::from("\u{1f389}")),
                skin_tone: None,
            },
        },
        _ => send(),
    }
}

#[gpui::test]
fn plugin_text_block_and_subtitle_name_the_action(cx: &mut TestAppContext) {
    let l = common::en();
    let sent = "Nothing is sent until you allow it.";
    let changed = "Nothing changes until you allow it.";
    for (kind, label, subtitle) in [
        ("send", "Exact message", sent),
        ("delete", "Message to delete", changed),
        ("pin", "Message to pin", changed),
        ("unpin", "Message to unpin", changed),
        ("save", "Message to save", changed),
        ("unsave", "Message to remove from saved", changed),
        ("edit", "New text", changed),
        ("react", "Message to react to", changed),
    ] {
        let (view, vcx, _) = open(cx, request(plugin(), excerpt_action(kind)));
        view.read_with(vcx, |v, _| {
            assert_eq!(v.exact_label(&l), label, "{kind}");
            assert_eq!(v.subtitle(&l), subtitle, "{kind}");
        });
    }
    // MCP keeps its wording.
    let (view, vcx, _) = open(cx, request(mcp(), excerpt_action("react")));
    view.read_with(vcx, |v, _| {
        assert_eq!(v.exact_label(&l), "Exact message");
        assert_eq!(v.subtitle(&l), sent);
    });
}

#[gpui::test]
fn delete_allow_is_the_danger_button(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, request(plugin(), delete()));
    assert!(view.read_with(vcx, |v, _| v.allow_is_danger()));
    let (view, vcx, _) = open(cx, request(plugin(), send()));
    assert!(!view.read_with(vcx, |v, _| v.allow_is_danger()));
}

#[gpui::test]
fn remember_row_sits_above_the_footer_and_fits(cx: &mut TestAppContext) {
    let long = "a-very-long-channel-name-of-forty-chars!";
    assert_eq!(long.chars().count(), 40);
    for locale in ["en", "es", "ca", "en-XA"] {
        let action = WriteAction::Send {
            conversation_name: Arc::from(long),
            thread: None,
            broadcast: false,
            text: Arc::from("hi"),
        };
        let (_view, vcx, _) = open(cx, request(plugin(), action));
        // The prompt window at its smallest.
        vcx.simulate_resize(gpui::size(
            themes::metrics::LAYOUT_WINDOW_CONFIRM_MIN_WIDTH,
            themes::metrics::LAYOUT_WINDOW_CONFIRM_MIN_HEIGHT,
        ));
        vcx.update(|_, cx| {
            i18n::gpui::set_pref(cx, i18n::LocalePref::Code(locale.parse().unwrap()))
        });
        vcx.run_until_parked();
        let window = vcx.update(|w, _| w.viewport_size());
        let remember = vcx.debug_bounds("confirm-write-remember").expect("drawn");
        let allow = vcx.debug_bounds("confirm-write-allow-once").expect("drawn");
        let deny = vcx.debug_bounds("confirm-write-deny").expect("drawn");
        assert!(
            remember.bottom() <= allow.top(),
            "{locale}: row above footer"
        );
        assert!(remember.bottom() <= deny.top(), "{locale}");
        assert!(remember.right() <= window.width, "{locale}: fits the width");
        assert!(
            allow.right() <= window.width,
            "{locale}: allow stays visible"
        );
        assert!(
            allow.bottom() <= window.height,
            "{locale}: allow stays visible"
        );
    }
}

#[gpui::test]
fn sigil_follows_the_conversation_kind(cx: &mut TestAppContext) {
    use domain::ConversationKind;
    let l = common::en();
    for (kind, value, remember) in [
        (
            ConversationKind::Channel,
            "#general",
            "Allow without asking in #general",
        ),
        (
            ConversationKind::PrivateChannel,
            "general",
            "Allow without asking in \u{201c}general\u{201d}",
        ),
        (
            ConversationKind::Group,
            "general",
            "Allow without asking in \u{201c}general\u{201d}",
        ),
        (
            ConversationKind::Direct,
            "general",
            "Allow without asking in your conversation with general",
        ),
    ] {
        let req = request(plugin(), send()).with_conversation_kind(kind);
        let (view, vcx, _) = open(cx, req);
        view.read_with(vcx, |v, _| {
            let rows = texts(v.rows(&l, &jiff::tz::TimeZone::UTC));
            assert_eq!(rows[2], row("Post to", value, Some("in Acme")), "{kind:?}");
            assert_eq!(v.remember_label(&l).as_deref(), Some(remember), "{kind:?}");
        });
    }
}
