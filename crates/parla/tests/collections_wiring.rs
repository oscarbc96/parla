//! Pins, bookmarks, Later, Threads and the message actions, wired to the
//! window and the provider (FakeProvider capabilities on and off).
use std::sync::Arc;

use components::{MessageAction, SidebarSource as _};
use domain::test_support::{Call, FakeFactory, FakeProvider, cid, conv, member, msg, uid};
use domain::{Capabilities, InboxThread, MessageId, Pin, SavedItem};
use gpui::TestAppContext;
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use settings::ConfigPaths;
use store::{MemoryStore, Store};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn mid(ts: &str) -> MessageId {
    msg("C1", ts, "").id
}

type View<'a> = (
    gpui::Entity<RootView>,
    Arc<FakeProvider>,
    &'a mut gpui::VisualTestContext,
);

fn window(cx: &mut TestAppContext, caps: Capabilities) -> View<'_> {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")).with_capabilities(caps));
    provider.add_conversation(conv("C1", "general"));
    provider.add_conversation(conv("C2", "random"));
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Bob"));
    let mut m = msg("C1", "1700000100.000000", "pin me");
    m.author = Some(uid("u2"));
    provider.add_message(m);
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider.clone()));
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, vcx) = cx.add_window_view(|w, cx| RootView::new(&registry, st, w, cx));
    vcx.run_until_parked();
    (view, provider, vcx)
}

fn all_caps() -> Capabilities {
    Capabilities {
        pins: true,
        bookmarks: true,
        saved: true,
        thread_inbox: true,
        threads: true,
        ..Default::default()
    }
}

fn open_general(view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext) {
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
}

#[gpui::test]
async fn capability_off_hides_the_entries(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, _p, cx) = window(cx, Capabilities::default());
    open_general(&view, cx);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.read_with(cx, |s, _| {
        assert!(s.threads_entry().is_none() && s.later_entry().is_none());
    });
    // The timeline offers neither pin nor save.
    let tl = view.read_with(cx, |v, _| v.timeline().unwrap().clone());
    let acts = tl.read_with(cx, |t, cx| t.actions(&mid("1700000100.000000"), cx));
    assert!(!acts.contains(&MessageAction::Pin) && !acts.contains(&MessageAction::Save));
}

#[gpui::test]
async fn later_view_opens_and_a_row_jumps_to_its_message(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, p, cx) = window(cx, all_caps());
    p.set_saved_items(vec![SavedItem {
        message: mid("1700000100.000000"),
        saved_at: 5,
        due: None,
        done: false,
    }]);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.read_with(cx, |s, _| assert_eq!(s.later_entry(), Some(0)));
    session.update(cx, |s, cx| s.open_later(cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.collection_open()));
    session.read_with(cx, |s, _| assert_eq!(s.later_entry(), Some(1)));
    // Picking a conversation in the sidebar leaves the view, even the open one.
    session.update(cx, |s, cx| {
        components::SidebarSource::open(s, cid("C2"), cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(!v.collection_open());
        assert_eq!(v.shown_conversation(), Some(&cid("C2")));
    });
    // A row jumps to its message in its conversation.
    session.update(cx, |s, cx| s.open_later(cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.collection_open()));
    view.update_in(cx, |v, w, cx| {
        v.open_collection_message(mid("1700000100.000000"), w, cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(!v.collection_open());
        assert_eq!(v.shown_conversation(), Some(&cid("C1")));
    });
}

#[gpui::test]
async fn pins_popover_unpins_through_the_provider(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, p, cx) = window(cx, all_caps());
    p.set_pins(
        &cid("C1"),
        vec![Pin {
            message: mid("1700000100.000000"),
            by: None,
            at: 1,
        }],
    );
    open_general(&view, cx);
    view.update_in(cx, |v, w, cx| v.toggle_pins(cid("C1"), w, cx));
    cx.run_until_parked();
    let pop = view
        .read_with(cx, |v, _| v.pins_popover().cloned())
        .expect("open");
    let rows = pop.read_with(cx, |p, cx| p.rows(cx)).expect("loaded");
    assert_eq!(rows.len(), 1);
    pop.update(cx, |p, cx| p.unpin(rows[0].message.clone(), cx));
    cx.run_until_parked();
    assert!(
        p.calls()
            .iter()
            .any(|c| matches!(c, Call::SetPinned { pinned: false, .. })),
        "{:?}",
        p.calls()
    );
    // Asking again closes it.
    view.update_in(cx, |v, w, cx| v.toggle_pins(cid("C1"), w, cx));
    view.read_with(cx, |v, _| assert!(v.pins_popover().is_none()));
}

#[gpui::test]
async fn timeline_actions_reach_the_provider(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, p, cx) = window(cx, all_caps());
    open_general(&view, cx);
    let tl = view.read_with(cx, |v, _| v.timeline().unwrap().clone());
    let id = mid("1700000100.000000");
    for a in [MessageAction::Pin, MessageAction::Save] {
        cx.update(|w, cx| tl.update(cx, |t, cx| t.run_action(a, id.clone(), w, cx)));
    }
    cx.run_until_parked();
    let calls = p.calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::SetPinned { pinned: true, .. })),
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::SetSaved { saved: true, .. })),
        "{calls:?}"
    );
}

#[gpui::test]
async fn threads_view_row_opens_the_thread_panel(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, p, cx) = window(cx, all_caps());
    p.set_thread_inbox(vec![InboxThread {
        root: mid("1700000100.000000"),
        unread_replies: 2,
        mentions: 0,
        latest: domain::test_support::ts("1700000200.000000"),
    }]);
    open_general(&view, cx);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open_threads(cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.collection_open()));
    session.read_with(cx, |s, _| assert_eq!(s.threads_entry(), Some(2)));
    view.update_in(cx, |v, w, cx| {
        v.open_thread_panel(mid("1700000100.000000"), w, cx)
    });
    view.read_with(cx, |v, _| assert!(v.thread_panel().is_some()));
}

#[gpui::test]
async fn thread_panel_replaces_other_right_panels(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, _p, cx) = window(cx, all_caps());
    open_general(&view, cx);
    view.update_in(cx, |v, w, cx| v.open_scheduled(w, cx));
    view.read_with(cx, |v, _| assert!(v.scheduled_view().is_some()));
    view.update_in(cx, |v, w, cx| {
        v.open_thread_panel(mid("1700000100.000000"), w, cx)
    });
    view.read_with(cx, |v, _| {
        assert!(v.thread_panel().is_some());
        assert!(v.scheduled_view().is_none(), "one right panel at a time");
    });
    view.update_in(cx, |v, w, cx| v.open_search(w, cx));
    view.read_with(cx, |v, _| {
        assert!(v.search_panel().is_some());
        assert!(v.thread_panel().is_none());
    });
}
