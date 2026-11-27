//! The Drafts view: every saved draft, newest first, opens or deletes one.

mod common;

use std::sync::Arc;

use components::{DraftItem, DraftKind, DraftsEvent, DraftsSource, DraftsView};
use domain::test_support::{cid, ts};
use domain::{DraftKey, FixedNow};
use gpui::{App, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use jiff::Timestamp;

/// 2026-11-15 10:00:00 UTC.
const NOW: i64 = 1_794_736_800;

struct Src {
    items: Vec<DraftItem>,
    deleted: Vec<DraftKey>,
}

impl DraftsSource for Src {
    fn draft_items(&self, _cx: &App) -> Vec<DraftItem> {
        self.items.clone()
    }
    fn delete_draft(&mut self, key: DraftKey, _cx: &mut App) {
        self.items.retain(|i| i.key != key);
        self.deleted.push(key);
    }
}

fn key(c: &str, thread: Option<&str>) -> DraftKey {
    DraftKey {
        conversation: cid(c),
        thread: thread.map(ts),
    }
}

fn item(c: &str, thread: Option<&str>, label: &str, text: &str, edited: i64) -> DraftItem {
    DraftItem {
        key: key(c, thread),
        conversation: label.into(),
        kind: if thread.is_some() {
            DraftKind::Thread
        } else {
            DraftKind::Channel
        },
        text: text.into(),
        edited,
    }
}

fn setup(
    cx: &mut TestAppContext,
    items: Vec<DraftItem>,
) -> (Entity<Src>, Entity<DraftsView<Src>>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(cx, &[components::catalog()], i18n::LocalePref::System);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        components::set_now(cx, Arc::new(FixedNow(Timestamp::from_second(NOW).unwrap())));
    });
    let src = cx.new(|_| Src {
        items,
        deleted: vec![],
    });
    let s = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| DraftsView::new(s, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    (src, view, vcx)
}

fn sample() -> Vec<DraftItem> {
    vec![
        item(
            "C1",
            None,
            "#launch-week",
            "Announcement\nfor the launch",
            NOW - 3 * 86_400,
        ),
        item("C2", None, "#design", "Keep five presets", NOW - 300),
        item(
            "C3",
            Some("1.000000"),
            "#eng",
            "I think the flake is the store",
            NOW - 7200,
        ),
    ]
}

#[gpui::test]
fn lists_drafts_newest_first_with_where_and_when(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, sample());
    let rows = vcx.update(|_, cx| view.read(cx).rows(cx));
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r.place.as_str(), r.when.as_str(), r.preview.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("#design", "Edited 5 minutes ago", "Keep five presets"),
            (
                "Thread in #eng",
                "Edited 2 hours ago",
                "I think the flake is the store"
            ),
            (
                "#launch-week",
                "Edited 3 days ago",
                "Announcement for the launch"
            ),
        ]
    );
}

#[gpui::test]
fn clicking_a_row_asks_to_open_its_draft(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, sample());
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = opened.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &DraftsEvent, _| {
            if let DraftsEvent::Open(k) = ev {
                log.borrow_mut().push(k.clone());
            }
        })
        .detach();
    });
    let b = vcx.debug_bounds("drafts-row-1").expect("second row drawn");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(*opened.borrow(), [key("C3", Some("1.000000"))]);
}

#[gpui::test]
fn the_trash_button_deletes_only_that_draft(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, sample());
    let b = vcx.debug_bounds("drafts-delete-0").expect("delete drawn");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        src.read_with(vcx, |s, _| s.deleted.clone()),
        [key("C2", None)]
    );
    assert_eq!(vcx.update(|_, cx| view.read(cx).rows(cx)).len(), 2);
}

#[gpui::test]
fn no_drafts_shows_the_empty_state(cx: &mut TestAppContext) {
    let (_src, _view, vcx) = setup(cx, vec![]);
    assert!(vcx.debug_bounds("drafts-empty").is_some());
    assert!(vcx.debug_bounds("drafts-row-0").is_none());
}

#[gpui::test]
fn rows_take_the_keyboard_and_escape_closes(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, sample());
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let l = log.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &DraftsEvent, _| {
            l.borrow_mut().push(ev.clone())
        })
        .detach();
    });
    let row = view.read_with(vcx, |v, _| v.row_focus(0));
    vcx.update(|w, cx| w.focus(&row, cx));
    vcx.simulate_keystrokes("enter");
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    vcx.simulate_keystrokes("escape");
    assert_eq!(
        *log.borrow(),
        [DraftsEvent::Open(key("C2", None)), DraftsEvent::Close]
    );
}
