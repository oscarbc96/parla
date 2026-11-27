//! The call bar (Connecting, Live, Mute, Leave, Show call window, Open in
//! browser) and the header's Start or Join huddle button.
mod common;

use std::sync::Arc;

use components::{
    CallBar, CallBarEvent, CallBarModel, CallBarPhase, CallPerson, DeviceMenu, HuddleAction,
    HuddleBar, HuddleSource, MediaState, MuteControl,
};
use domain::test_support::{cid, member, uid};
use domain::{ConversationId, Huddle, HuddleId, Member, UserId, WorkspaceState};
use gpui::{App, AppContext as _, Entity, TestAppContext};
use settings::WorkspaceKey;

fn model() -> CallBarModel {
    CallBarModel {
        title: "general".into(),
        phase: CallBarPhase::Live,
        mic: MediaState::Active,
        camera: MediaState::Inactive,
        participants: 3,
        load_slow: false,
        screen_share_unavailable: false,
        show_window: true,
        people: Vec::new(),
        devices: None,
    }
}

fn person(name: &str, speaking: bool) -> CallPerson {
    CallPerson {
        name: name.to_string().into(),
        speaking,
        muted: false,
    }
}

fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
}

fn bar(
    cx: &mut TestAppContext,
    m: CallBarModel,
) -> (Entity<CallBar>, &mut gpui::VisualTestContext) {
    setup(cx);
    cx.add_window_view(move |_, cx| CallBar::new(m, cx))
}

#[gpui::test]
fn connecting_until_live(cx: &mut TestAppContext) {
    let mut m = model();
    m.phase = CallBarPhase::Loading;
    let (b, vcx) = bar(cx, m);
    let v = vcx.update(|_, cx| b.read(cx).layout(cx));
    assert_eq!(v.status, "Connecting...");
    b.update(vcx, |b, cx| b.set_model(model(), cx));
    let v = vcx.update(|_, cx| b.read(cx).layout(cx));
    assert_eq!(v.status, "3 in the huddle");
}

#[gpui::test]
fn mute_reflects_capture_state(cx: &mut TestAppContext) {
    let (b, vcx) = bar(cx, model());
    let v = vcx.update(|_, cx| b.read(cx).layout(cx));
    assert_eq!(
        v.mute,
        MuteControl::Button {
            label: "Mute".into(),
            muted: false
        }
    );
    let mut m = model();
    m.mic = MediaState::Muted;
    b.update(vcx, |b, cx| b.set_model(m, cx));
    let v = vcx.update(|_, cx| b.read(cx).layout(cx));
    assert_eq!(
        v.mute,
        MuteControl::Button {
            label: "Unmute".into(),
            muted: true
        }
    );
}

#[gpui::test]
fn mute_hidden_when_unsupported(cx: &mut TestAppContext) {
    let mut m = model();
    m.mic = MediaState::Unsupported;
    m.camera = MediaState::Active;
    let (b, vcx) = bar(cx, m);
    let v = vcx.update(|_, cx| b.read(cx).layout(cx));
    assert_eq!(
        v.mute,
        MuteControl::InCallWindow("Mute in the call window".into())
    );
    // The camera is an indicator only.
    assert_eq!(v.camera_on, Some(true));
}

#[gpui::test]
fn native_call_has_no_window_button(cx: &mut TestAppContext) {
    let (b, vcx) = bar(cx, model());
    assert!(vcx.update(|_, cx| b.read(cx).layout(cx)).show_window);
    let mut m = model();
    m.show_window = false;
    b.update(vcx, |b, cx| b.set_model(m, cx));
    assert!(!vcx.update(|_, cx| b.read(cx).layout(cx)).show_window);
}

#[gpui::test]
fn slow_load_offers_browser(cx: &mut TestAppContext) {
    let (b, vcx) = bar(cx, model());
    assert!(!vcx.update(|_, cx| b.read(cx).layout(cx)).open_in_browser);
    let mut m = model();
    m.load_slow = true;
    b.update(vcx, |b, cx| b.set_model(m, cx));
    assert!(vcx.update(|_, cx| b.read(cx).layout(cx)).open_in_browser);
    let mut m = model();
    m.screen_share_unavailable = true;
    b.update(vcx, |b, cx| b.set_model(m, cx));
    assert!(vcx.update(|_, cx| b.read(cx).layout(cx)).open_in_browser);
}

#[gpui::test]
fn buttons_emit_events(cx: &mut TestAppContext) {
    let (b, vcx) = bar(cx, model());
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&b, move |_, e: &CallBarEvent, _| {
            s.lock().unwrap().push(e.clone())
        })
        .detach();
    });
    b.update(vcx, |b, cx| {
        b.toggle_mute(cx);
        b.show_window(cx);
        b.open_in_browser(cx);
        b.leave(cx);
    });
    assert_eq!(
        *seen.lock().unwrap(),
        [
            CallBarEvent::ToggleMute,
            CallBarEvent::ShowWindow,
            CallBarEvent::OpenInBrowser,
            CallBarEvent::Leave
        ]
    );
}

