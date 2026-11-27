//! The keyboard shortcuts sheet: groups, the filter field, Esc and the
//! keymap button.

use std::cell::RefCell;
use std::rc::Rc;

use components::shortcuts_sheet::{
    ShortcutGroup, ShortcutItem, ShortcutsSheet, ShortcutsSheetEvent,
};
use gpui::{Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext};

fn item(label: &str, keystrokes: &str, caps: &[&str]) -> ShortcutItem {
    ShortcutItem {
        label: label.to_string().into(),
        keystrokes: keystrokes.to_string().into(),
        keys: caps.iter().map(|c| c.to_string().into()).collect(),
    }
}

fn groups() -> Vec<ShortcutGroup> {
    vec![
        ShortcutGroup {
            title: "Navigation".into(),
            items: vec![
                item("Jump to", "cmd-k", &["\u{2318}", "K"]),
                item("Back", "cmd-[", &["\u{2318}", "["]),
            ],
        },
        ShortcutGroup {
            title: "Composer".into(),
            items: vec![item("Bold", "cmd-b", &["\u{2318}", "B"])],
        },
    ]
}

type Events = Rc<RefCell<Vec<ShortcutsSheetEvent>>>;

fn setup(cx: &mut TestAppContext) -> (Entity<ShortcutsSheet>, &mut VisualTestContext, Events) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(
            cx,
            &[components::catalog()],
            i18n::LocalePref::Code("en".parse().unwrap()),
        );
        let text = include_str!("../../../assets/keymaps/default-macos.json");
        let blocks = settings::parse_keymap(std::path::Path::new("k"), text).unwrap();
        let (bindings, _) = settings::resolve_bindings(&blocks, &[], &|_| true);
        let mut keys = Vec::new();
        for b in bindings {
            if let Ok(action) = cx.build_action(&b.action, None) {
                let pred = b
                    .context
                    .as_deref()
                    .and_then(|c| gpui::KeyBindingContextPredicate::parse(c).ok())
                    .map(Rc::new);
                if let Ok(k) = gpui::KeyBinding::load(&b.keystrokes, action, pred, false, None, cx)
                {
                    keys.push(k);
                }
            }
        }
        cx.bind_keys(keys);
    });
    let (view, vcx) = cx.add_window_view(|_, cx| ShortcutsSheet::new(groups(), "cmd-/", true, cx));
    let events: Events = Rc::new(RefCell::new(Vec::new()));
    let e = events.clone();
    vcx.update(|w, cx| {
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx);
        cx.subscribe(&view, move |_, ev: &ShortcutsSheetEvent, _| {
            e.borrow_mut().push(*ev)
        })
        .detach();
    });
    vcx.run_until_parked();
    (view, vcx, events)
}

fn shown(view: &Entity<ShortcutsSheet>, vcx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(vcx, |v, cx| v.visible_labels(cx))
}

#[gpui::test]
fn lists_every_group_and_filters_as_you_type(cx: &mut TestAppContext) {
    let (view, vcx, _) = setup(cx);
    assert_eq!(shown(&view, vcx), ["Jump to", "Back", "Bold"]);
    assert!(vcx.debug_bounds("shortcuts-group-Navigation").is_some());
    assert!(vcx.debug_bounds("shortcuts-filter").is_some());
    vcx.simulate_input("bold");
    vcx.run_until_parked();
    assert_eq!(shown(&view, vcx), ["Bold"]);
    assert!(vcx.debug_bounds("shortcuts-group-Navigation").is_none());
    view.update_in(vcx, |v, w, cx| v.set_query("cmd-[", w, cx));
    vcx.run_until_parked();
    assert_eq!(shown(&view, vcx), ["Back"]);
    view.update_in(vcx, |v, w, cx| v.set_query("nothing like it", w, cx));
    vcx.run_until_parked();
    assert!(shown(&view, vcx).is_empty());
    assert!(vcx.debug_bounds("shortcuts-empty").is_some());
}

#[gpui::test]
fn escape_dismisses(cx: &mut TestAppContext) {
    let (_view, vcx, events) = setup(cx);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert_eq!(*events.borrow(), [ShortcutsSheetEvent::Dismissed]);
}

#[gpui::test]
fn close_and_keymap_buttons_are_tab_stops(cx: &mut TestAppContext) {
    let (_view, vcx, events) = setup(cx);
    // Filter field, then Close, then Edit keymap.
    vcx.simulate_keystrokes("tab");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(*events.borrow(), [ShortcutsSheetEvent::Dismissed]);
    let b = vcx.debug_bounds("shortcuts-edit-keymap").expect("drawn");
    vcx.simulate_click(b.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        [
            ShortcutsSheetEvent::Dismissed,
            ShortcutsSheetEvent::EditKeymap
        ]
    );
}
