//! Reaching a channel the user is not in: the row menu copies a
//! conversation's link, and the quick switcher and search's Conversations
//! results open channels the user has not joined, shown with a Join action
//! that sending needs first.

use std::sync::Arc;
use std::time::Duration;

use components::{
    PickerDelegate as _, SearchPanelEvent, SearchScope, SearchSource as _, SwitchTarget,
};
use domain::test_support::{Call, FakeFactory, FakeProvider, Op, cid, conv, member, uid};
use domain::{AdminCall, Capabilities};
use gpui::{Entity, TestAppContext, VisualTestContext};
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use parla::session::WorkspaceSession;
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

fn fake() -> Arc<FakeProvider> {
    let p = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            channel_admin: true,
            search: true,
            open_direct: true,
            ..Default::default()
        }),
    );
    p.add_conversation(conv("C1", "general"));
    let mut other = conv("C2", "design-crit");
    other.is_member = false;
    p.add_conversation(other);
    p.add_member(member("u1", "Me"));
    p.add_member(member("u3", "Grace Hopper"));
    p
}

async fn start(
    cx: &mut TestAppContext,
    p: Arc<FakeProvider>,
) -> (Entity<RootView>, &mut VisualTestContext) {
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store, window, cx));
    cx.run_until_parked();
    (view, cx)
}

fn session(view: &Entity<RootView>, cx: &mut VisualTestContext) -> Entity<WorkspaceSession> {
    view.read_with(cx, |v, _| v.session().unwrap().clone())
}

fn frames(cx: &mut VisualTestContext) {
    for _ in 0..2 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

fn click(cx: &mut VisualTestContext, sel: &'static str) {
    frames(cx);
    let b = cx
        .debug_bounds(sel)
        .unwrap_or_else(|| panic!("{sel} is drawn"));
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    cx.run_until_parked();
}

fn joins(p: &FakeProvider) -> Vec<AdminCall> {
    p.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Admin(a @ AdminCall::Join(_)) => Some(a),
            _ => None,
        })
        .collect()
}

/// The switcher's matches: label and whether the user is in it.
fn matches(view: &Entity<RootView>, cx: &mut VisualTestContext) -> Vec<(String, bool)> {
    let picker = view.read_with(cx, |v, _| v.quick_switcher().cloned().expect("open"));
    picker.read_with(cx, |p, _| {
        p.delegate()
            .matches()
            .iter()
            .map(|i| (i.label.to_string(), i.joined))
            .collect()
    })
}

fn switch(view: &Entity<RootView>, cx: &mut VisualTestContext, query: &str) {
    view.update_in(cx, |v, w, cx| v.open_quick_switcher(w, cx));
    cx.run_until_parked();
    let picker = view.read_with(cx, |v, _| v.quick_switcher().cloned().unwrap());
    picker.update(cx, |p, cx| p.set_query(query, cx));
    cx.run_until_parked();
    // The browse waits for the typing to settle.
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
}

/// The composer of the shown conversation offers Join; a click joins it and
/// the input comes back.
fn join_through_the_composer(
    view: &Entity<RootView>,
    cx: &mut VisualTestContext,
    p: &FakeProvider,
    c: &str,
) {
    frames(cx);
    assert!(cx.debug_bounds("composer-join").is_some(), "join offered");
    assert!(cx.debug_bounds("composer-input").is_none());
    click(cx, "composer-join-button");
    assert_eq!(joins(p), [AdminCall::Join(cid(c))]);
    let s = session(view, cx);
    assert!(s.read_with(cx, |s, _| s.state().conversations[&cid(c)].is_member));
    frames(cx);
    assert!(cx.debug_bounds("composer-join").is_none());
    assert!(cx.debug_bounds("composer-input").is_some());
}

