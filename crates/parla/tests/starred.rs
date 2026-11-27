//! Starred conversations through the app: the sidebar row menu stars, the
//! Starred section follows, nothing goes to the provider, and the star is
//! stored for the next start.

use std::sync::Arc;

use components::{SidebarRow, SidebarSource};
use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, ts, uid};
use gpui::{TestAppContext, VisualTestContext};
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use settings::ConfigPaths;
use store::{MemoryStore, Store};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        parla::init_without_watcher(cx, ConfigPaths::at(dir.path()));
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn headers(rows: &[SidebarRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| match r {
            SidebarRow::Header { label: h, .. } => Some(h.to_string()),
            _ => None,
        })
        .collect()
}

async fn start(
    cx: &mut TestAppContext,
) -> (
    gpui::Entity<RootView>,
    &mut VisualTestContext,
    Arc<MemoryStore>,
) {
    let p = Arc::new(FakeProvider::new("w1", uid("u1")));
    p.add_conversation(conv("C1", "general"));
    p.add_conversation(conv("C2", "random"));
    p.add_member(member("u1", "Me"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let store = Arc::new(MemoryStore::default());
    let st: Arc<dyn Store> = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    cx.run_until_parked();
    (view, cx, store)
}

#[gpui::test]
async fn row_menu_stars_and_unstars_and_the_section_follows(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx, store) = start(cx).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    let sidebar = view.read_with(cx, |v, _| v.sidebar().unwrap().clone());
    assert!(!headers(&session.read_with(cx, SidebarSource::rows)).contains(&"Starred".into()));

    for starred in [true, false] {
        sidebar.update(cx, |_, cx| {
            cx.emit(components::SidebarEvent::ContextMenu {
                conversation: cid("C2"),
                position: gpui::point(gpui::px(100.0), gpui::px(100.0)),
            })
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert!(v.notify_menu().is_some()));
        // Entry 1 of the row menu is the star toggle.
        let b = cx.debug_bounds("notify-extra-1").expect("star entry");
        cx.simulate_click(b.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        session.read_with(cx, |s, _| assert_eq!(s.is_starred(&cid("C2")), starred));
    }
    // Starred first, then back to normal.
    let rows = session.read_with(cx, SidebarSource::rows);
    assert!(!headers(&rows).contains(&"Starred".into()));
    session.update(cx, |s, cx| s.set_starred(cid("C2"), true, cx));
    cx.run_until_parked();
    assert_eq!(
        headers(&session.read_with(cx, SidebarSource::rows)),
        ["Starred", "Channels"]
    );
    let ws = session.read_with(cx, |s, _| s.state().workspace.clone());
    assert!(store.local_starred(&ws).await.unwrap().contains(&cid("C2")));
}

#[gpui::test]
async fn row_menu_marks_the_conversation_read(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = Arc::new(FakeProvider::new("w1", uid("u1")));
    let mut c = conv("C1", "general");
    c.unread = true;
    c.latest = Some(ts("5.000000"));
    p.add_conversation(c);
    p.add_member(member("u1", "Me"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    let sidebar = view.read_with(cx, |v, _| v.sidebar().unwrap().clone());
    let unread = |cx: &mut VisualTestContext| {
        session.read_with(cx, |s, _| s.state().conversations[&cid("C1")].unread)
    };
    assert!(unread(cx));
    sidebar.update(cx, |_, cx| {
        cx.emit(components::SidebarEvent::ContextMenu {
            conversation: cid("C1"),
            position: gpui::point(gpui::px(100.0), gpui::px(100.0)),
        })
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    // Entry 2 of the row menu is Mark as read.
    let b = cx.debug_bounds("notify-extra-2").expect("mark read entry");
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(!unread(cx));
}

#[gpui::test]
async fn saved_stars_show_the_section_at_startup(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = Arc::new(FakeProvider::new("w1", uid("u1")));
    p.add_conversation(conv("C1", "general"));
    p.add_conversation(conv("C2", "random"));
    p.add_member(member("u1", "Me"));
    let ws = domain::ChatProvider::workspace(&*p).clone();
    let store = Arc::new(MemoryStore::default());
    store
        .apply(
            &ws,
            domain::PersistOp::LocalStarredPut {
                conversation: cid("C2"),
                starred: true,
            },
        )
        .await
        .unwrap();
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let st: Arc<dyn Store> = store;
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    assert_eq!(
        headers(&session.read_with(cx, SidebarSource::rows)),
        ["Starred", "Channels"]
    );
}

#[gpui::test]
async fn header_star_toggles_the_open_conversation(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx, _store) = start(cx).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    let open = session.read_with(cx, |s, _| s.state().conversations.keys().min().cloned());
    let open = open.expect("a conversation");
    session.update(cx, |s, cx| {
        components::SidebarSource::open(s, open.clone(), cx)
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    for want in [true, false] {
        let b = cx.debug_bounds("header-star").expect("header star");
        cx.simulate_click(b.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        session.read_with(cx, |s, _| assert_eq!(s.is_starred(&open), want));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

#[gpui::test]
async fn view_details_opens_the_right_clicked_conversation(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx, _store) = start(cx).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    let sidebar = view.read_with(cx, |v, _| v.sidebar().unwrap().clone());
    session.update(cx, |s, cx| {
        components::SidebarSource::open(s, cid("C1"), cx)
    });
    cx.run_until_parked();
    sidebar.update(cx, |_, cx| {
        cx.emit(components::SidebarEvent::ContextMenu {
            conversation: cid("C2"),
            position: gpui::point(gpui::px(100.0), gpui::px(100.0)),
        })
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let b = cx.debug_bounds("notify-extra-0").expect("details entry");
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    view.read_with(cx, |v, _cx| {
        assert!(v.details_panel().is_some(), "details panel open");
        assert_eq!(v.shown_conversation(), Some(&cid("C2")));
    });
}
