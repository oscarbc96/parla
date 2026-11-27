//! The Files tab through the app: the header switches the conversation to its
//! file list (read from the store, newest first) and back to the messages.

use std::sync::Arc;

use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, msg, uid};
use domain::{FileId, FileKind, FileRef, PersistOp, ProviderExt};
use gpui::{Focusable as _, TestAppContext, VisualTestContext};
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

fn shared(id: &str, name: &str) -> FileRef {
    FileRef {
        id: FileId(Arc::from(id)),
        name: Arc::from(name),
        title: None,
        mime: Arc::from("application/octet-stream"),
        size: 1_024,
        kind: FileKind::Other,
        thumbs: Arc::from([]),
        download: None,
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui::test]
async fn the_files_tab_lists_the_conversations_files_and_switches_back(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = Arc::new(FakeProvider::new("w1", uid("u1")));
    p.add_conversation(conv("C1", "general"));
    p.add_member(member("u1", "Me"));
    let ws = domain::ChatProvider::workspace(&*p).clone();
    let store = Arc::new(MemoryStore::default());
    let mut older = msg("C1", "1700000000.000100", "plan");
    older.files = Arc::from([shared("F1", "plan.txt")]);
    let mut newer = msg("C1", "1700000100.000100", "export");
    newer.files = Arc::from([shared("F2", "export.bin")]);
    store
        .apply(
            &ws,
            PersistOp::Messages {
                conversation: cid("C1"),
                messages: vec![older, newer],
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
    session.update(cx, |s, cx| {
        components::SidebarSource::open(s, cid("C1"), cx)
    });
    redraw(cx);
    assert!(cx.debug_bounds("files-tab").is_none());
    let tab = cx.debug_bounds("header-tab-files").expect("files tab");
    cx.simulate_click(tab.center(), gpui::Modifiers::none());
    redraw(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.conversation_tab()),
        components::ConversationTab::Files
    );
    assert!(cx.debug_bounds("files-tab").is_some());
    assert!(cx.debug_bounds("files-row-0").is_some(), "newest file row");
    assert!(cx.debug_bounds("files-row-1").is_some(), "older file row");
    let back = cx
        .debug_bounds("header-tab-messages")
        .expect("messages tab");
    cx.simulate_click(back.center(), gpui::Modifiers::none());
    redraw(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.conversation_tab()),
        components::ConversationTab::Messages
    );
    assert!(cx.debug_bounds("files-tab").is_none());
    let composer = view.read_with(cx, |v, cx| v.composer().unwrap().read(cx).focus_handle(cx));
    assert!(
        cx.update(|w, _| composer.is_focused(w)),
        "the composer has the focus again"
    );
}
