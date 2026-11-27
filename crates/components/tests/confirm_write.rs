//! The MCP write confirmation prompt: the key-value rows, the exact text, the
//! Details disclosure, the three answers (Deny, Always for this session,
//! Allow once), default focus, the arming delay, Escape and Enter.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use components::confirm_write::{ARM_DELAY, ConfirmButton, Row, escape_text};
use components::{
    ApproveWrite, ApproveWriteForSession, ConfirmWriteEvent, ConfirmWriteView, DeclineWrite,
};
use domain::{Capabilities, EmojiRef, Status};
use gpui::{
    Entity, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, TestAppContext,
    VisualTestContext,
};
use mcp::audit::ClientInfo;
use mcp::backend::WorkspaceInfo;
use mcp::confirm::{WriteAction, WriteRequest};
use mcp::limits::ClientId;
use settings::WorkspaceKey;

fn request(action: WriteAction) -> WriteRequest {
    WriteRequest {
        client: ClientInfo {
            id: ClientId(1),
            name: Arc::from("claude-code"),
            peer_pid: Some(4242),
        },
        workspace: WorkspaceInfo {
            key: WorkspaceKey::new("fake", "W1"),
            name: Arc::from("Acme"),
            provider_display: Arc::from("Fakechat"),
            self_user: None,
            caps: Capabilities::default(),
        },
        as_user: Some(Arc::from("Marta Vidal")),
        action,
    }
}

fn send(text: &str) -> WriteAction {
    WriteAction::Send {
        conversation_name: Arc::from("general"),
        thread: None,
        broadcast: false,
        text: Arc::from(text),
    }
}

type Events = Rc<RefCell<Vec<ConfirmWriteEvent>>>;

fn open(
    cx: &mut TestAppContext,
    action: WriteAction,
) -> (Entity<ConfirmWriteView>, &mut VisualTestContext, Events) {
    open_request(cx, request(action))
}

fn open_request(
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

/// The rows as text, without how each value is drawn.
fn texts(rows: Vec<Row>) -> Vec<Text> {
    rows.into_iter()
        .map(|r| (r.label, r.value, r.note))
        .collect()
}

fn focused(view: &Entity<ConfirmWriteView>, vcx: &mut VisualTestContext) -> Option<ConfirmButton> {
    vcx.update(|window, cx| view.read(cx).focused(window))
}

#[test]
fn escape_shows_controls_and_bidi_but_keeps_newlines() {
    assert_eq!(escape_text("a\nb\tc"), "a\nb\tc");
    assert_eq!(escape_text("x\u{202e}y"), "x\\u{202e}y");
    assert_eq!(
        escape_text("\u{0}\r\u{7f}\u{200b}"),
        "\\u{0000}\\u{000d}\\u{007f}\\u{200b}"
    );
    assert_eq!(
        escape_text("*bold* <@U1> https://x.test"),
        "*bold* <@U1> https://x.test"
    );
}

#[gpui::test]
fn prompt_shows_the_key_value_rows_and_exact_text(cx: &mut TestAppContext) {
    let text = "*bold* <@U1>\u{202e} done\nsecond line";
    let (view, vcx, _) = open(cx, send(text));
    let l = common::en();
    view.read_with(vcx, |v, _| {
        assert_eq!(v.title(&l), "An MCP client wants to send a message");
        assert_eq!(
            texts(v.rows(&l, &jiff::tz::TimeZone::UTC)),
            [
                row("Client", "claude-code", Some("stdio · pid 4242")),
                row("Tool", "send_message", None),
                row("Post to", "#general", Some("in Acme")),
                row("As", "Marta Vidal (you)", None),
            ]
        );
        assert_eq!(v.body(), "*bold* <@U1>\\u{202e} done\nsecond line");
    });
}

#[gpui::test]
fn unknown_pid_and_user_still_fill_the_rows(cx: &mut TestAppContext) {
    let mut req = request(send("hi"));
    req.client.peer_pid = None;
    req.as_user = None;
    let (view, vcx, _) = open_request(cx, req);
    let l = common::en();
    view.read_with(vcx, |v, _| {
        let rows = texts(v.rows(&l, &jiff::tz::TimeZone::UTC));
        assert_eq!(rows[0], row("Client", "claude-code", Some("stdio")));
        assert_eq!(rows[3], row("As", "You", None));
    });
}

#[gpui::test]
fn the_chip_names_the_client_process(cx: &mut TestAppContext) {
    let req = request(send("hi"));
    let (view, vcx, _) = open_request(cx, req);
    let l = common::en();
    view.read_with(vcx, |v, _| {
        let rows = texts(v.rows(&l, &jiff::tz::TimeZone::UTC));
        assert_eq!(
            rows[0],
            row("Client", "claude-code", Some("stdio · pid 4242"))
        );
    });
}

#[gpui::test]
fn thread_send_names_thread_and_broadcast(cx: &mut TestAppContext) {
    let action = WriteAction::Send {
        conversation_name: Arc::from("general"),
        thread: Some(domain::MessageTs(Arc::from("1.0"))),
        broadcast: true,
        text: Arc::from("hi"),
    };
    let (view, vcx, _) = open(cx, action);
    let l = common::en();
    view.read_with(vcx, |v, _| {
        let rows = texts(v.rows(&l, &jiff::tz::TimeZone::UTC));
        assert_eq!(
            rows[2],
            row("Post to", "thread in #general", Some("in Acme"))
        );
        assert_eq!(rows[4], row("Also", "Sent to the conversation", None));
    });
}

#[gpui::test]
fn deny_is_default_focus_and_enter_denies(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, send("hi"));
    assert_eq!(focused(&view, vcx), Some(ConfirmButton::Deny));
    activate(vcx);
    arm(vcx);
    // Armed or not, Enter on the default focus never allows.
    key_down(vcx, "enter", false);
    key_up(vcx, "enter");
    assert_eq!(events.borrow().first(), Some(&ConfirmWriteEvent::Declined));
    assert!(!approved(&events), "{:?}", events.borrow());
}

