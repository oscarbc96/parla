//! The main window wired to a workspace session; no network.

use std::sync::Arc;

use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, msg, uid};
use gpui::TestAppContext;
use parla::providers::ProviderRegistry;
use parla::root_view::{Phase, RootView};
use settings::ConfigPaths;
use store::{MemoryStore, Store};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        // Tests never depend on the OS language.
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn store() -> Arc<dyn Store> {
    Arc::new(MemoryStore::default())
}

#[gpui::test]
async fn no_accounts_shows_sign_in_state(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let registry = ProviderRegistry::new();
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(*v.phase(), Phase::NoAccounts);
        assert!(v.session().is_none());
    });
}

#[gpui::test]
async fn account_starts_and_selection_shows_timeline(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u2", "Other"));
    provider.add_message(msg("C1", "1.000000", "hi"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(*v.phase(), Phase::Ready));
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.shown_conversation(), Some(&cid("C1")))
    });
}

#[gpui::test]
async fn chrome_header_badge_and_hidden_empty_panel(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.set_workspace_name("Example Workspace");
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u2", "Other"));
    provider.add_message(msg("C1", "1.000000", "hi"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    let draw = |cx: &mut gpui::VisualTestContext| {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    };
    draw(cx);
    assert!(cx.debug_bounds("workspace-badge").is_some(), "rail badge");
    assert!(cx.debug_bounds("conversation-header").is_none());
    assert!(cx.debug_bounds("side-panel").is_none(), "no empty panel");
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    draw(cx);
    assert!(cx.debug_bounds("conversation-header").is_some(), "header");
    assert!(cx.debug_bounds("side-panel").is_none(), "no empty panel");
}

#[gpui::test]
async fn clicking_a_mention_opens_the_direct_message_with_that_person(cx: &mut TestAppContext) {
    use components::TimelineSource as _;
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    let mut dm = conv("D1", "Other");
    dm.kind = domain::ConversationKind::Direct;
    dm.peer = Some(uid("u2"));
    provider.add_conversation(dm);
    provider.add_member(member("u2", "Other"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    // A person with no direct message here, then one with.
    session.update(cx, |s, cx| s.open_direct(uid("u9"), cx));
    cx.run_until_parked();
    session.read_with(cx, |s, _| assert_eq!(s.selected(), None));
    session.update(cx, |s, cx| s.open_direct(uid("u2"), cx));
    cx.run_until_parked();
    session.read_with(cx, |s, _| assert_eq!(s.selected(), Some(&cid("D1"))));
}

#[gpui::test]
async fn rail_icon_comes_from_the_persisted_account_row(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.set_workspace_name("Example Workspace");
    provider.add_conversation(conv("C1", "general"));
    let key = domain::ChatProvider::workspace(&*provider).clone();
    let icon = domain::ImageRef::Web("http://example.invalid/icon.png".into());
    let factory = FakeFactory::default().with_provider(provider);
    factory.set_accounts(vec![domain::AccountInfo {
        key,
        name: Arc::from("Example Workspace"),
        org: None,
        icon: Some(icon.clone()),
    }]);
    let mut registry = ProviderRegistry::new();
    registry.add(factory);
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(v.rail_icon(), Some(&icon)));
}

#[gpui::test]
async fn the_header_details_button_opens_the_panel_and_escape_closes_it(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u2", "Other"));
    provider.add_message(msg("C1", "1.000000", "hi"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    let draw = |cx: &mut gpui::VisualTestContext| {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    };
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    draw(cx);
    assert!(view.read_with(cx, |v, _| v.details_panel().is_none()));
    let button = cx.debug_bounds("header-details").expect("details button");
    cx.simulate_click(button.center(), gpui::Modifiers::none());
    draw(cx);
    assert!(view.read_with(cx, |v, _| v.details_panel().is_some()));
    assert!(cx.debug_bounds("channel-details").is_some());
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(view.read_with(cx, |v, _| v.details_panel().is_none()));
}

#[gpui::test]
async fn the_member_count_opens_the_members_tab_and_never_closes_the_panel(
    cx: &mut TestAppContext,
) {
    use components::DetailsTab;
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    let mut c = conv("C1", "general");
    c.member_count = Some(2);
    provider.add_conversation(c);
    provider.add_member(member("u2", "Other"));
    provider.add_message(msg("C1", "1.000000", "hi"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    let draw = |cx: &mut gpui::VisualTestContext| {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    };
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    draw(cx);
    let tab = |view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext| {
        view.read_with(cx, |v, cx| v.details_panel().map(|p| p.read(cx).tab()))
    };
    assert_eq!(tab(&view, cx), None);
    // Closed: opens on Members.
    let count = cx.debug_bounds("header-members").expect("member count");
    cx.simulate_click(count.center(), gpui::Modifiers::none());
    draw(cx);
    assert_eq!(tab(&view, cx), Some(DetailsTab::Members));
    // Open on Members: a second click keeps it open.
    let count = cx.debug_bounds("header-members").expect("member count");
    cx.simulate_click(count.center(), gpui::Modifiers::none());
    draw(cx);
    assert_eq!(tab(&view, cx), Some(DetailsTab::Members));
    // Open on About: switches to Members, still open.
    let about = cx.debug_bounds("details-tab-about").expect("about tab");
    cx.simulate_click(about.center(), gpui::Modifiers::none());
    draw(cx);
    assert_eq!(tab(&view, cx), Some(DetailsTab::About));
    let count = cx.debug_bounds("header-members").expect("member count");
    cx.simulate_click(count.center(), gpui::Modifiers::none());
    draw(cx);
    assert_eq!(tab(&view, cx), Some(DetailsTab::Members));
}