#[gpui::test]
fn people_show_as_avatars_and_the_speaker_is_named(cx: &mut TestAppContext) {
    let mut m = model();
    m.people = vec![
        person("Ada Lovelace", false),
        person("Bob Reyes", true),
        person("Cy", false),
    ];
    let (b, vcx) = bar(cx, m);
    let v = vcx.update(|_, cx| b.read(cx).layout(cx));
    let avatars: Vec<_> = v
        .avatars
        .iter()
        .map(|a| (a.initials.to_string(), a.speaking))
        .collect();
    assert_eq!(
        avatars,
        [
            ("AL".to_string(), false),
            ("BR".to_string(), true),
            ("CY".to_string(), false)
        ]
    );
    assert_eq!(v.more, 0);
    // Speaking is said in words too, not by the ring colour alone.
    assert_eq!(v.status, "Bob Reyes is speaking");
    // Nobody speaking: the count again.
    let mut m = model();
    m.people = vec![person("Ada Lovelace", false)];
    b.update(vcx, |b, cx| b.set_model(m, cx));
    assert_eq!(
        vcx.update(|_, cx| b.read(cx).layout(cx)).status,
        "3 in the huddle"
    );
}

#[gpui::test]
fn many_people_collapse_into_more(cx: &mut TestAppContext) {
    let mut m = model();
    m.people = (0..8).map(|i| person(&format!("P{i}"), false)).collect();
    let (b, vcx) = bar(cx, m);
    let v = vcx.update(|_, cx| b.read(cx).layout(cx));
    assert_eq!(v.avatars.len(), components::call_bar::MAX_CALL_AVATARS);
    assert_eq!(v.more, 8 - components::call_bar::MAX_CALL_AVATARS);
}

#[gpui::test]
fn device_picker_lists_default_and_devices(cx: &mut TestAppContext) {
    let mut m = model();
    m.devices = Some(DeviceMenu {
        open: true,
        inputs: vec!["Built-in Microphone".into(), "USB Mic".into()],
        outputs: vec!["Built-in Speakers".into()],
        input: Some("USB Mic".into()),
        output: None,
    });
    let (b, vcx) = bar(cx, m);
    let v = vcx.update(|_, cx| b.read(cx).layout(cx));
    let menu = v.devices.expect("the picker is offered");
    assert!(menu.open);
    let inputs: Vec<_> = menu
        .inputs
        .iter()
        .map(|r| (r.label.clone(), r.selected))
        .collect();
    assert_eq!(
        inputs,
        [
            ("System default".to_string(), false),
            ("Built-in Microphone".to_string(), false),
            ("USB Mic".to_string(), true)
        ]
    );
    assert_eq!(menu.outputs[0].label, "System default");
    assert!(menu.outputs[0].selected);
    // The window path has no picker.
    b.update(vcx, |b, cx| b.set_model(model(), cx));
    assert!(vcx.update(|_, cx| b.read(cx).layout(cx)).devices.is_none());
}

#[gpui::test]
fn device_picks_emit_events(cx: &mut TestAppContext) {
    let (b, vcx) = bar(cx, model());
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&b, move |_, e: &CallBarEvent, _| {
            s.lock().unwrap().push(e.clone())
        })
        .detach();
    });
    b.update(vcx, |b, cx| {
        b.toggle_devices(cx);
        b.pick_input(Some("USB Mic".into()), cx);
        b.pick_output(None, cx);
    });
    assert_eq!(
        *seen.lock().unwrap(),
        [
            CallBarEvent::ToggleDevices,
            CallBarEvent::PickInput(Some("USB Mic".into())),
            CallBarEvent::PickOutput(None),
        ]
    );
}

fn picker_model(open: bool) -> CallBarModel {
    let mut m = model();
    m.devices = Some(DeviceMenu {
        open,
        inputs: vec!["USB Mic".into()],
        outputs: vec![],
        input: None,
        output: None,
    });
    m
}

fn recorder(
    b: &Entity<CallBar>,
    vcx: &mut gpui::VisualTestContext,
) -> Arc<std::sync::Mutex<Vec<CallBarEvent>>> {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    vcx.update(|_, cx| {
        cx.subscribe(b, move |_, e: &CallBarEvent, _| {
            s.lock().unwrap().push(e.clone())
        })
        .detach();
    });
    seen
}

fn click_on(vcx: &mut gpui::VisualTestContext, id: &'static str) {
    let at = vcx
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("{id} is drawn"));
    vcx.simulate_click(at.center(), gpui::Modifiers::none());
}

#[gpui::test]
fn avatars_name_the_person_and_say_muted(cx: &mut TestAppContext) {
    let mut m = model();
    m.people = vec![
        CallPerson {
            name: "Ada Lovelace".into(),
            speaking: false,
            muted: true,
        },
        person("Bob Reyes", false),
    ];
    let (b, vcx) = bar(cx, m);
    let v = vcx.update(|_, cx| b.read(cx).layout(cx));
    assert_eq!(v.avatars[0].label, "Ada Lovelace, muted");
    assert_eq!(v.avatars[1].label, "Bob Reyes");
    assert!(vcx.debug_bounds("call-bar-avatar-0-muted").is_some());
    assert!(vcx.debug_bounds("call-bar-avatar-1-muted").is_none());
}

