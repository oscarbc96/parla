//! The first-run flow in the main window: Welcome, provider, the provider's
//! sign-in, first sync driven by the session's own progress, then the main
//! window with the tour, which shows once.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use components::onboarding::OnboardingStep;
use components::sign_in::{Candidate, SignIn, SignInError, SignInOption, SignInResult, SignInText};
use components::workspace_picker::RefreshStep;
use domain::test_support::{FakeFactory, FakeProvider, Op, conv, member, uid};
use domain::{ChatProvider, ContentParser, ExtCodec, ProviderError, ProviderFactory};
use futures::channel::mpsc::UnboundedSender;
use futures::future::BoxFuture;
use gpui::{App, Entity, Focusable as _, Task, TestAppContext, VisualTestContext, Window};
use parla::providers::ProviderRegistry;
use parla::root_view::{Phase, RootView};
use settings::{ConfigPaths, ProviderId, WorkspaceKey};
use store::{MemoryStore, Store};

/// Accounts appear once the sign-in ran.
struct LateFactory {
    inner: FakeFactory,
    provider: Arc<FakeProvider>,
    signed_in: Arc<AtomicBool>,
}

impl ProviderFactory for LateFactory {
    fn catalog(&self) -> &'static i18n::Catalog {
        domain::test_support::fake_catalog()
    }
    fn id(&self) -> &ProviderId {
        self.inner.id()
    }
    fn display_name(&self) -> &str {
        "Fake"
    }
    fn accounts(&self) -> Vec<WorkspaceKey> {
        if self.signed_in.load(Ordering::SeqCst) {
            vec![self.provider.workspace().clone()]
        } else {
            vec![]
        }
    }
    fn start(
        &self,
        _: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        let p = self.provider.clone() as Arc<dyn ChatProvider>;
        Box::pin(async move { Ok(p) })
    }
    fn ext_codec(&self) -> Arc<dyn ExtCodec> {
        self.inner.ext_codec()
    }
    fn content_parser(&self) -> Arc<dyn ContentParser> {
        self.inner.content_parser()
    }
}

struct FakeSignIn {
    key: WorkspaceKey,
    signed_in: Arc<AtomicBool>,
}

impl SignIn for FakeSignIn {
    fn provider_name(&self) -> String {
        "Fake".into()
    }
    fn notice(&self) -> Vec<SignInText> {
        vec![]
    }
    fn options(&self) -> Vec<SignInOption> {
        vec![SignInOption {
            id: "window".into(),
            label: "Sign in with browser window".into(),
            fields: vec![],
            help: None,
            copy: None,
        }]
    }
    fn run(
        &self,
        _: &str,
        _: Vec<(String, String)>,
        _: UnboundedSender<RefreshStep>,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        self.signed_in.store(true, Ordering::SeqCst);
        Task::ready(Ok(SignInResult::Workspaces(vec![Candidate {
            key: self.key.clone(),
            name: "w1".into(),
            ready: true,
        }])))
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        self.signed_in.store(false, Ordering::SeqCst);
        Task::ready(Ok(()))
    }
}

struct Fixture {
    registry: ProviderRegistry,
    provider: Arc<FakeProvider>,
}

fn fixture(signed_in: bool) -> Fixture {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_conversation(conv("C2", "random"));
    provider.add_member(member("u1", "Me"));
    let signed_in = Arc::new(AtomicBool::new(signed_in));
    let mut registry = ProviderRegistry::new();
    registry.add(LateFactory {
        inner: FakeFactory::default(),
        provider: provider.clone(),
        signed_in: signed_in.clone(),
    });
    registry.add_sign_in(Rc::new(FakeSignIn {
        key: provider.workspace().clone(),
        signed_in,
    }));
    Fixture { registry, provider }
}

