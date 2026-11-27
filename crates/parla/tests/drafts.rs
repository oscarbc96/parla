//! Drafts and the sidebar entry points through the app: the Drafts view lists
//! what the store kept, opens a draft into its composer and deletes one; the
//! New message and "+" buttons open the right picker; collapsed sections are
//! kept per workspace.

use std::sync::Arc;

use components::{
    DraftsEvent, DraftsSource, PickerDelegate as _, SidebarEvent, SidebarRow, SidebarSection,
    SidebarSource, SwitchScope, SwitchTarget,
};
use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, msg, ts, uid};
use domain::{ComposedText, ConversationKind, DraftKey};
use gpui::{Entity, TestAppContext, VisualTestContext};
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use settings::{ConfigPaths, WorkspaceKey};
use store::{MemoryStore, Store};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        parla::init_without_watcher(cx, ConfigPaths::at(dir.path()));
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn ws() -> WorkspaceKey {
    WorkspaceKey::new("fake", "w1")
}

fn key(c: &str, thread: Option<&str>) -> DraftKey {
    DraftKey {
        conversation: cid(c),
        thread: thread.map(ts),
    }
}

fn provider() -> Arc<FakeProvider> {
    provider_with(true)
}

fn provider_with(open_direct: bool) -> Arc<FakeProvider> {
    let p = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(domain::Capabilities {
            open_direct,
            threads: true,
            ..Default::default()
        }),
    );
    p.add_conversation(conv("C1", "general"));
    p.add_conversation(conv("C2", "random"));
    let mut dm = conv("D1", "");
    dm.kind = ConversationKind::Direct;
    dm.peer = Some(uid("u2"));
    p.add_conversation(dm);
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Ana"));
    p.add_member(member("u3", "Bo"));
    p.add_messages(&cid("C2"), vec![msg("C2", "1.000000", "root")]);
    p
}

async fn seeded_store() -> Arc<MemoryStore> {
    let store = Arc::new(MemoryStore::default());
    store
        .draft_put(
            &ws(),
            key("C2", None),
            Some(ComposedText::plain("hello")),
            100,
        )
        .await
        .unwrap();
    store
        .draft_put(
            &ws(),
            key("C2", Some("1.000000")),
            Some(ComposedText::plain("in the thread")),
            200,
        )
        .await
        .unwrap();
    store
}

async fn start(
    cx: &mut TestAppContext,
    store: Arc<MemoryStore>,
) -> (Entity<RootView>, &mut VisualTestContext) {
    start_with(cx, store, provider()).await
}

async fn start_with(
    cx: &mut TestAppContext,
    store: Arc<MemoryStore>,
    p: Arc<FakeProvider>,
) -> (Entity<RootView>, &mut VisualTestContext) {
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let st: Arc<dyn Store> = store;
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    cx.run_until_parked();
    (view, cx)
}

fn open_drafts(view: &Entity<RootView>, cx: &mut VisualTestContext) {
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| SidebarSource::open_drafts(s, cx));
    cx.run_until_parked();
}

#[gpui::test]
async fn the_drafts_view_lists_what_the_store_kept(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = start(cx, seeded_store().await).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    assert_eq!(session.read_with(cx, |s, _| s.drafts_entry()), Some(2));
    open_drafts(&view, cx);
    let drafts = view.read_with(cx, |v, _| v.drafts_view().cloned().expect("drafts open"));
    let rows = cx.update(|_, cx| drafts.read(cx).rows(cx));
    let places: Vec<_> = rows.iter().map(|r| r.place.clone()).collect();
    assert_eq!(places, ["Thread in #random", "#random"]);
}

#[gpui::test]
async fn opening_a_draft_puts_it_in_its_composer(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = start(cx, seeded_store().await).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    open_drafts(&view, cx);
    let drafts = view.read_with(cx, |v, _| v.drafts_view().cloned().unwrap());
    drafts.update(cx, |_, cx| cx.emit(DraftsEvent::Open(key("C2", None))));
    cx.run_until_parked();
    assert_eq!(
        session.read_with(cx, |s, _| s.selected().cloned()),
        Some(cid("C2"))
    );
    let composer = view.read_with(cx, |v, _| v.composer().unwrap().clone());
    assert_eq!(
        composer.read_with(cx, |c, cx| c.text(cx).to_string()),
        "hello"
    );
    assert!(view.read_with(cx, |v, _| v.drafts_view().is_none()));

    // A thread draft opens the thread with the draft in the thread composer.
    open_drafts(&view, cx);
    let drafts = view.read_with(cx, |v, _| v.drafts_view().cloned().unwrap());
    drafts.update(cx, |_, cx| {
        cx.emit(DraftsEvent::Open(key("C2", Some("1.000000"))))
    });
    cx.run_until_parked();
    let panel = view.read_with(cx, |v, _| v.thread_panel().cloned().expect("thread open"));
    let text = cx.update(|_, cx| panel.read(cx).composer().read(cx).text(cx).to_string());
    assert_eq!(text, "in the thread");
}