#[gpui::test]
fn stray_typing_then_enter_never_allows(cx: &mut TestAppContext) {
    // Typing meant for another window lands in the prompt, then Enter.
    let (_view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    vcx.simulate_keystrokes("h e l l o");
    vcx.executor()
        .advance_clock(ARM_DELAY + Duration::from_millis(100));
    vcx.run_until_parked();
    key_down(vcx, "enter", false);
    key_up(vcx, "enter");
    assert!(!approved(&events), "{:?}", events.borrow());
}

#[gpui::test]
fn a_keystroke_restarts_the_arm_lock(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    arm(vcx);
    vcx.simulate_keystrokes("tab tab");
    assert_eq!(focused(&view, vcx), Some(ConfirmButton::AllowOnce));
    assert!(view.read_with(vcx, |v, _| v.can_approve()));
    // A stray key locks the allowing buttons again ...
    vcx.simulate_keystrokes("x");
    assert!(!view.read_with(vcx, |v, _| v.can_approve()));
    key_down(vcx, "enter", false);
    key_up(vcx, "enter");
    assert!(!approved(&events), "{:?}", events.borrow());
    // ... and Tab does not.
    arm(vcx);
    vcx.simulate_keystrokes("shift-tab tab");
    assert!(view.read_with(vcx, |v, _| v.can_approve()));
    // A deliberate Enter on the focused, armed Allow once allows once.
    key_down(vcx, "enter", false);
    key_up(vcx, "enter");
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::Approved]);
}

#[gpui::test]
fn allow_disabled_for_first_second(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, send("hi"));
    // The countdown starts when the window gains focus.
    vcx.executor().advance_clock(Duration::from_secs(5));
    vcx.run_until_parked();
    assert!(!view.read_with(vcx, |v, _| v.can_approve()));
    activate(vcx);
    vcx.executor().advance_clock(Duration::from_millis(500));
    vcx.run_until_parked();
    vcx.dispatch_action(ApproveWrite);
    vcx.dispatch_action(ApproveWriteForSession);
    assert!(events.borrow().is_empty());
    assert!(!view.read_with(vcx, |v, _| v.can_approve()));
    vcx.executor().advance_clock(Duration::from_millis(600));
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.can_approve()));
    vcx.dispatch_action(ApproveWrite);
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::Approved]);
}

#[gpui::test]
fn allow_disarms_when_window_loses_focus(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    vcx.executor().advance_clock(Duration::from_secs(2));
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.can_approve()));
    vcx.deactivate_window();
    vcx.run_until_parked();
    assert!(!view.read_with(vcx, |v, _| v.can_approve()));
    // Regaining focus starts the countdown again.
    activate(vcx);
    vcx.dispatch_action(ApproveWrite);
    assert!(events.borrow().is_empty());
    vcx.executor().advance_clock(Duration::from_millis(1100));
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.can_approve()));
}