/// Settings in a temp dir and, with `data`, the app's data dir there too (where
/// the tour's dismissal is kept).
fn init(cx: &mut TestAppContext, data: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    let data_dir = dir.path().join("data");
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        if data {
            cx.set_global(parla::removal::AppDirs {
                data: data_dir.clone(),
                cache: data_dir.join("cache"),
            });
        }
    });
    dir
}

fn store() -> Arc<dyn Store> {
    Arc::new(MemoryStore::default())
}

fn step(view: &Entity<RootView>, cx: &mut VisualTestContext) -> Option<OnboardingStep> {
    view.read_with(cx, |v, cx| v.onboarding().map(|o| o.read(cx).step()))
}

/// Welcome and provider by keyboard, then the provider's sign-in.
fn sign_in(view: &Entity<RootView>, cx: &mut VisualTestContext) {
    assert_eq!(step(view, cx), Some(OnboardingStep::Welcome));
    cx.simulate_keystrokes("enter");
    assert_eq!(step(view, cx), Some(OnboardingStep::Provider));
    cx.simulate_keystrokes("enter");
    assert_eq!(step(view, cx), Some(OnboardingStep::SignIn));
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    screen.update_in(cx, |s, window, cx| s.choose("window", window, cx));
}

fn finish(view: &Entity<RootView>, cx: &mut VisualTestContext) {
    let ob = view.read_with(cx, |v, _| v.onboarding().unwrap().clone());
    assert!(ob.read_with(cx, |o, _| o.can_finish()));
    ob.update(cx, |o, cx| o.finish(cx));
    cx.run_until_parked();
}

#[gpui::test]
async fn first_run_goes_welcome_provider_sign_in_first_sync_then_main_window(
    cx: &mut TestAppContext,
) {
    let _dir = init(cx, false);
    let f = fixture(false);
    // The bootstrap waits: the sync stops part way.
    f.provider.hold(Op::Bootstrap);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |v, _| v.phase().clone()),
        Phase::NoAccounts
    );
    sign_in(&view, cx);
    cx.run_until_parked();

    // Signed in: the first-sync screen shows, fed by the session's bootstrap.
    assert_eq!(step(&view, cx), Some(OnboardingStep::FirstSync));
    assert!(view.read_with(cx, |v, _| v.sign_in_screen().is_none()));
    let ob = view.read_with(cx, |v, _| v.onboarding().unwrap().clone());
    let mid = ob.read_with(cx, |o, _| o.workspaces()[0].clone());
    assert!(mid.progress.store_ready, "{mid:?}");
    assert_eq!(mid.progress.directory, None, "{mid:?}");
    let pct = mid.progress.percent();
    assert!(pct > 0 && pct < 100, "{pct}");
    assert!(!ob.read_with(cx, |o, _| o.can_finish()));
    assert!(view.read_with(cx, |v, _| v.tour().is_none()));

    // The bootstrap answers: the directory is in and catch-up runs to the end.
    f.provider.release();
    cx.run_until_parked();
    let done = ob.read_with(cx, |o, _| o.workspaces()[0].clone());
    assert_eq!(done.progress.directory, Some((2, 1)), "{done:?}");
    assert_eq!(done.progress.percent(), 100, "{done:?}");

    // Open Parla: the main window, with the tour over it.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.onboarding().is_none());
        assert!(v.tour().is_some());
        assert_eq!(*v.phase(), Phase::Ready);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workspace-rail").is_some());
    assert!(cx.debug_bounds("tour-coach").is_some());

    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert!(v.tour().is_none()));
    assert!(cx.update(|_, cx| parla::tour_state::dismissed(cx)));
}

