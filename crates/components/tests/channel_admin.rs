//! The channel admin dialogs: the provider's name rules, the edit dialog's
//! changed fields, the confirm's safe default, Enter in the invite picker and
//! the browser's row text.
mod common;

use std::cell::RefCell;
use std::rc::Rc;

use components::{
    BrowseRow, ChannelBrowser, ConfirmDialog, ConfirmEvent, EditChannelDialog, EditChannelEvent,
    EditField, InviteDialog, InviteEvent, InviteRow, NameCheck,
};
use domain::test_support::{cid, uid};
use domain::{ChannelName, NameProblem};
use gpui::{
    Entity, EventEmitter, Focusable, KeyDownEvent, Keystroke, TestAppContext, VisualTestContext,
};

/// Lowercases; refuses `!`.
fn check() -> NameCheck {
    Rc::new(|typed: &str| {
        let name = typed.trim().to_lowercase();
        let problem = if name.is_empty() {
            Some(NameProblem::Empty)
        } else if name.contains('!') {
            Some(NameProblem::Characters)
        } else {
            None
        };
        ChannelName { name, problem }
    })
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
}

fn events<V: EventEmitter<E> + 'static, E: Clone + 'static>(
    v: &Entity<V>,
    vcx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<E>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    vcx.update(|_, cx| {
        cx.subscribe(v, move |_, e: &E, _| l.borrow_mut().push(e.clone()))
            .detach()
    });
    log
}

fn focus<V: Focusable>(v: &Entity<V>, vcx: &mut VisualTestContext) {
    vcx.update(|w, cx| {
        let h = v.read(cx).focus_handle(cx);
        w.focus(&h, cx);
    });
    vcx.run_until_parked();
}

fn key(vcx: &mut VisualTestContext, k: &'static str, held: bool) {
    vcx.simulate_event(KeyDownEvent {
        keystroke: Keystroke::parse(k).unwrap(),
        is_held: held,
        prefer_character_input: false,
    });
}

#[gpui::test]
fn edit_renames_only_when_the_name_was_typed_in(cx: &mut TestAppContext) {
    init(cx);
    // The stored name differs from what the check makes of it: untouched, it
    // must not be sent back as a rename.
    let (d, vcx) =
        cx.add_window_view(|_, cx| EditChannelDialog::new("Launch", "old", "", true, check(), cx));
    let log = events::<_, EditChannelEvent>(&d, vcx);
    d.update(vcx, |d, cx| {
        d.set_field(EditField::Topic, "new", cx);
        d.save(cx);
    });
    assert_eq!(
        log.borrow().last(),
        Some(&EditChannelEvent::Save {
            name: None,
            topic: Some("new".into()),
            purpose: None,
        })
    );
    d.update(vcx, |d, cx| {
        d.set_field(EditField::Name, "Launch Day", cx);
        d.save(cx);
    });
    assert_eq!(
        log.borrow().last(),
        Some(&EditChannelEvent::Save {
            name: Some("launch day".into()),
            topic: Some("new".into()),
            purpose: None,
        })
    );
}

#[gpui::test]
fn edit_uses_the_providers_name_rules(cx: &mut TestAppContext) {
    init(cx);
    let (d, vcx) =
        cx.add_window_view(|_, cx| EditChannelDialog::new("launch", "", "", true, check(), cx));
    let log = events::<_, EditChannelEvent>(&d, vcx);
    d.update(vcx, |d, cx| {
        d.set_field(EditField::Name, "launch!", cx);
        assert_eq!(d.problem(cx), Some(NameProblem::Characters));
        d.save(cx);
    });
    assert!(log.borrow().is_empty());
}

#[gpui::test]
fn confirm_starts_on_cancel_and_ignores_held_or_modified_enter(cx: &mut TestAppContext) {
    init(cx);
    let (d, vcx) = cx.add_window_view(|_, cx| {
        ConfirmDialog::new("Archive #x?".into(), "body".into(), "Archive".into(), cx)
    });
    vcx.run_until_parked();
    let log = events::<_, ConfirmEvent>(&d, vcx);
    focus(&d, vcx);
    // A held Enter (from the menu that opened it) and Cmd+Enter do nothing.
    key(vcx, "enter", true);
    key(vcx, "cmd-enter", false);
    assert!(log.borrow().is_empty());
    // A fresh Enter activates the focused button: Cancel.
    key(vcx, "enter", false);
    assert_eq!(*log.borrow(), [ConfirmEvent::Cancel]);
}