#[gpui::test]
fn escape_denies(cx: &mut TestAppContext) {
    let (_view, vcx, events) = open(cx, send("hi"));
    vcx.simulate_keystrokes("escape");
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::Declined]);
    // The action works too.
    vcx.dispatch_action(DeclineWrite);
    assert_eq!(events.borrow().len(), 2);
}

#[gpui::test]
fn deny_click_denies_even_before_arming(cx: &mut TestAppContext) {
    let (_view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    let at = center(vcx, "confirm-write-deny");
    vcx.simulate_click(at, Modifiers::none());
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::Declined]);
}

#[gpui::test]
fn always_for_this_session_click_answers_for_the_session(cx: &mut TestAppContext) {
    let (_view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    let at = center(vcx, "confirm-write-allow-session");
    vcx.simulate_click(at, Modifiers::none());
    assert!(events.borrow().is_empty(), "locked before arming");
    arm(vcx);
    vcx.simulate_click(at, Modifiers::none());
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::ApprovedForSession]);
}

#[gpui::test]
fn allow_once_click_answers_once(cx: &mut TestAppContext) {
    let (_view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    arm(vcx);
    let at = center(vcx, "confirm-write-allow-once");
    vcx.simulate_click(at, Modifiers::none());
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::Approved]);
}

#[gpui::test]
fn tab_order_is_details_deny_session_allow(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    arm(vcx);
    assert_eq!(focused(&view, vcx), Some(ConfirmButton::Deny));
    vcx.simulate_keystrokes("shift-tab");
    assert_eq!(focused(&view, vcx), Some(ConfirmButton::Details));
    vcx.simulate_keystrokes("tab tab tab");
    assert_eq!(focused(&view, vcx), Some(ConfirmButton::AllowOnce));
    vcx.simulate_keystrokes("tab");
    assert_eq!(focused(&view, vcx), Some(ConfirmButton::Details));
    vcx.simulate_keystrokes("tab tab tab");
    // Enter on the focused session button answers for the session.
    vcx.simulate_keystrokes("shift-tab");
    key_down(vcx, "enter", false);
    key_up(vcx, "enter");
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::ApprovedForSession]);
}

#[gpui::test]
fn details_disclosure_opens_and_shows_the_arguments(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, send("hi\u{202e}"));
    assert!(!view.read_with(vcx, |v, _| v.details_open()));
    assert!(vcx.debug_bounds("confirm-write-details-body").is_none());
    let details = view.read_with(vcx, |v, _| v.details());
    assert!(details.contains("\"tool\": \"send_message\""), "{details}");
    assert!(
        details.contains("\"conversation\": \"general\""),
        "{details}"
    );
    assert!(details.contains("hi\\u{202e}"), "escaped: {details}");
    let at = center(vcx, "confirm-write-details");
    vcx.simulate_click(at, Modifiers::none());
    assert!(view.read_with(vcx, |v, _| v.details_open()));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("confirm-write-details-body").is_some());
    // Enter and Space on the focused toggle close and reopen it.
    assert_eq!(focused(&view, vcx), Some(ConfirmButton::Details));
    vcx.simulate_keystrokes("enter");
    assert!(!view.read_with(vcx, |v, _| v.details_open()));
    vcx.simulate_keystrokes("space");
    assert!(view.read_with(vcx, |v, _| v.details_open()));
    assert!(events.borrow().is_empty(), "the toggle never answers");
}

#[gpui::test]
fn titles_per_action(cx: &mut TestAppContext) {
    let l = common::en();
    let cases = [
        (send("a"), "An MCP client wants to send a message"),
        (
            WriteAction::Schedule {
                conversation_name: Arc::from("general"),
                thread: None,
                text: Arc::from("a"),
                at: jiff::Timestamp::from_second(1_800_000_000).unwrap(),
            },
            "An MCP client wants to schedule a message",
        ),
        (
            WriteAction::React {
                conversation_name: Arc::from("general"),
                message_excerpt: Arc::from("hello"),
                emoji: EmojiRef {
                    shortcode: "tada".into(),
                    unicode: None,
                    skin_tone: None,
                },
            },
            "An MCP client wants to add a reaction",
        ),
        (
            WriteAction::SetStatus {
                status: Some(Status {
                    emoji: None,
                    text: Arc::from("Focus"),
                    expires: None,
                }),
            },
            "An MCP client wants to change your status",
        ),
    ];
    for (action, title) in cases {
        let (view, vcx, _) = open(cx, action);
        assert_eq!(view.read_with(vcx, |v, _| v.title(&l)), title);
    }
}