#[gpui::test]
async fn the_tour_shows_once(cx: &mut TestAppContext) {
    let dir = init(cx, true);
    let f = fixture(false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    sign_in(&view, cx);
    cx.run_until_parked();
    finish(&view, cx);
    let tour = view.read_with(cx, |v, _| v.tour().unwrap().clone());
    tour.update(cx, |t, cx| t.skip(cx));
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.tour().is_none()));
    assert!(
        dir.path().join("data").join("onboarding.json").exists(),
        "the dismissal is kept in the app state"
    );

    // Signed out and in again: the flow runs, the tour does not come back.
    view.update_in(cx, |v, window, cx| v.sign_out(window, cx));
    cx.run_until_parked();
    sign_in(&view, cx);
    cx.run_until_parked();
    finish(&view, cx);
    assert!(view.read_with(cx, |v, _| v.tour().is_none()));
}

#[gpui::test]
async fn a_saved_dismissal_survives_a_restart(cx: &mut TestAppContext) {
    let dir = init(cx, true);
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::write(
        dir.path().join("data").join("onboarding.json"),
        r#"{"tour_dismissed":true}"#,
    )
    .unwrap();
    assert!(cx.update(parla::tour_state::dismissed));
}

#[gpui::test]
async fn an_existing_account_skips_the_first_run_screens(cx: &mut TestAppContext) {
    let _dir = init(cx, false);
    let f = fixture(true);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.onboarding().is_none());
        assert!(v.tour().is_none());
        assert_eq!(*v.phase(), Phase::Ready);
    });
}

#[gpui::test]
async fn first_sync_waits_for_the_bootstrap(cx: &mut TestAppContext) {
    let _dir = init(cx, false);
    let f = fixture(false);
    f.provider.hold(Op::Bootstrap);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    sign_in(&view, cx);
    cx.run_until_parked();
    let ob = view.read_with(cx, |v, _| v.onboarding().unwrap().clone());
    let before = ob.read_with(cx, |o, _| o.workspaces()[0].clone());
    assert!(before.progress.store_ready);
    assert_eq!(before.progress.directory, None);
    f.provider.release();
    cx.run_until_parked();
    let after = ob.read_with(cx, |o, _| o.workspaces()[0].clone());
    assert!(after.progress.percent() > before.progress.percent());
    assert!(ob.read_with(cx, |o, _| o.can_finish()));
}

#[gpui::test]
async fn signing_out_during_first_sync_forgets_it(cx: &mut TestAppContext) {
    let _dir = init(cx, false);
    let f = fixture(false);
    f.provider.hold(Op::Bootstrap);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    sign_in(&view, cx);
    cx.run_until_parked();
    assert_eq!(step(&view, cx), Some(OnboardingStep::FirstSync));

    view.update_in(cx, |v, window, cx| v.sign_out(window, cx));
    cx.run_until_parked();
    assert_eq!(step(&view, cx), Some(OnboardingStep::Welcome));
    assert_eq!(view.read_with(cx, |v, _| v.first_sync_pending()), 0);

    // The user is on the sign-in screen when a workspace event arrives.
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    let focus = screen.read_with(cx, gpui::Focusable::focus_handle);
    cx.update(|window, cx| window.focus(&focus, cx));
    let workspaces =
        cx.update(|_, cx| cx.global::<parla::workspaces::WorkspacesHandle>().0.clone());
    let key = f.provider.workspace().clone();
    workspaces.update(cx, |w, cx| w.forget(&key, cx));
    f.provider.release();
    cx.run_until_parked();

    assert!(
        cx.update(|window, _| focus.is_focused(window)),
        "focus kept"
    );
    let ob = view.read_with(cx, |v, _| v.onboarding().unwrap().clone());
    assert!(
        ob.read_with(cx, |o, _| o.workspaces().is_empty()),
        "no old session watched"
    );
    assert_eq!(step(&view, cx), Some(OnboardingStep::Welcome));
}

#[gpui::test]
async fn a_failed_bootstrap_still_lets_the_user_open_parla(cx: &mut TestAppContext) {
    let _dir = init(cx, false);
    let f = fixture(false);
    f.provider
        .fail_next(Op::Bootstrap, ProviderError::Other("down".into()));
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    sign_in(&view, cx);
    cx.run_until_parked();
    let ob = view.read_with(cx, |v, _| v.onboarding().unwrap().clone());
    let ws = ob.read_with(cx, |o, _| o.workspaces()[0].clone());
    assert!(ws.progress.failed, "{ws:?}");
    assert!(ob.read_with(cx, |o, _| o.can_finish()));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.onboarding().is_none()));
}