#[gpui::test]
async fn deleting_a_draft_removes_it_from_the_store(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let store = seeded_store().await;
    let (view, cx) = start(cx, store.clone()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    for k in [key("C2", None), key("C2", Some("1.000000"))] {
        session.update(cx, |s, cx| DraftsSource::delete_draft(s, k, cx));
        cx.run_until_parked();
    }
    assert!(store.drafts(&ws()).await.unwrap().is_empty());
    assert_eq!(session.read_with(cx, |s, _| s.drafts_entry()), None);
}

#[gpui::test]
async fn deleting_the_open_conversations_draft_empties_its_composer(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let store = seeded_store().await;
    let (view, cx) = start(cx, store.clone()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C2"), cx));
    cx.run_until_parked();
    let composer = view.read_with(cx, |v, _| v.composer().unwrap().clone());
    assert_eq!(
        composer.read_with(cx, |c, cx| c.text(cx).to_string()),
        "hello"
    );
    session.update(cx, |s, cx| {
        DraftsSource::delete_draft(s, key("C2", None), cx)
    });
    cx.run_until_parked();
    assert_eq!(composer.read_with(cx, |c, cx| c.text(cx).to_string()), "");
    // Leaving does not save it back.
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    assert_eq!(store.drafts(&ws()).await.unwrap().len(), 1);
}

fn collapsed(rows: &[SidebarRow]) -> Vec<SidebarSection> {
    rows.iter()
        .filter_map(|r| match r {
            SidebarRow::Header {
                section,
                collapsed: true,
                ..
            } => Some(*section),
            _ => None,
        })
        .collect()
}

#[gpui::test]
async fn a_collapsed_section_stays_collapsed_after_a_restart(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let store = Arc::new(MemoryStore::default());
    {
        let (view, cx) = start(cx, store.clone()).await;
        let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
        session.update(cx, |s, cx| s.toggle_section(SidebarSection::Channels, cx));
        cx.run_until_parked();
        assert_eq!(
            collapsed(&session.read_with(cx, SidebarSource::rows)),
            [SidebarSection::Channels]
        );
    }
    assert!(
        store
            .sidebar_collapsed(&ws())
            .await
            .unwrap()
            .contains("channels")
    );
    let (view, cx) = start(cx, store.clone()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    assert_eq!(
        collapsed(&session.read_with(cx, SidebarSource::rows)),
        [SidebarSection::Channels]
    );
    session.update(cx, |s, cx| s.toggle_section(SidebarSection::Channels, cx));
    cx.run_until_parked();
    assert!(store.sidebar_collapsed(&ws()).await.unwrap().is_empty());
}

fn emit(view: &Entity<RootView>, cx: &mut VisualTestContext, ev: SidebarEvent) {
    let sidebar = view.read_with(cx, |v, _| v.sidebar().unwrap().clone());
    sidebar.update(cx, |_, cx| cx.emit(ev));
    cx.run_until_parked();
}

/// The open switcher's scope and the kinds of what it lists.
fn switcher(view: &Entity<RootView>, cx: &mut VisualTestContext) -> (SwitchScope, Vec<String>) {
    let picker = view.read_with(cx, |v, _| {
        v.quick_switcher().cloned().expect("switcher open")
    });
    cx.update(|_, cx| {
        let d = picker.read(cx).delegate();
        let labels = d
            .matches()
            .iter()
            .map(|i| match &i.target {
                SwitchTarget::Conversation(c) => format!("c:{}", i.label).replace(&*c.0, ""),
                SwitchTarget::Person(_) => format!("p:{}", i.label),
            })
            .collect();
        (d.scope(), labels)
    })
}

#[gpui::test]
async fn the_sidebar_buttons_open_the_right_picker(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = start(cx, Arc::new(MemoryStore::default())).await;

    emit(&view, cx, SidebarEvent::BrowseChannels);
    let (scope, mut items) = switcher(&view, cx);
    items.sort();
    assert_eq!(scope, SwitchScope::Channels);
    assert_eq!(items, ["c:general", "c:random"]);
    emit(&view, cx, SidebarEvent::BrowseChannels); // a second press closes it
    assert!(view.read_with(cx, |v, _| v.quick_switcher().is_none()));

    emit(&view, cx, SidebarEvent::BrowsePeople);
    let (scope, mut items) = switcher(&view, cx);
    items.sort();
    assert_eq!(scope, SwitchScope::People);
    assert_eq!(items, ["c:Ana", "p:Bo"]);
    emit(&view, cx, SidebarEvent::BrowsePeople);

    emit(&view, cx, SidebarEvent::NewMessage);
    let (scope, items) = switcher(&view, cx);
    assert_eq!(scope, SwitchScope::Compose);
    assert_eq!(items.len(), 4);
}

#[gpui::test]
async fn picking_from_new_message_focuses_that_composer(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = start(cx, Arc::new(MemoryStore::default())).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    emit(&view, cx, SidebarEvent::NewMessage);
    let picker = view.read_with(cx, |v, _| v.quick_switcher().cloned().unwrap());
    picker.update(cx, |p, cx| p.set_query("random", cx));
    let first = cx.update(|_, cx| picker.read(cx).delegate().matches()[0].target.clone());
    assert_eq!(first, SwitchTarget::Conversation(cid("C2")));
    picker.update(cx, |p, cx| p.confirm(cx));
    cx.run_until_parked();
    assert_eq!(
        session.read_with(cx, |s, _| s.selected().cloned()),
        Some(cid("C2"))
    );
    let composer = view.read_with(cx, |v, _| v.composer().unwrap().clone());
    let focused = cx.update(|window, cx| {
        gpui::Focusable::focus_handle(composer.read(cx), cx).is_focused(window)
    });
    assert!(focused, "the composer has the keyboard");
}

#[gpui::test]
async fn the_workspace_name_opens_the_workspace_menu(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = start(cx, Arc::new(MemoryStore::default())).await;
    emit(
        &view,
        cx,
        SidebarEvent::WorkspaceMenu {
            position: gpui::point(gpui::px(80.0), gpui::px(60.0)),
        },
    );
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.notify_menu().is_some()));
    // Preferences and Sign out are on it.
    let labels = view.read_with(cx, |v, cx| {
        v.notify_menu()
            .unwrap()
            .read(cx)
            .extra_labels()
            .into_iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
    });
    for want in ["Settings…", "Sign out"] {
        assert!(
            labels.iter().any(|l| l == want),
            "{want} missing: {labels:?}"
        );
    }
}

/// New message, then a person with no direct message yet: Bo (u3).
fn pick_bo(view: &Entity<RootView>, cx: &mut VisualTestContext) {
    emit(view, cx, SidebarEvent::NewMessage);
    let picker = view.read_with(cx, |v, _| v.quick_switcher().cloned().unwrap());
    picker.update(cx, |p, cx| p.set_query("Bo", cx));
    let first = cx.update(|_, cx| picker.read(cx).delegate().matches()[0].target.clone());
    assert_eq!(first, SwitchTarget::Person(uid("u3")));
    picker.update(cx, |p, cx| p.confirm(cx));
}

fn composer_focused(view: &Entity<RootView>, cx: &mut VisualTestContext) -> bool {
    let composer = view.read_with(cx, |v, _| v.composer().unwrap().clone());
    cx.update(|window, cx| gpui::Focusable::focus_handle(composer.read(cx), cx).is_focused(window))
}

#[gpui::test]
async fn new_message_to_someone_new_waits_for_the_dm_then_focuses_it(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.hold(domain::test_support::Op::OpenDirect);
    let (view, cx) = start_with(cx, Arc::new(MemoryStore::default()), p.clone()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    pick_bo(&view, cx);
    cx.run_until_parked();
    // Still waiting: nothing new is open yet.
    assert_ne!(
        session.read_with(cx, |s, _| s.selected().cloned()),
        Some(cid("dm-u3"))
    );
    p.release();
    cx.run_until_parked();
    assert_eq!(
        session.read_with(cx, |s, _| s.selected().cloned()),
        Some(cid("dm-u3"))
    );
    assert!(
        composer_focused(&view, cx),
        "the new DM's composer has the keyboard"
    );
    assert!(view.read_with(cx, |v, _| v.notice().is_none()));
}

#[gpui::test]
async fn new_message_shows_why_a_dm_could_not_open(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.fail_next(
        domain::test_support::Op::OpenDirect,
        domain::ProviderError::Network("down".into()),
    );
    let (view, cx) = start_with(cx, Arc::new(MemoryStore::default()), p).await;
    pick_bo(&view, cx);
    cx.run_until_parked();
    let notice = view.read_with(cx, |v, _| v.notice().map(str::to_string));
    assert!(
        notice.is_some_and(|n| n.contains("Bo")),
        "a notice names the person"
    );
}

#[gpui::test]
async fn new_message_says_when_the_provider_cannot_start_a_dm(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = start_with(cx, Arc::new(MemoryStore::default()), provider_with(false)).await;
    pick_bo(&view, cx);
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.notice().is_some()));
}