#[gpui::test]
fn delete_confirm_shows_the_preview_and_confirms_on_enter_or_delete(cx: &mut TestAppContext) {
    init(cx);
    let (d, vcx) = cx.add_window_view(|_, cx| {
        ConfirmDialog::new("Delete message?".into(), String::new(), "Delete".into(), cx)
            .danger(true)
            .preview("delete me".into())
            .delete_confirms()
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("confirm-preview").is_some());
    let log = events::<_, ConfirmEvent>(&d, vcx);
    // The host focuses Delete (not Cancel): Enter and Delete confirm.
    vcx.update(|w, cx| {
        let h = d.read(cx).confirm_focus();
        w.focus(&h, cx);
    });
    vcx.run_until_parked();
    key(vcx, "enter", false);
    assert_eq!(*log.borrow(), [ConfirmEvent::Confirm]);
    log.borrow_mut().clear();
    key(vcx, "delete", false);
    assert_eq!(*log.borrow(), [ConfirmEvent::Confirm]);
    log.borrow_mut().clear();
    if cfg!(target_os = "macos") {
        key(vcx, "cmd-backspace", false);
        assert_eq!(*log.borrow(), [ConfirmEvent::Confirm]);
        log.borrow_mut().clear();
    }
    key(vcx, "escape", false);
    assert_eq!(*log.borrow(), [ConfirmEvent::Cancel]);
}

#[gpui::test]
fn enter_in_the_invite_picker_sends_the_picked_people(cx: &mut TestAppContext) {
    init(cx);
    let people = vec![
        InviteRow {
            id: uid("u2"),
            name: "Ana".into(),
            detail: "".into(),
        },
        InviteRow {
            id: uid("u3"),
            name: "Ben".into(),
            detail: "".into(),
        },
    ];
    let (d, vcx) = cx.add_window_view(move |_, cx| InviteDialog::new("Add".into(), people, cx));
    vcx.run_until_parked();
    let log = events::<_, InviteEvent>(&d, vcx);
    focus(&d, vcx);
    key(vcx, "enter", false);
    assert!(log.borrow().is_empty());
    d.update(vcx, |d, cx| d.toggle(&uid("u3"), cx));
    key(vcx, "enter", false);
    assert_eq!(*log.borrow(), [InviteEvent::Invite(vec![uid("u3")])]);
}

#[gpui::test]
fn browser_rows_carry_their_text_and_action_labels(cx: &mut TestAppContext) {
    init(cx);
    let (b, vcx) = cx.add_window_view(|_, cx| ChannelBrowser::new(cx));
    let row = |id: &str, name: &str, member: bool| BrowseRow {
        id: cid(id),
        name: name.to_string().into(),
        private: false,
        purpose: "".into(),
        members: Some(12),
        is_member: member,
    };
    b.update(vcx, |b, cx| {
        b.set_page(
            vec![row("C1", "design", false), row("C2", "general", true)],
            true,
            false,
            cx,
        )
    });
    let texts = b.read_with(vcx, |b, _| b.row_texts().to_vec());
    assert_eq!(&*texts[0].action_label, "Join #design");
    assert_eq!(&*texts[0].meta, "12 members");
    assert_eq!(&*texts[1].action_label, "Open #general");
    assert_eq!(&*texts[1].meta, "12 members \u{b7} Joined");
}

#[gpui::test]
fn an_empty_preview_draws_no_box(cx: &mut TestAppContext) {
    init(cx);
    let (d, vcx) = cx.add_window_view(|_, cx| {
        ConfirmDialog::new("Delete message?".into(), String::new(), "Delete".into(), cx)
            .preview(String::new())
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("confirm-preview").is_none());
    assert_eq!(
        d.read_with(vcx, |d, _| d.preview_str().map(str::to_owned)),
        None
    );
}