#[gpui::test]
async fn a_corrupt_state_file_reads_as_not_dismissed_and_is_rewritten(cx: &mut TestAppContext) {
    let dir = init(cx, true);
    let file = dir.path().join("data").join("onboarding.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "{not json").unwrap();
    assert!(!cx.update(parla::tour_state::dismissed));
    cx.update(parla::tour_state::dismiss);
    cx.run_until_parked();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(v["tour_dismissed"], serde_json::Value::Bool(true));
}

#[gpui::test]
async fn dismissing_keeps_the_other_app_state_keys(cx: &mut TestAppContext) {
    let dir = init(cx, true);
    let file = dir.path().join("data").join("onboarding.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, r#"{"other":1}"#).unwrap();
    cx.update(parla::tour_state::dismiss);
    cx.run_until_parked();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(v["other"], serde_json::json!(1));
    assert_eq!(v["tour_dismissed"], serde_json::Value::Bool(true));
}

fn composer_text(view: &Entity<RootView>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, cx| {
        v.composer().map(|c| c.read(cx).text(cx).to_string())
    })
}

/// Fresh sign-in to the main window with the tour up and nothing open.
fn tour_up(cx: &mut TestAppContext) -> (Entity<RootView>, &mut VisualTestContext, Fixture) {
    let _ = init(cx, false);
    let f = fixture(false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    sign_in(&view, cx);
    cx.run_until_parked();
    finish(&view, cx);
    assert!(view.read_with(cx, |v, _| v.tour().is_some()));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    (view, cx, f)
}

fn open_c1(view: &Entity<RootView>, cx: &mut VisualTestContext) {
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(domain::test_support::cid("C1"), cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let b = cx.debug_bounds(selector).expect(selector);
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    cx.run_until_parked();
}

fn tour_is_up(view: &Entity<RootView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| v.tour().is_some())
}

/// The tour is a hint card, not a modal: opening a conversation with it up
/// moves the keyboard to the composer, and typing lands there.
#[gpui::test]
async fn typing_works_with_the_tour_up_and_after_it(cx: &mut TestAppContext) {
    let (view, cx, _f) = tour_up(cx);
    open_c1(&view, cx);
    cx.simulate_input("hi");
    assert_eq!(composer_text(&view, cx).as_deref(), Some("hi"));
    assert!(tour_is_up(&view, cx));
    let tour = view.read_with(cx, |v, _| v.tour().unwrap().clone());
    tour.update(cx, |t, cx| t.skip(cx));
    cx.run_until_parked();
    cx.simulate_input("!");
    assert_eq!(composer_text(&view, cx).as_deref(), Some("hi!"));
}

/// Clicking a sidebar row with the tour up opens it and types into its composer.
#[gpui::test]
async fn clicking_a_sidebar_row_with_the_tour_up_lets_you_type(cx: &mut TestAppContext) {
    let (view, cx, _f) = tour_up(cx);
    click(cx, "sidebar-row-C2");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_input("yo");
    assert_eq!(composer_text(&view, cx).as_deref(), Some("yo"));
    assert!(tour_is_up(&view, cx));
}

/// Clicking the composer with the tour up keeps the tour and types there.
#[gpui::test]
async fn clicking_the_composer_with_the_tour_up_lets_you_type(cx: &mut TestAppContext) {
    let (view, cx, _f) = tour_up(cx);
    open_c1(&view, cx);
    click(cx, "text-input");
    cx.simulate_input("hi");
    assert_eq!(composer_text(&view, cx).as_deref(), Some("hi"));
    assert!(tour_is_up(&view, cx));
}

/// "Jump or search" with the tour up: the switcher opens above it (the card
/// steps aside), takes typing, and the card is back once it closes.
#[gpui::test]
async fn the_jump_field_accepts_typing_with_the_tour_up(cx: &mut TestAppContext) {
    let (view, cx, _f) = tour_up(cx);
    assert!(cx.debug_bounds("tour-coach").is_some());
    click(cx, "sidebar-search");
    let switcher = view
        .read_with(cx, |v, _| v.quick_switcher().cloned())
        .expect("switcher opened");
    cx.simulate_input("gen");
    assert_eq!(switcher.read_with(cx, |p, _| p.query().to_string()), "gen");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("tour-coach").is_none(), "card steps aside");
    assert!(tour_is_up(&view, cx));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(view.read_with(cx, |v, _| v.quick_switcher().is_none()));
    assert!(cx.debug_bounds("tour-coach").is_some(), "card is back");
}

/// Sidebar keys work with the tour up, also after a click on empty chrome.
#[gpui::test]
async fn sidebar_keys_work_with_the_tour_up(cx: &mut TestAppContext) {
    let (view, cx, _f) = tour_up(cx);
    let focus = view.read_with(cx, |v, cx| v.sidebar().unwrap().read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&focus, cx));
    let b = cx.debug_bounds("sidebar-search").expect("sidebar");
    // Empty space below the list.
    let blank = gpui::point(b.center().x, gpui::px(700.0));
    cx.simulate_click(blank, gpui::Modifiers::none());
    cx.run_until_parked();
    let pane = |view: &Entity<RootView>, cx: &mut VisualTestContext| {
        cx.update(|window, app| view.read(app).focused_pane_name(window, app))
    };
    assert_eq!(pane(&view, cx), Some("sidebar"), "the click kept the focus");
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(pane(&view, cx), Some("sidebar"));
    let moved = view.read_with(cx, |v, cx| {
        v.sidebar().unwrap().read(cx).cursor_section().is_some()
    });
    assert!(moved, "down moves the sidebar cursor");
    assert!(tour_is_up(&view, cx));
}

/// Escape ends the tour while the card has the keyboard.
#[gpui::test]
async fn escape_ends_the_tour_while_the_card_has_focus(cx: &mut TestAppContext) {
    let (view, cx, _f) = tour_up(cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!tour_is_up(&view, cx));
}

/// A provider whose sign-in lists two workspaces in the native picker. The
/// add finishes when the test fires `done`, from another thread, as the
/// provider's own save does.
struct PickSignIn {
    key: WorkspaceKey,
    signed_in: Arc<AtomicBool>,
    done: std::cell::RefCell<Option<futures::channel::oneshot::Receiver<()>>>,
}

struct PickAdder {
    key: WorkspaceKey,
    signed_in: Arc<AtomicBool>,
    done: std::cell::RefCell<Option<futures::channel::oneshot::Receiver<()>>>,
}

impl components::workspace_picker::WorkspaceAdder for PickAdder {
    fn add(
        &self,
        ids: Vec<String>,
        progress: UnboundedSender<components::workspace_picker::AddStep>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<Candidate>, SignInError>> {
        let _ = progress.unbounded_send(components::workspace_picker::AddStep {
            id: ids[0].clone(),
            state: components::workspace_picker::AddState::SigningIn,
        });
        let done = self.done.borrow_mut().take().unwrap();
        let key = self.key.clone();
        let signed_in = self.signed_in.clone();
        window.spawn(cx, async move |_| {
            let _ = done.await;
            signed_in.store(true, Ordering::SeqCst);
            drop(progress);
            Ok(vec![Candidate {
                key,
                name: "w1".into(),
                ready: true,
            }])
        })
    }
}

impl SignIn for PickSignIn {
    fn provider_name(&self) -> String {
        "Fake".into()
    }
    fn notice(&self) -> Vec<SignInText> {
        vec![]
    }
    fn options(&self) -> Vec<SignInOption> {
        FakeSignIn {
            key: self.key.clone(),
            signed_in: self.signed_in.clone(),
        }
        .options()
    }
    fn run(
        &self,
        _: &str,
        _: Vec<(String, String)>,
        _: UnboundedSender<RefreshStep>,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        let row = |id: &str| components::workspace_picker::WorkspaceChoice {
            id: id.into(),
            name: id.into(),
            domain: id.into(),
            icon: None,
            sso: false,
            two_factor: false,
            signed_out: true,
            added: false,
            updated: false,
            updating: false,
            update_error: false,
            needs_sign_in: false,
        };
        Task::ready(Ok(SignInResult::Pick(
            components::sign_in::WorkspacePickerSession {
                choices: vec![row("w1"), row("w2")],
                adder: Rc::new(PickAdder {
                    key: self.key.clone(),
                    signed_in: self.signed_in.clone(),
                    done: std::cell::RefCell::new(self.done.borrow_mut().take()),
                }),
                refreshed: Vec::new(),
                refresh: Vec::new(),
            },
        )))
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        Task::ready(Ok(()))
    }
}

/// A route regression test only: the picker, the sign-in screen and the root
/// advance to first sync when the add finishes without the window closing. It
/// does not drive a provider's webview capture (the provider's own sequence
/// test covers the resumed handoff that left the picker on "Signing in").
#[gpui::test]
async fn the_picker_advances_to_first_sync_when_the_sign_in_finishes(cx: &mut TestAppContext) {
    let _dir = init(cx, false);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    provider.hold(Op::Bootstrap);
    let signed_in = Arc::new(AtomicBool::new(false));
    let (fire, done) = futures::channel::oneshot::channel();
    let mut registry = ProviderRegistry::new();
    registry.add(LateFactory {
        inner: FakeFactory::default(),
        provider: provider.clone(),
        signed_in: signed_in.clone(),
    });
    registry.add_sign_in(Rc::new(PickSignIn {
        key: provider.workspace().clone(),
        signed_in,
        done: std::cell::RefCell::new(Some(done)),
    }));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    sign_in(&view, cx);
    cx.run_until_parked();
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    let components::sign_in::Step::PickWorkspaces(picker) =
        screen.read_with(cx, |s, _| s.step().clone())
    else {
        panic!("the picker is open");
    };
    picker.update_in(cx, |p, window, cx| {
        p.choose("w1", cx);
        p.add(window, cx);
    });
    cx.run_until_parked();
    assert_eq!(step(&view, cx), Some(OnboardingStep::SignIn));

    // The sign-in finishes in the background: no click, no window closed.
    let _ = fire.send(());
    cx.run_until_parked();
    assert_eq!(step(&view, cx), Some(OnboardingStep::FirstSync));
}

#[gpui::test]
async fn the_tour_spotlight_is_the_sidebar_bounds_with_a_banner_shown(cx: &mut TestAppContext) {
    let _dir = init(cx, false);
    cx.update(|cx| {
        cx.global_mut::<parla::toast::ConfigBanner>()
            .warnings
            .push("a configuration warning".to_string());
    });
    let f = fixture(false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    sign_in(&view, cx);
    cx.run_until_parked();
    finish(&view, cx);
    assert!(view.read_with(cx, |v, _| v.tour().is_some()));
    // The measured bounds arrive after the first paint; the tour redraws then.
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }
    let sidebar = cx.debug_bounds("workspace-sidebar").expect("sidebar");
    let banner = cx.debug_bounds("config-banner").expect("banner");
    let spot = cx.debug_bounds("tour-spotlight").expect("spotlight");
    assert_eq!(spot, sidebar, "the spotlight is the sidebar's own bounds");
    assert!(spot.bottom() <= banner.top(), "{spot:?} over {banner:?}");
}
