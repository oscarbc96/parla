use std::cell::RefCell;
use std::rc::Rc;

use components::{
    CommandDelegate, CommandItem, CommandPalette, PickerDelegate as _, PickerEvent,
    is_palette_action, palette_action_names,
};
use gpui::{Entity, Focusable as _, SharedString, TestAppContext, VisualTestContext};

fn item(name: &str, title: &str, binding: Option<&str>) -> CommandItem {
    CommandItem {
        name: SharedString::from(name.to_string()),
        title: SharedString::from(title.to_string()),
        binding: binding.map(|b| SharedString::from(b.to_string())),
    }
}

fn sample_items() -> Vec<CommandItem> {
    vec![
        item("app::Quit", "Quit Parla", Some("cmd-q")),
        item("app::OpenSettings", "Open settings", Some("cmd-,")),
        item("workspace::QuickSwitch", "Quick switch", Some("cmd-k")),
        item("theme::ToggleMode", "Toggle light and dark mode", None),
    ]
}

type Events = Rc<RefCell<Vec<PickerEvent>>>;

fn setup(cx: &mut TestAppContext) -> (Entity<CommandPalette>, &mut VisualTestContext, Events) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (view, vcx) =
        cx.add_window_view(|_, cx| CommandPalette::new(CommandDelegate::new(sample_items()), cx));
    let events: Events = Rc::new(RefCell::new(Vec::new()));
    let e = events.clone();
    vcx.update(|w, cx| {
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx);
        cx.subscribe(&view, move |_, ev: &PickerEvent, _| {
            e.borrow_mut().push(ev.clone())
        })
        .detach();
    });
    vcx.run_until_parked();
    (view, vcx, events)
}

fn shown(view: &Entity<CommandPalette>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| {
        v.delegate()
            .matches()
            .iter()
            .map(|i| i.title.to_string())
            .collect()
    })
}

#[test]
fn palette_excludes_internal_actions() {
    assert!(!is_palette_action("text_input::Left"));
    assert!(!is_palette_action("text_input::Paste"));
    assert!(!is_palette_action("picker::Confirm"));
    assert!(!is_palette_action("picker::Dismiss"));
    assert!(is_palette_action("app::Quit"));
    assert!(is_palette_action("workspace::QuickSwitch"));
    let names = palette_action_names(&[
        "app::Quit",
        "text_input::Left",
        "picker::Confirm",
        "theme::ToggleMode",
    ]);
    assert_eq!(names, vec!["app::Quit", "theme::ToggleMode"]);
}

#[gpui::test]
fn opening_lists_actions(cx: &mut TestAppContext) {
    let (view, vcx, _) = setup(cx);
    assert_eq!(
        shown(&view, vcx),
        [
            "Quit Parla",
            "Open settings",
            "Quick switch",
            "Toggle light and dark mode",
        ]
    );
}

#[gpui::test]
fn filtering_narrows_the_list(cx: &mut TestAppContext) {
    let (view, vcx, _) = setup(cx);
    vcx.simulate_input("settings");
    assert_eq!(shown(&view, vcx), ["Open settings"]);
    vcx.simulate_keystrokes(
        "backspace backspace backspace backspace backspace backspace backspace backspace",
    );
    assert_eq!(shown(&view, vcx).len(), 4);
}

#[gpui::test]
fn enter_dispatches_the_chosen_action(cx: &mut TestAppContext) {
    let (view, vcx, events) = setup(cx);
    vcx.simulate_input("quick");
    vcx.simulate_keystrokes("enter");
    assert_eq!(*events.borrow(), [PickerEvent::Confirmed(0)]);
    view.read_with(vcx, |v, _| {
        let c = v.delegate().chosen().expect("chosen");
        assert_eq!(c.name.as_ref(), "workspace::QuickSwitch");
    });
}

#[gpui::test]
fn escape_closes(cx: &mut TestAppContext) {
    let (_view, vcx, events) = setup(cx);
    vcx.simulate_keystrokes("escape");
    assert_eq!(*events.borrow(), [PickerEvent::Dismissed]);
}