#[gpui::test]
fn status_rows_name_the_workspace(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, WriteAction::SetStatus { status: None });
    let l = common::en();
    view.read_with(vcx, |v, _| {
        let rows = texts(v.rows(&l, &jiff::tz::TimeZone::UTC));
        assert_eq!(rows[1], row("Tool", "set_status", None));
        assert_eq!(rows[2], row("Workspace", "Acme", None));
        assert_eq!(rows[4], row("Status", "Clears your status", None));
    });
}

#[gpui::test]
fn schedule_time_uses_the_given_zone(cx: &mut TestAppContext) {
    let action = WriteAction::Schedule {
        conversation_name: Arc::from("general"),
        thread: None,
        text: Arc::from("hi"),
        at: jiff::Timestamp::from_second(0).unwrap(),
    };
    let (view, vcx, _) = open(cx, action);
    let l = common::en();
    let tokyo = jiff::tz::TimeZone::get("Asia/Tokyo").unwrap();
    view.read_with(vcx, |v, _| {
        let utc = v.rows(&l, &jiff::tz::TimeZone::UTC);
        let jst = v.rows(&l, &tokyo);
        assert_eq!(utc[4].label, "At");
        assert_ne!(utc[4], jst[4]);
        assert!(jst[4].value.contains("09:00"), "{:?}", jst[4]);
    });
}

#[gpui::test]
fn space_on_focused_deny_denies(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    arm(vcx);
    assert_eq!(focused(&view, vcx), Some(ConfirmButton::Deny));
    vcx.simulate_keystrokes("space");
    assert_eq!(*events.borrow(), [ConfirmWriteEvent::Declined]);
}

fn activate(vcx: &mut VisualTestContext) {
    vcx.update(|w, _| w.activate_window());
    vcx.run_until_parked();
}

fn arm(vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(Duration::from_millis(1100));
    vcx.run_until_parked();
}

fn center(vcx: &mut VisualTestContext, sel: &'static str) -> gpui::Point<gpui::Pixels> {
    vcx.debug_bounds(sel).expect("drawn").center()
}

fn key_down(vcx: &mut VisualTestContext, key: &'static str, held: bool) {
    vcx.simulate_event(KeyDownEvent {
        keystroke: Keystroke::parse(key).unwrap(),
        is_held: held,
        prefer_character_input: false,
    });
}

fn key_up(vcx: &mut VisualTestContext, key: &'static str) {
    vcx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse(key).unwrap(),
    });
}

fn approved(events: &Events) -> bool {
    events.borrow().iter().any(|e| {
        matches!(
            e,
            ConfirmWriteEvent::Approved | ConfirmWriteEvent::ApprovedForSession
        )
    })
}

#[gpui::test]
fn mouse_down_before_arming_cannot_allow_after(cx: &mut TestAppContext) {
    for sel in ["confirm-write-allow-once", "confirm-write-allow-session"] {
        let (_view, vcx, events) = open(cx, send("hi"));
        activate(vcx);
        let at = center(vcx, sel);
        vcx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
        arm(vcx);
        vcx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
        assert!(!approved(&events), "{sel}: {:?}", events.borrow());
    }
}

#[gpui::test]
fn enter_down_before_arming_cannot_allow_after(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    arm(vcx);
    vcx.simulate_keystrokes("tab tab");
    assert_eq!(focused(&view, vcx), Some(ConfirmButton::AllowOnce));
    // Lose and regain focus: the lock restarts with Enter already down.
    vcx.deactivate_window();
    activate(vcx);
    key_down(vcx, "enter", false);
    arm(vcx);
    key_up(vcx, "enter");
    assert!(!approved(&events), "{:?}", events.borrow());
}

#[gpui::test]
fn held_enter_on_armed_allow_never_allows(cx: &mut TestAppContext) {
    let (_view, vcx, events) = open(cx, send("hi"));
    activate(vcx);
    arm(vcx);
    vcx.simulate_keystrokes("tab tab");
    key_down(vcx, "enter", true);
    key_up(vcx, "enter");
    key_down(vcx, "space", true);
    key_up(vcx, "space");
    assert!(!approved(&events), "{:?}", events.borrow());
}

