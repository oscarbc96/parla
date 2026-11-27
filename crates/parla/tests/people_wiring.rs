//! The People directory wired to the window: the sidebar entry opens it,
//! Message opens or starts the direct message (waiting for the provider,
//! with a notice when it fails), View profile opens the profile panel.

use std::sync::Arc;

use components::{PeopleEvent, SidebarSource as _, TimelineSource as _};
use domain::test_support::{FakeFactory, FakeProvider, Op, cid, conv, member, uid};
use domain::{Capabilities, ProviderError};
use gpui::{Entity, TestAppContext, VisualTestContext};
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

fn provider() -> Arc<FakeProvider> {
    let p = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            open_direct: true,
            presence_watch: true,
            ..Default::default()
        }),
    );
    p.add_conversation(conv("C1", "general"));
    let mut dm = conv("D1", "Bob");
    dm.kind = domain::ConversationKind::Direct;
    dm.peer = Some(uid("u2"));
    p.add_conversation(dm);
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Bob"));
    let mut bo = member("u3", "Bo Lund");
    bo.title = Some(Arc::from("Designer"));
    p.add_member(bo);
    p
}

fn window(
    cx: &mut TestAppContext,
    p: Arc<FakeProvider>,
) -> (Entity<RootView>, &mut VisualTestContext) {
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, vcx) = cx.add_window_view(|w, cx| RootView::new(&registry, st, w, cx));
    vcx.run_until_parked();
    (view, vcx)
}

/// Opens the directory from the sidebar entry, as a click does.
fn open_people(view: &Entity<RootView>, cx: &mut VisualTestContext) {
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    assert!(session.read_with(cx, |s, _| s.people_entry()));
    session.update(cx, |s, cx| s.open_people(cx));
    cx.run_until_parked();
}

fn emit(view: &Entity<RootView>, cx: &mut VisualTestContext, ev: PeopleEvent) {
    let dir = view
        .read_with(cx, |v, _| v.people_directory().cloned())
        .expect("directory open");
    dir.update(cx, |_, cx| cx.emit(ev));
    cx.run_until_parked();
}

fn bo() -> PeopleEvent {
    PeopleEvent::Message {
        user: uid("u3"),
        name: "Bo Lund".into(),
    }
}

#[gpui::test]
async fn the_sidebar_entry_opens_the_directory_with_the_members(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = window(cx, provider());
    open_people(&view, cx);
    let dir = view
        .read_with(cx, |v, _| v.people_directory().cloned())
        .expect("directory open");
    assert_eq!(
        dir.update(cx, |d, cx| d.results(cx)),
        [uid("u3"), uid("u2")]
    );
    dir.update(cx, |d, cx| d.set_query("design", cx));
    assert_eq!(dir.update(cx, |d, cx| d.results(cx)), [uid("u3")]);
    emit(&view, cx, PeopleEvent::Close);
    view.read_with(cx, |v, _| assert!(v.people_directory().is_none()));
}

#[gpui::test]
async fn message_opens_an_existing_dm(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = window(cx, provider());
    open_people(&view, cx);
    emit(
        &view,
        cx,
        PeopleEvent::Message {
            user: uid("u2"),
            name: "Bob".into(),
        },
    );
    view.read_with(cx, |v, _| {
        assert_eq!(v.shown_conversation(), Some(&cid("D1")))
    });
}

#[gpui::test]
async fn message_to_someone_new_waits_for_the_dm(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.hold(Op::OpenDirect);
    let (view, cx) = window(cx, p.clone());
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    open_people(&view, cx);
    emit(&view, cx, bo());
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
    assert!(view.read_with(cx, |v, _| v.notice().is_none()));
}

#[gpui::test]
async fn message_says_why_a_dm_could_not_open(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = provider();
    p.fail_next(Op::OpenDirect, ProviderError::Network("down".into()));
    let (view, cx) = window(cx, p);
    open_people(&view, cx);
    emit(&view, cx, bo());
    let notice = view.read_with(cx, |v, _| v.notice().map(str::to_string));
    assert!(
        notice.is_some_and(|n| n.contains("Bo Lund")),
        "a notice names the person"
    );
}

#[gpui::test]
async fn view_profile_opens_the_profile_panel(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = window(cx, provider());
    open_people(&view, cx);
    emit(&view, cx, PeopleEvent::ViewProfile(uid("u3")));
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.read_with(cx, |s, _| {
        assert_eq!(
            components::ProfileSource::shown_profile(s),
            Some(&uid("u3"))
        )
    });
    view.read_with(cx, |v, _| assert!(v.profile_panel().is_some()));
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|w, cx| w.draw(cx).clear(cx));
    cx.run_until_parked();
}

#[gpui::test]
async fn leaving_for_threads_stops_watching_the_cards(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = window(cx, provider());
    open_people(&view, cx);
    draw(cx);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    assert!(!session.read_with(cx, |s, _| s.directory_watch().is_empty()));
    view.update_in(cx, |v, w, cx| v.open_collection(true, w, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.people_directory().is_none()));
    assert!(session.read_with(cx, |s, _| s.directory_watch().is_empty()));
}

#[gpui::test]
async fn closing_a_profile_opened_from_the_directory_returns_to_it(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = window(cx, provider());
    open_people(&view, cx);
    let dir = view
        .read_with(cx, |v, _| v.people_directory().cloned())
        .unwrap();
    dir.update(cx, |d, cx| d.set_query("lund", cx));
    emit(&view, cx, PeopleEvent::ViewProfile(uid("u3")));
    let panel = view
        .read_with(cx, |v, _| v.profile_panel().cloned())
        .expect("profile open");
    panel.update(cx, |p, cx| p.close(cx));
    cx.run_until_parked();
    let back = view
        .read_with(cx, |v, _| v.people_directory().cloned())
        .expect("directory back");
    assert_eq!(back.update(cx, |d, cx| d.results(cx)), [uid("u3")]);
    draw(cx);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    assert!(!session.read_with(cx, |s, _| s.directory_watch().is_empty()));
}

#[gpui::test]
async fn a_profile_opened_elsewhere_closes_to_nothing(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = window(cx, provider());
    open_people(&view, cx);
    emit(&view, cx, PeopleEvent::ViewProfile(uid("u3")));
    view.update_in(cx, |v, w, cx| v.open_collection(true, w, cx));
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open_profile(uid("u2"), cx));
    cx.run_until_parked();
    let panel = view
        .read_with(cx, |v, _| v.profile_panel().cloned())
        .expect("profile open");
    panel.update(cx, |p, cx| p.close(cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.people_directory().is_none()));
}