#[gpui::test]
async fn folding_unreads_is_not_possible(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let store = Arc::new(MemoryStore::default());
    let (view, cx) = start(cx, store.clone()).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.toggle_section(SidebarSection::Unreads, cx));
    cx.run_until_parked();
    assert!(store.sidebar_collapsed(&ws()).await.unwrap().is_empty());
}

#[gpui::test]
async fn the_drafts_view_shows_what_the_open_composer_holds_now(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = start(cx, seeded_store().await).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let composer = view.read_with(cx, |v, _| v.composer().unwrap().clone());
    composer.update(cx, |c, cx| c.set_text("first".into(), cx));
    open_drafts(&view, cx);
    let drafts = view.read_with(cx, |v, _| v.drafts_view().cloned().unwrap());
    let general = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            drafts
                .read(cx)
                .rows(cx)
                .into_iter()
                .find(|r| r.key == key("C1", None))
                .map(|r| r.preview)
        })
    };
    assert_eq!(general(cx).as_deref(), Some("first"));
    // Typing while the view is open shows at once.
    composer.update(cx, |c, cx| c.set_text("first and more".into(), cx));
    cx.run_until_parked();
    assert_eq!(general(cx).as_deref(), Some("first and more"));
    // Deleting it empties the composer that showed it.
    session.update(cx, |s, cx| {
        DraftsSource::delete_draft(s, key("C1", None), cx)
    });
    cx.run_until_parked();
    assert_eq!(composer.read_with(cx, |c, cx| c.text(cx).to_string()), "");
    assert_eq!(general(cx), None);
}

