//! The profile panel wired to the window: switcher, search, mentions and
//! authors open it; the status dialog and presence go to the provider.

use std::sync::Arc;

use components::{ProfileSource as _, TimelineSource as _};
use domain::test_support::{Call, FakeFactory, FakeProvider, cid, conv, member, uid};
use domain::{Capabilities, PresenceSetting, Profile};
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

fn profile(u: &str, name: &str) -> Profile {
    Profile {
        user: uid(u),
        display_name: Arc::from(name),
        real_name: Arc::from(name),
        title: Some(Arc::from("Engineer")),
        pronouns: None,
        tz: None,
        status: None,
        email: None,
        phone: None,
        fields: Vec::new(),
        image: None,
        is_bot: false,
        deleted: false,
    }
}

type View<'a> = (
    gpui::Entity<RootView>,
    Arc<FakeProvider>,
    &'a mut gpui::VisualTestContext,
);

fn window(cx: &mut TestAppContext) -> View<'_> {
    let provider = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            status: true,
            presence: true,
            dnd: true,
            ..Default::default()
        }),
    );
    provider.add_conversation(conv("C1", "general"));
    let mut dm = conv("D1", "Bob");
    dm.kind = domain::ConversationKind::Direct;
    dm.peer = Some(uid("u2"));
    provider.add_conversation(dm);
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Bob"));
    provider.add_member(member("u9", "Zed Zorro"));
    provider.set_profile(profile("u9", "Zed Zorro"));
    provider.set_profile(profile("u2", "Bob"));
    provider.set_profile(profile("u1", "Me"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider.clone()));
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, vcx) = cx.add_window_view(|w, cx| RootView::new(&registry, st, w, cx));
    vcx.run_until_parked();
    (view, provider, vcx)
}

fn shown(view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext) -> Option<String> {
    let panel = view.read_with(cx, |v, _| v.profile_panel().cloned())?;
    let model = panel.read_with(cx, |p, cx| p.model(cx));
    model.map(|m| m.name)
}

fn switch_to(view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext, query: &str) {
    view.update_in(cx, |v, w, cx| v.open_quick_switcher(w, cx));
    cx.run_until_parked();
    cx.simulate_input(query);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

#[gpui::test]
async fn person_without_dm_opens_profile(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, _p, cx) = window(cx);
    switch_to(&view, cx, "Zorro");
    assert_eq!(shown(&view, cx).as_deref(), Some("Zed Zorro"));
}

#[gpui::test]
async fn person_with_dm_opens_the_dm_not_the_profile(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, _p, cx) = window(cx);
    switch_to(&view, cx, "Bob");
    view.read_with(cx, |v, _| {
        assert_eq!(v.shown_conversation(), Some(&cid("D1")));
        assert!(v.profile_panel().is_none());
    });
}

#[gpui::test]
async fn mentions_and_authors_open_the_profile(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, _p, cx) = window(cx);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open_profile(uid("u9"), cx));
    cx.run_until_parked();
    assert_eq!(shown(&view, cx).as_deref(), Some("Zed Zorro"));
    // Another person replaces it; the profile loads from the provider.
    session.update(cx, |s, cx| s.open_profile(uid("u2"), cx));
    cx.run_until_parked();
    assert_eq!(shown(&view, cx).as_deref(), Some("Bob"));
}

#[gpui::test]
async fn search_person_hit_opens_the_profile(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, _p, cx) = window(cx);
    view.update_in(cx, |v, w, cx| v.open_search(w, cx));
    cx.run_until_parked();
    let panel = view
        .read_with(cx, |v, _| v.search_panel().cloned())
        .unwrap();
    panel.update(cx, |_, cx| {
        cx.emit(components::SearchPanelEvent::OpenPerson(uid("u9")))
    });
    cx.run_until_parked();
    assert_eq!(shown(&view, cx).as_deref(), Some("Zed Zorro"));
    view.read_with(cx, |v, _| assert!(v.search_panel().is_none()));
}

#[gpui::test]
async fn closing_the_profile_clears_the_session(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, _p, cx) = window(cx);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open_profile(uid("u9"), cx));
    cx.run_until_parked();
    let panel = view
        .read_with(cx, |v, _| v.profile_panel().cloned())
        .unwrap();
    panel.update(cx, |p, cx| p.close(cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.profile_panel().is_none()));
    session.read_with(cx, |s, _| assert!(s.shown_profile().is_none()));
}

#[gpui::test]
async fn set_status_action_saves_through_the_provider(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, provider, cx) = window(cx);
    view.update_in(cx, |v, w, cx| v.set_status_action(w, cx));
    cx.run_until_parked();
    let dialog = view
        .read_with(cx, |v, _| v.status_dialog().cloned())
        .expect("dialog");
    dialog.update(cx, |d, cx| {
        d.set_text("Heads down", cx);
        d.save(cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.status_dialog().is_none()));
    assert!(
        provider
            .calls()
            .iter()
            .any(|c| matches!(c, Call::SetStatus(Some(s)) if &*s.text == "Heads down")),
        "{:?}",
        provider.calls()
    );
}

#[gpui::test]
async fn presence_goes_to_the_provider(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, provider, cx) = window(cx);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.set_presence(PresenceSetting::Away, cx));
    cx.run_until_parked();
    assert!(
        provider
            .calls()
            .contains(&Call::SetPresence(PresenceSetting::Away))
    );
    session.read_with(cx, |s, _| {
        assert_eq!(s.own_presence(), Some(PresenceSetting::Away))
    });
}
