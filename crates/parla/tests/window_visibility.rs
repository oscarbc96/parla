//! What the main window's lean timers follow: whether the window is being
//! presented (visible), not whether it has keyboard focus. An unfocused window
//! that is still on screen is being looked at; a covered or minimised one is
//! not.

mod support;

use std::sync::Arc;
use std::time::Duration;

use domain::test_support::{FakeFactory, FakeProvider, conv, member, msg, uid};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, WindowVisibility};
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use parla::workspaces::Workspaces;
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

/// Three fake accounts, each with one conversation and one message.
fn accounts() -> Vec<Arc<FakeProvider>> {
    ["w1", "w2", "w3"]
        .iter()
        .map(|id| {
            let p = Arc::new(FakeProvider::new(id, uid("u1")));
            p.add_conversation(conv("C1", "general"));
            p.add_member(member("u2", "Other"));
            p.add_message(msg("C1", "1.000000", "hi"));
            p
        })
        .collect()
}

fn registry(providers: &[Arc<FakeProvider>]) -> ProviderRegistry {
    let mut factory = FakeFactory::default();
    for p in providers {
        factory = factory.with_provider(p.clone());
    }
    let mut registry = ProviderRegistry::new();
    registry.add(factory);
    registry
}

fn workspaces(cx: &mut TestAppContext, registry: &ProviderRegistry) -> Entity<Workspaces> {
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    cx.new(|cx| Workspaces::new(registry, store, cx))
}

/// Opens the main window over `w`.
fn window<'a>(
    cx: &'a mut TestAppContext,
    registry: &ProviderRegistry,
    w: &Entity<Workspaces>,
) -> (Entity<RootView>, &'a mut VisualTestContext) {
    let ready = w.clone();
    cx.add_window_view(|window, cx| RootView::with_workspaces(registry, ready.clone(), window, cx))
}

/// The main window over three fake accounts. Its active workspace is the
/// first one; the other two arm lean timers (ten minutes by default).
fn open(cx: &mut TestAppContext) -> (Entity<Workspaces>, Entity<RootView>, &mut VisualTestContext) {
    let providers = accounts();
    let registry = registry(&providers);
    let w = workspaces(cx, &registry);
    let (view, vcx) = window(cx, &registry, &w);
    // Every account has started and the shown one keeps its timers.
    support::settle_until(vcx, |cx| {
        w.read_with(cx, |w, _| w.sessions().count() == 3 && w.lean_timers() == 2)
    });
    (w, view, vcx)
}

fn minutes(cx: &mut TestAppContext, m: u64) {
    cx.executor().advance_clock(Duration::from_secs(m * 60));
    support::settle(cx);
}

/// Runs the main loop until the platform reports `done` for the main window.
/// Focus and presentation reach a window from the platform through the
/// foreground executor, so a test that needs the new state waits for it rather
/// than assuming that one loop turn delivered it.
fn until_window(vcx: &mut VisualTestContext, mut done: impl FnMut(&gpui::Window) -> bool) {
    support::settle_visual_until(vcx, |cx| cx.update(|window, _| done(window)));
}

#[gpui::test]
async fn a_visible_window_behind_another_app_is_not_hidden(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (w, view, vcx) = open(cx);
    // Focus the main window, then let another window take it: another app in
    // front, the main window still fully visible. Each activation lands
    // asynchronously and must land before the next step, or the platform could
    // report the main window active again when it is checked.
    vcx.update(|window, _| window.activate_window());
    until_window(vcx, |window| window.is_window_active());
    {
        let mut app = vcx.cx.clone();
        let other = app.add_empty_window();
        other.update(|window, _| window.activate_window());
    }
    until_window(vcx, |window| !window.is_window_active());
    assert!(
        vcx.update(|window, _| window.is_visible()),
        "the main window is still on screen"
    );
    assert_eq!(
        w.read_with(vcx, |w, _| w.lean_timers()),
        2,
        "a visible but unfocused window keeps the active workspace awake"
    );
    minutes(vcx, 10);
    view.read_with(vcx, |v, cx| {
        assert!(
            !v.session().unwrap().read(cx).is_lean(),
            "the active workspace stays awake while its window is visible"
        );
    });
}

#[gpui::test]
async fn a_hidden_window_starts_the_lean_timers_and_showing_it_wakes_them(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (w, view, vcx) = open(cx);
    // Covered, minimised or on a sleeping display: nobody looks at it.
    vcx.simulate_visibility_change(WindowVisibility::Hidden);
    until_window(vcx, |window| !window.is_visible());
    assert_eq!(
        w.read_with(vcx, |w, _| w.lean_timers()),
        3,
        "a hidden window puts even the active workspace on a lean timer"
    );
    minutes(vcx, 10);
    support::settle_until(vcx, |cx| {
        w.read_with(cx, |w, cx| w.sessions().all(|s| s.read(cx).is_lean()))
    });
    // Shown again: the active workspace wakes. The two that already went lean
    // stay lean (they wake when activated), so no timer is re-armed.
    vcx.simulate_visibility_change(WindowVisibility::Visible);
    until_window(vcx, |window| window.is_visible());
    support::settle_until(vcx, |cx| {
        view.read_with(cx, |v, cx| !v.session().unwrap().read(cx).is_lean())
    });
    assert_eq!(w.read_with(vcx, |w, _| w.lean_timers()), 0);
}

#[gpui::test]
async fn a_window_opened_after_a_hidden_start_wakes_the_active_workspace(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let providers = accounts();
    let registry = registry(&providers);
    let w = workspaces(cx, &registry);
    // A `--hidden` start with the icon on: no window yet, so the sessions
    // start as if the window were hidden and each one arms its timer.
    w.update(cx, |w, cx| {
        w.set_window_visible(false, cx);
        w.start_all(cx);
    });
    support::settle_until(cx, |cx| {
        w.read_with(cx, |w, _| w.sessions().count() == 3 && w.lean_timers() == 3)
    });
    minutes(cx, 10);
    support::settle_until(cx, |cx| {
        w.read_with(cx, |w, cx| w.sessions().all(|s| s.read(cx).is_lean()))
    });
    // The window opens. Opening does not change the window's visibility, so no
    // event fires: the root view's start state is what wakes the active
    // workspace. If it did not, the first window someone opens would show a
    // lean workspace until they switched away and back.
    let (view, vcx) = window(cx, &registry, &w);
    support::settle(vcx);
    view.read_with(vcx, |v, cx| {
        assert!(
            !v.session().unwrap().read(cx).is_lean(),
            "opening the window wakes the workspace it shows"
        );
    });
    w.read_with(vcx, |w, cx| {
        assert_eq!(w.lean_timers(), 0, "no lean timer is left pending");
        let active = w.active().cloned().expect("an active workspace");
        for id in ["w1", "w2", "w3"] {
            let key = support::key(id);
            let lean = w.session(&key).unwrap().read(cx).is_lean();
            assert_eq!(!lean, key == active, "only the active workspace wakes");
        }
    });
}