#[gpui::test]
async fn row_menu_copies_the_conversation_link(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, cx) = start(cx, fake()).await;
    let sidebar = view.read_with(cx, |v, _| v.sidebar().unwrap().clone());
    sidebar.update(cx, |_, cx| {
        cx.emit(components::SidebarEvent::ContextMenu {
            conversation: cid("C1"),
            position: gpui::point(gpui::px(100.0), gpui::px(100.0)),
        })
    });
    cx.run_until_parked();
    let labels: Vec<String> = view.read_with(cx, |v, cx| {
        v.notify_menu()
            .unwrap()
            .read(cx)
            .extra_labels()
            .into_iter()
            .map(|l| l.to_string())
            .collect()
    });
    let ix = labels
        .iter()
        .position(|l| l == "Copy link")
        .unwrap_or_else(|| panic!("no Copy link: {labels:?}"));
    click(cx, Box::leak(format!("notify-extra-{ix}").into_boxed_str()));
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("https://fake.example.test/C1"));
    // The same "Link copied" toast as a message's Copy link.
    let toasts = cx.update(|_, cx| {
        components::Toasts::target(cx)
            .map(|t| {
                t.read(cx)
                    .visible()
                    .iter()
                    .map(|s| s.title.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    assert_eq!(toasts, ["Link copied"]);
}

#[gpui::test]
async fn the_switcher_lists_channels_not_joined_and_opens_them_to_join(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    switch(&view, cx, "design");
    assert_eq!(matches(&view, cx), [("design-crit".to_string(), false)]);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("C2"))
    );
    // Opening is not joining, and nothing is marked read on the server.
    assert!(joins(&p).is_empty());
    assert!(!p.calls().iter().any(|c| matches!(c, Call::MarkRead { .. })));
    join_through_the_composer(&view, cx, &p, "C2");
}

#[gpui::test]
async fn the_switcher_finds_channels_the_list_does_not_hold(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    // Created after the start: only the provider's browse knows it.
    let mut gem = conv("C8", "hidden-gem");
    gem.is_member = false;
    p.add_conversation(gem);
    switch(&view, cx, "gem");
    assert_eq!(matches(&view, cx), [("hidden-gem".to_string(), false)]);
    assert!(
        p.calls()
            .iter()
            .any(|c| matches!(c, Call::BrowseChannels(q) if q == "gem"))
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("C8"))
    );
    let s = session(&view, cx);
    let name = s.read_with(cx, |s, _| s.state().conversations[&cid("C8")].name.clone());
    assert_eq!(&*name, "hidden-gem");
    join_through_the_composer(&view, cx, &p, "C8");
}

#[gpui::test]
async fn a_person_from_the_switcher_opens_or_starts_the_direct_message(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    switch(&view, cx, "Grace");
    let picker = view.read_with(cx, |v, _| v.quick_switcher().cloned().unwrap());
    let first = picker.read_with(cx, |p, _| p.delegate().matches()[0].target.clone());
    assert_eq!(first, SwitchTarget::Person(uid("u3")));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(p.calls().contains(&Call::OpenDirect(uid("u3"))));
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("dm-u3"))
    );
}

#[gpui::test]
async fn a_conversations_search_hit_not_joined_opens_with_join(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    let mut gem = conv("C8", "hidden-gem");
    gem.is_member = false;
    p.add_conversation(gem);
    let s = session(&view, cx);
    cx.dispatch_action(components::Search);
    cx.run_until_parked();
    s.update(cx, |s, cx| {
        s.search(SearchScope::Conversations, "gem".into(), cx)
    });
    cx.run_until_parked();
    let remote = s.read_with(cx, |s, _| s.search_results().remote.len());
    assert_eq!(remote, 1);
    let panel = view.read_with(cx, |v, _| v.search_panel().unwrap().clone());
    panel.update(cx, |_, cx| {
        cx.emit(SearchPanelEvent::OpenConversation(cid("C8")))
    });
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("C8"))
    );
    join_through_the_composer(&view, cx, &p, "C8");
}

fn browses(p: &FakeProvider) -> Vec<String> {
    p.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::BrowseChannels(q) => Some(q),
            _ => None,
        })
        .collect()
}