#[gpui::test]
fn enter_variants_on_focused_deny_never_allow(cx: &mut TestAppContext) {
    for key in ["enter", "shift-enter", "cmd-enter"] {
        let (view, vcx, events) = open(cx, send("hi"));
        activate(vcx);
        arm(vcx);
        assert_eq!(focused(&view, vcx), Some(ConfirmButton::Deny));
        key_down(vcx, key, false);
        key_up(vcx, key);
        assert!(!approved(&events), "{key}: {:?}", events.borrow());
    }
}

#[gpui::test]
fn modified_enter_on_allow_never_allows(cx: &mut TestAppContext) {
    for key in ["shift-enter", "cmd-enter"] {
        let (_view, vcx, events) = open(cx, send("hi"));
        activate(vcx);
        arm(vcx);
        vcx.simulate_keystrokes("tab tab");
        key_down(vcx, key, false);
        key_up(vcx, key);
        assert!(!approved(&events), "{key}: {:?}", events.borrow());
    }
}

#[gpui::test]
fn details_use_the_buffer_font_after_a_reload(cx: &mut TestAppContext) {
    use gpui::BorrowAppContext as _;
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.settings, "{}").unwrap();
    let file = paths.settings.clone();
    cx.update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    let (view, vcx, _) = open(cx, send("hi"));
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).code_font(cx).to_string()),
        "JetBrains Mono"
    );
    std::fs::write(&file, r#"{"buffer_font_family": "Menlo"}"#).unwrap();
    vcx.update(|_, cx| {
        cx.update_global::<settings::SettingsStore, _>(|s, _| {
            s.reload();
        })
    });
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| view.read(cx).code_font(cx).to_string()),
        "Menlo"
    );
}

#[gpui::test]
fn ui_font_follows_a_settings_reload(cx: &mut TestAppContext) {
    use gpui::BorrowAppContext as _;
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.settings, "{}").unwrap();
    let file = paths.settings.clone();
    cx.update(|cx| {
        cx.set_global(settings::SettingsStore::load(paths));
        components::live_settings::init_ui_font(cx);
    });
    let (f, size) = cx.update(|cx| components::live_settings::ui_font(cx));
    assert_eq!(
        (f.to_string(), f32::from(size)),
        ("Inter".to_string(), 14.0)
    );
    std::fs::write(&file, r#"{"ui_font_family": "Menlo", "ui_font_size": 40}"#).unwrap();
    cx.update(|cx| {
        cx.update_global::<settings::SettingsStore, _>(|s, _| {
            s.reload();
        })
    });
    let (f, size) = cx.update(|cx| components::live_settings::ui_font(cx));
    assert_eq!(
        (f.to_string(), f32::from(size)),
        ("Menlo".to_string(), 24.0)
    );
}

#[gpui::test]
fn tab_never_lands_on_an_unarmed_allow(cx: &mut TestAppContext) {
    let (view, vcx, _events) = open(cx, send("hi"));
    activate(vcx);
    assert!(!view.read_with(vcx, |v, _| v.can_approve()));
    for _ in 0..8 {
        vcx.simulate_keystrokes("tab");
        let at = focused(&view, vcx);
        assert!(
            !matches!(
                at,
                Some(ConfirmButton::AllowOnce | ConfirmButton::AllowSession)
            ),
            "Tab reached a disabled Allow: {at:?}"
        );
    }
    for _ in 0..8 {
        vcx.simulate_keystrokes("shift-tab");
        let at = focused(&view, vcx);
        assert!(
            !matches!(
                at,
                Some(ConfirmButton::AllowOnce | ConfirmButton::AllowSession)
            ),
            "Shift-Tab reached a disabled Allow: {at:?}"
        );
    }
}

#[gpui::test]
fn the_allow_buttons_say_unavailable_until_armed(cx: &mut TestAppContext) {
    let (view, vcx, _events) = open(cx, send("hi"));
    activate(vcx);
    let l = common::en();
    let label = |vcx: &mut VisualTestContext, button| {
        view.read_with(vcx, |v, _| v.allow_a11y_label(button, &l))
    };
    assert_eq!(
        label(vcx, ConfirmButton::AllowOnce),
        "Allow once, unavailable"
    );
    assert!(label(vcx, ConfirmButton::AllowSession).ends_with(", unavailable"));
    arm(vcx);
    assert_eq!(label(vcx, ConfirmButton::AllowOnce), "Allow once");
}