#[gpui::test]
async fn a_draft_deleted_before_the_store_loads_stays_deleted(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let store = seeded_store().await;
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider()));
    let st: Arc<dyn Store> = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, st, window, cx));
    // Before anything settles: the store has not answered yet.
    let session = view.read_with(cx, |v, _| v.session().cloned());
    if let Some(session) = session {
        session.update(cx, |s, cx| s.forget_draft(key("C2", None), cx));
    }
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.forget_draft(key("C2", None), cx));
    cx.run_until_parked();
    assert!(session.read_with(cx, |s, _| s.draft(&key("C2", None)).is_none()));
    let keys: Vec<_> = store
        .drafts(&ws())
        .await
        .unwrap()
        .into_iter()
        .map(|(k, _, _)| k)
        .collect();
    assert_eq!(keys, [key("C2", Some("1.000000"))]);
}

#[gpui::test]
async fn deleting_a_draft_empties_the_popout_showing_it(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = start(cx, seeded_store().await).await;
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    cx.update(|_, cx| parla::popout::open_conversation(ws(), cid("C2"), cx));
    cx.run_until_parked();
    let popout = cx.update(|_, cx| {
        parla::popout::composers_for(&ws(), &key("C2", None), cx)
            .into_iter()
            .next()
            .expect("popout composer")
    });
    assert_eq!(
        popout.read_with(cx, |c, cx| c.text(cx).to_string()),
        "hello"
    );
    session.update(cx, |s, cx| {
        DraftsSource::delete_draft(s, key("C2", None), cx)
    });
    cx.run_until_parked();
    assert_eq!(popout.read_with(cx, |c, cx| c.text(cx).to_string()), "");
}