#[gpui::test]
fn the_picker_is_keyboard_operable(cx: &mut TestAppContext) {
    let (b, vcx) = bar(cx, picker_model(false));
    let seen = recorder(&b, vcx);
    // Devices takes the focus; Enter works it like a click.
    click_on(vcx, "call-bar-devices");
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        *seen.lock().unwrap(),
        [CallBarEvent::ToggleDevices, CallBarEvent::ToggleDevices]
    );
    // Opening moves the focus to the first row; Enter picks it.
    b.update(vcx, |b, cx| b.set_model(picker_model(true), cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    assert_eq!(seen.lock().unwrap()[2], CallBarEvent::PickInput(None));
    // Escape closes.
    b.update(vcx, |b, cx| b.set_model(picker_model(false), cx));
    b.update(vcx, |b, cx| b.set_model(picker_model(true), cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("escape");
    assert_eq!(
        seen.lock().unwrap().last(),
        Some(&CallBarEvent::CloseDevices)
    );
}

#[gpui::test]
fn the_picker_closes_on_a_click_outside(cx: &mut TestAppContext) {
    let (b, vcx) = bar(cx, picker_model(true));
    let seen = recorder(&b, vcx);
    click_on(vcx, "call-bar-status");
    assert_eq!(*seen.lock().unwrap(), [CallBarEvent::CloseDevices]);
    // A click on Devices while open closes it, without opening it again.
    seen.lock().unwrap().clear();
    click_on(vcx, "call-bar-devices");
    assert_eq!(*seen.lock().unwrap(), [CallBarEvent::CloseDevices]);
}

// header

struct Src {
    state: WorkspaceState,
    start_ok: bool,
    in_call: bool,
    joined: Vec<ConversationId>,
}

impl HuddleSource for Src {
    fn huddle(&self, c: &ConversationId) -> Option<&Huddle> {
        self.state.huddles.get(c)
    }
    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn join_huddle(&mut self, c: ConversationId, _: &mut App) {
        self.joined.push(c);
    }
    fn can_start_huddle(&self) -> bool {
        self.start_ok
    }
    fn in_call_here(&self, _: &ConversationId, _: &App) -> bool {
        self.in_call
    }
}

fn header(cx: &mut TestAppContext, live: bool, in_call: bool) -> Entity<Src> {
    setup(cx);
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    state.self_user = Some(uid("u1"));
    state.members.insert(uid("u2"), member("u2", "Bob"));
    if live {
        state.huddles.insert(
            cid("C1"),
            Huddle {
                id: HuddleId(Arc::from("H1")),
                conversation: cid("C1"),
                participants: vec![uid("u2")].into(),
                started: 0,
                created_by: None,
                is_dm: false,
            },
        );
    }
    cx.new(|_| Src {
        state,
        start_ok: true,
        in_call,
        joined: vec![],
    })
}

#[gpui::test]
fn header_says_start_without_huddle(cx: &mut TestAppContext) {
    let s = header(cx, false, false);
    let s2 = s.clone();
    let (b, vcx) = cx.add_window_view(move |_, cx| HuddleBar::new(s2, cid("C1"), cx));
    let a = vcx.update(|_, cx| b.read(cx).action(cx));
    assert_eq!(a, Some(HuddleAction::Start));
    assert_eq!(
        a.map(|a| a.label(&common::en())),
        Some("Start huddle".into())
    );
    b.update(vcx, |b, cx| b.start(cx));
    s.read_with(vcx, |s, _| assert_eq!(s.joined, [cid("C1")]));
    // Without the capability nothing is offered.
    s.update(vcx, |s, _| s.start_ok = false);
    assert_eq!(vcx.update(|_, cx| b.read(cx).action(cx)), None);
}

#[gpui::test]
fn header_join_hidden_while_in_this_call(cx: &mut TestAppContext) {
    let s = header(cx, true, false);
    let s2 = s.clone();
    let (b, vcx) = cx.add_window_view(move |_, cx| HuddleBar::new(s2, cid("C1"), cx));
    assert_eq!(
        vcx.update(|_, cx| b.read(cx).action(cx)),
        Some(HuddleAction::Join)
    );
    s.update(vcx, |s, _| s.in_call = true);
    assert_eq!(vcx.update(|_, cx| b.read(cx).action(cx)), None);
    // A join or start the user cannot see does nothing.
    b.update(vcx, |b, cx| {
        b.join(cx);
        b.start(cx);
    });
    s.read_with(vcx, |s, _| assert!(s.joined.is_empty()));
    // Neither is Start offered while this conversation's call is open.
    let s = header(cx, false, true);
    let (b, vcx) = cx.add_window_view(move |_, cx| HuddleBar::new(s, cid("C1"), cx));
    assert_eq!(vcx.update(|_, cx| b.read(cx).action(cx)), None);
}