fn type_in(view: &Entity<RootView>, cx: &mut VisualTestContext, query: &str) {
    let picker = view.read_with(cx, |v, _| v.quick_switcher().cloned().unwrap());
    picker.update(cx, |p, cx| p.set_query(query, cx));
    cx.run_until_parked();
}

fn toasts(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        components::Toasts::target(cx)
            .map(|t| {
                t.read(cx)
                    .visible()
                    .iter()
                    .map(|s| s.title.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    })
}

/// Whether the shown composer holds the focus (its Join while the strip
/// shows, else its input).
fn composer_focused(view: &Entity<RootView>, cx: &mut VisualTestContext) -> bool {
    let composer = view.read_with(cx, |v, _| v.composer().cloned().expect("composer"));
    cx.update(|window, cx| {
        use gpui::Focusable as _;
        composer.read(cx).focus_handle(cx).is_focused(window)
    })
}

#[gpui::test]
async fn the_switcher_browses_once_the_typing_settles(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    view.update_in(cx, |v, w, cx| v.open_quick_switcher(w, cx));
    cx.run_until_parked();
    type_in(&view, cx, "des");
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    type_in(&view, cx, "desi");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert!(
        browses(&p).is_empty(),
        "no browse before the typing settles"
    );
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(browses(&p), ["desi"]);
    cx.executor().advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();
    assert_eq!(browses(&p), ["desi"], "one browse per settled query");
}

#[gpui::test]
async fn a_newer_switcher_query_drops_the_older_answer(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    let mut gem = conv("C8", "hidden-gem");
    gem.is_member = false;
    p.add_conversation(gem);
    let mut old = conv("C9", "gemstones-old");
    old.is_member = false;
    p.add_conversation(old);
    p.hold(Op::BrowseChannels);
    view.update_in(cx, |v, w, cx| v.open_quick_switcher(w, cx));
    cx.run_until_parked();
    type_in(&view, cx, "gemst");
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    type_in(&view, cx, "hidden");
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert_eq!(browses(&p), ["gemst", "hidden"]);
    p.release();
    cx.run_until_parked();
    assert_eq!(matches(&view, cx), [("hidden-gem".to_string(), false)]);
}

#[gpui::test]
async fn the_join_strip_takes_the_focus_and_hands_it_to_the_input(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    switch(&view, cx, "design");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    frames(cx);
    assert!(cx.debug_bounds("composer-join").is_some());
    assert!(composer_focused(&view, cx), "Join has the focus");
    // Enter on the focused Join joins.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(joins(&p), [AdminCall::Join(cid("C2"))]);
    frames(cx);
    assert!(cx.debug_bounds("composer-input").is_some());
    assert!(composer_focused(&view, cx), "the input has the focus");
}

#[gpui::test]
async fn a_second_click_while_joining_sends_nothing_and_a_failure_shows_a_toast(
    cx: &mut TestAppContext,
) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    switch(&view, cx, "design");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    p.hold(Op::Admin);
    click(cx, "composer-join-button");
    click(cx, "composer-join-button");
    assert_eq!(joins(&p), [AdminCall::Join(cid("C2"))], "one join");
    p.release();
    cx.run_until_parked();
    let s = session(&view, cx);
    assert!(s.read_with(cx, |s, _| s.state().conversations[&cid("C2")].is_member));

    // A join that fails says so in a toast, and Join can be tried again.
    let mut other = conv("C3", "ops");
    other.is_member = false;
    p.add_conversation(other);
    switch(&view, cx, "ops");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    p.fail_next(
        Op::Admin,
        domain::ProviderError::NotAllowed("restricted".into()),
    );
    click(cx, "composer-join-button");
    assert!(
        toasts(cx).iter().any(|t| t == "Couldn't join the channel"),
        "{:?}",
        toasts(cx)
    );
    // The refused call is not recorded by the fake; the retry is.
    click(cx, "composer-join-button");
    assert_eq!(
        joins(&p),
        [AdminCall::Join(cid("C2")), AdminCall::Join(cid("C3"))]
    );
    assert!(s.read_with(cx, |s, _| s.state().conversations[&cid("C3")].is_member));
}
