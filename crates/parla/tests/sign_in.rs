//! Sign-in screen, expiry banner and sign out in the main window, against a
//! fake provider and a fake `SignIn`: no webview, network or keychain.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use components::onboarding::OnboardingStep;
use components::sign_in::{Candidate, SignIn, SignInError, SignInOption, SignInResult, SignInText};
use components::workspace_picker::{RefreshOutcome, RefreshStep};
use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, uid};
use domain::{
    ChatProvider, ContentParser, ExtCodec, ProviderError, ProviderEvent, ProviderFactory,
};
use futures::channel::mpsc::UnboundedSender;
use futures::future::BoxFuture;
use gpui::{App, Task, TestAppContext, VisualTestContext, Window};
use parla::providers::ProviderRegistry;
use parla::root_view::{Phase, RootView};
use settings::{ConfigPaths, ProviderId, WorkspaceKey};
use store::{MemoryStore, Store};

/// Accounts appear after `signed_in` is set; `start` fails with an expired
/// session while `expired` is set.
struct LateFactory {
    inner: FakeFactory,
    provider: Arc<FakeProvider>,
    signed_in: Arc<AtomicBool>,
    expired: Arc<AtomicBool>,
    starts: Arc<AtomicU32>,
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
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.expired.load(Ordering::SeqCst) {
            return Box::pin(async { Err(ProviderError::SessionExpired) });
        }
        let p = self.provider.clone() as Arc<dyn ChatProvider>;
        let ws = ws.clone();
        Box::pin(async move {
            assert_eq!(&ws, p.workspace());
            Ok(p)
        })
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
    expired: Arc<AtomicBool>,
    runs: Rc<RefCell<u32>>,
    outs: Rc<RefCell<Vec<WorkspaceKey>>>,
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
        *self.runs.borrow_mut() += 1;
        self.signed_in.store(true, Ordering::SeqCst);
        self.expired.store(false, Ordering::SeqCst);
        Task::ready(Ok(SignInResult::Workspaces(vec![Candidate {
            key: self.key.clone(),
            name: "w1".into(),
            ready: true,
        }])))
    }
    fn sign_out(&self, ws: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        self.outs.borrow_mut().push(ws.clone());
        self.signed_in.store(false, Ordering::SeqCst);
        Task::ready(Ok(()))
    }
}

struct Fixture {
    registry: ProviderRegistry,
    provider: Arc<FakeProvider>,
    signed_in: Arc<AtomicBool>,
    expired: Arc<AtomicBool>,
    runs: Rc<RefCell<u32>>,
    outs: Rc<RefCell<Vec<WorkspaceKey>>>,
}

fn fixture(signed_in: bool, expired: bool) -> Fixture {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    let signed_in = Arc::new(AtomicBool::new(signed_in));
    let expired = Arc::new(AtomicBool::new(expired));
    let starts = Arc::new(AtomicU32::new(0));
    let runs = Rc::default();
    let outs = Rc::default();
    let mut registry = ProviderRegistry::new();
    registry.add(LateFactory {
        inner: FakeFactory::default(),
        provider: provider.clone(),
        signed_in: signed_in.clone(),
        expired: expired.clone(),
        starts,
    });
    registry.add_sign_in(Rc::new(FakeSignIn {
        key: provider.workspace().clone(),
        signed_in: signed_in.clone(),
        expired: expired.clone(),
        runs: Rc::clone(&runs),
        outs: Rc::clone(&outs),
    }));
    Fixture {
        registry,
        provider,
        signed_in,
        expired,
        runs,
        outs,
    }
}

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
async fn no_accounts_shows_sign_in_and_success_starts_session(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(false, false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(*v.phase(), Phase::NoAccounts);
        assert!(v.sign_in_screen().is_some());
    });
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    screen.update_in(cx, |s, window, cx| s.choose("window", window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(*v.phase(), Phase::Ready);
        assert!(v.session().is_some());
        assert!(v.sign_in_screen().is_none());
    });
}

#[gpui::test]
async fn expired_shows_banner_and_reauth_restarts_keeping_conversation(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(true, false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    assert!(!view.update(cx, |v, cx| v.expiry_banner_visible(cx)));

    f.expired.store(true, Ordering::SeqCst);
    f.provider.push_event(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    assert!(view.update(cx, |v, cx| v.expiry_banner_visible(cx)));

    view.update_in(cx, |v, window, cx| v.sign_in_again(window, cx));
    assert!(view.read_with(cx, |v, _| v.sign_in_screen().is_some()));
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    screen.update_in(cx, |s, window, cx| s.choose("window", window, cx));
    cx.run_until_parked();
    assert_eq!(*f.runs.borrow(), 1);
    view.read_with(cx, |v, _| {
        assert_eq!(*v.phase(), Phase::Ready);
        assert!(v.sign_in_screen().is_none());
        assert_eq!(v.shown_conversation(), Some(&cid("C1")));
    });
    assert!(!view.update(cx, |v, cx| v.expiry_banner_visible(cx)));
}

#[derive(Clone, Default)]
struct ExpiredNotifier(Rc<RefCell<Vec<WorkspaceKey>>>);

impl parla::notifications::Notifier for ExpiredNotifier {
    fn show(&self, _: &domain::notify::Notification, _: &mut App) {}
    fn dismiss_conversation(&self, _: &WorkspaceKey, _: &domain::ConversationId, _: &mut App) {}
    fn set_badge(&self, _: Option<u32>, _: &mut App) {}
    fn show_session_expired(&self, n: &domain::notify::SessionExpiredNotice, _: &mut App) {
        self.0.borrow_mut().push(n.workspace.clone());
    }
}

#[gpui::test]
async fn expired_at_startup_posts_one_notification(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let notifier = ExpiredNotifier::default();
    cx.update(|cx| parla::notifications::Notifications::new(Rc::new(notifier.clone())).install(cx));
    let f = fixture(true, true);
    let (_view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    assert_eq!(*notifier.0.borrow(), vec![f.provider.workspace().clone()]);
}

#[gpui::test]
async fn expired_at_startup_offers_sign_in_again(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(true, true);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.phase().clone()), Phase::Expired);
    assert!(view.update(cx, |v, cx| v.expiry_banner_visible(cx)));
    view.update_in(cx, |v, window, cx| v.sign_in_again(window, cx));
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    screen.update_in(cx, |s, window, cx| s.choose("window", window, cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.phase().clone()), Phase::Ready);
}

#[gpui::test]
async fn sign_out_deletes_secrets_and_returns_to_sign_in(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(true, false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| v.sign_out(window, cx));
    cx.run_until_parked();
    assert_eq!(*f.outs.borrow(), vec![f.provider.workspace().clone()]);
    assert!(!f.signed_in.load(Ordering::SeqCst));
    view.read_with(cx, |v, _| {
        assert_eq!(*v.phase(), Phase::NoAccounts);
        assert!(v.session().is_none());
        assert!(v.sign_in_screen().is_some());
    });
}

#[gpui::test]
async fn add_workspace_opens_its_own_window_and_the_main_window_keeps_working(
    cx: &mut TestAppContext,
) {
    let _dir = init(cx);
    let f = fixture(true, false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);

    view.update_in(cx, |v, window, cx| v.add_workspace(window, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2, "sign-in window");
    view.read_with(cx, |v, _| {
        assert_eq!(*v.phase(), Phase::Ready);
        assert!(v.session().is_some(), "the main window keeps the workspace");
        assert!(v.sign_in_screen().is_some());
    });
    // The main window still renders the workspace, not the sign-in screen.
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("workspace-rail").is_some());
    assert!(cx.debug_bounds("sign-in-screen").is_none());

    // A second request focuses the open window instead of opening another.
    view.update_in(cx, |v, window, cx| v.add_workspace(window, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
}

#[gpui::test]
async fn cancelling_the_sign_in_window_closes_it_and_keeps_the_workspace(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(true, false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| v.add_workspace(window, cx));
    cx.run_until_parked();
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    screen.update(cx, |s, cx| s.cancel(cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
    view.read_with(cx, |v, _| {
        assert!(v.sign_in_screen().is_none());
        assert_eq!(*v.phase(), Phase::Ready);
    });
    // It can be opened again afterwards.
    view.update_in(cx, |v, window, cx| v.add_workspace(window, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
}

#[gpui::test]
async fn signing_in_through_the_window_closes_it_and_shows_the_workspace(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(true, false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| v.add_workspace(window, cx));
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    screen.update_in(cx, |s, window, cx| s.choose("window", window, cx));
    cx.run_until_parked();
    assert_eq!(*f.runs.borrow(), 1);
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
    view.read_with(cx, |v, _| {
        assert_eq!(*v.phase(), Phase::Ready);
        assert!(v.session().is_some());
        assert!(v.sign_in_screen().is_none());
    });
}
struct ReportingSignIn {
    key: WorkspaceKey,
}

impl SignIn for ReportingSignIn {
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
        progress: UnboundedSender<RefreshStep>,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        let _ = progress.unbounded_send(RefreshStep {
            id: self.key.id.0.clone(),
            outcome: RefreshOutcome::Updated(self.key.clone()),
        });
        cx.spawn(async move |_| std::future::pending().await)
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        Task::ready(Ok(()))
    }
}

#[gpui::test]
async fn closing_the_sign_in_window_by_hand_restarts_a_reported_refresh(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    let signed_in = Arc::new(AtomicBool::new(true));
    let expired = Arc::new(AtomicBool::new(false));
    let starts = Arc::new(AtomicU32::new(0));
    let mut registry = ProviderRegistry::new();
    registry.add(LateFactory {
        inner: FakeFactory::default(),
        provider: provider.clone(),
        signed_in: signed_in.clone(),
        expired: expired.clone(),
        starts: starts.clone(),
    });
    registry.add_sign_in(Rc::new(ReportingSignIn {
        key: provider.workspace().clone(),
    }));

    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store(), window, cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.phase().clone()), Phase::Ready);
    let started = starts.load(Ordering::SeqCst);
    assert!(started >= 1);

    view.update_in(cx, |v, window, cx| v.add_workspace(window, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2, "sign-in window");

    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    screen.update_in(cx, |s, window, cx| s.choose("window", window, cx));
    cx.run_until_parked();

    let sign_in_window = cx
        .update(|_, cx| {
            cx.windows().into_iter().find(|w| {
                w.downcast::<parla::sign_in_window::SignInWindow>()
                    .is_some()
            })
        })
        .expect("sign-in window handle");
    sign_in_window
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();

    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
    assert!(
        starts.load(Ordering::SeqCst) > started,
        "workspace restarted"
    );
    assert_eq!(view.read_with(cx, |v, _| v.phase().clone()), Phase::Ready);
}

#[gpui::test]
async fn add_workspace_starts_at_welcome_then_the_provider_picker(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(true, false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| v.add_workspace(window, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2, "sign-in window");
    let ob = view
        .read_with(cx, |v, _| v.sign_in_window_onboarding().cloned())
        .expect("the window starts with the first-run screens");
    assert_eq!(ob.read_with(cx, |o, _| o.step()), OnboardingStep::Welcome);
    // Get started, then the picker.
    view.update_in(cx, |v, _window, cx| {
        let ob = v.sign_in_window_onboarding().unwrap().clone();
        ob.update(cx, |o, cx| o.next(cx));
    });
    assert_eq!(ob.read_with(cx, |o, _| o.step()), OnboardingStep::Provider);
    // Back to Welcome, and back again from Welcome closes the window.
    view.update_in(cx, |v, _window, cx| {
        let ob = v.sign_in_window_onboarding().unwrap().clone();
        ob.update(cx, |o, cx| o.back(cx));
    });
    assert_eq!(ob.read_with(cx, |o, _| o.step()), OnboardingStep::Welcome);
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    view.update_in(cx, |v, _window, cx| {
        let ob = v.sign_in_window_onboarding().unwrap().clone();
        ob.update(cx, |o, cx| o.back(cx));
    });
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
    view.read_with(cx, |v, _| {
        assert!(v.sign_in_window_onboarding().is_none());
        assert_eq!(*v.phase(), Phase::Ready);
    });
}

#[gpui::test]
async fn sign_in_again_skips_the_welcome(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(true, false);
    let (view, cx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| v.sign_in_again(window, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    view.read_with(cx, |v, _| {
        assert!(v.sign_in_window_onboarding().is_none());
        assert!(v.sign_in_screen().is_some());
    });
}

/// A context on the add-workspace window, drawn once so selectors resolve.
fn sign_in_window_cx(cx: &mut VisualTestContext) -> VisualTestContext {
    let handle = cx
        .update(|_, cx| {
            cx.windows().into_iter().find(|w| {
                w.downcast::<parla::sign_in_window::SignInWindow>()
                    .is_some()
            })
        })
        .expect("sign-in window handle");
    let mut wcx = VisualTestContext::from_window(handle, cx);
    wcx.run_until_parked();
    wcx.update(|window, cx| window.draw(cx).clear(cx));
    wcx
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let b = cx.debug_bounds(selector).expect(selector);
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    if selector != "onboarding-cancel" {
        // The page push is a slide: draw to start its clock, then let it
        // settle before the next click.
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for _ in 0..2 {
            cx.executor().advance_clock(
                themes::metrics::DURATION_BASE + std::time::Duration::from_millis(1),
            );
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
    }
}

#[gpui::test]
async fn escape_on_welcome_closes_the_add_workspace_window(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(true, false);
    let (view, vcx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    vcx.run_until_parked();
    view.update_in(vcx, |v, window, cx| v.add_workspace(window, cx));
    let mut wcx = sign_in_window_cx(vcx);
    assert!(wcx.debug_bounds("onboarding-next").is_some(), "Welcome");
    wcx.simulate_keystrokes("escape");
    wcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| cx.windows().len()), 1);
    view.read_with(vcx, |v, _| {
        assert!(v.sign_in_window_onboarding().is_none());
        assert_eq!(*v.phase(), Phase::Ready);
    });
}

#[gpui::test]
async fn welcome_has_a_cancel_button_and_a_four_step_flow(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let f = fixture(true, false);
    let (view, vcx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    vcx.run_until_parked();
    view.update_in(vcx, |v, window, cx| v.add_workspace(window, cx));
    let mut wcx = sign_in_window_cx(vcx);
    let ob = view
        .read_with(vcx, |v, _| v.sign_in_window_onboarding().cloned())
        .unwrap();
    assert!(ob.read_with(&wcx, |o, _| o.is_added_workspace()));
    // Get started goes to the picker, whose Back returns to Welcome.
    click(&mut wcx, "onboarding-next");
    assert_eq!(
        ob.read_with(&wcx, |o, _| o.step()),
        OnboardingStep::Provider
    );
    click(&mut wcx, "onboarding-back");
    assert_eq!(ob.read_with(&wcx, |o, _| o.step()), OnboardingStep::Welcome);
    // Cancel closes the window.
    click(&mut wcx, "onboarding-cancel");
    assert_eq!(vcx.update(|_, cx| cx.windows().len()), 1);
    view.read_with(vcx, |v, _| assert_eq!(*v.phase(), Phase::Ready));
}

#[gpui::test]
async fn picking_a_provider_and_signing_in_through_the_window_adds_the_workspace(
    cx: &mut TestAppContext,
) {
    let _dir = init(cx);
    let mut f = fixture(true, false);
    f.registry.add_sign_in(Rc::new(FakeSignIn {
        key: f.provider.workspace().clone(),
        signed_in: f.signed_in.clone(),
        expired: f.expired.clone(),
        runs: Rc::clone(&f.runs),
        outs: Rc::clone(&f.outs),
    }));
    let (view, vcx) =
        cx.add_window_view(|window, cx| RootView::new(&f.registry, store(), window, cx));
    vcx.run_until_parked();
    view.update_in(vcx, |v, window, cx| v.add_workspace(window, cx));
    let mut wcx = sign_in_window_cx(vcx);
    let first = view.read_with(vcx, |v, _| v.sign_in_screen().unwrap().clone());
    click(&mut wcx, "onboarding-next");
    click(&mut wcx, "provider-card-1");
    click(&mut wcx, "onboarding-next");
    let ob = view
        .read_with(vcx, |v, _| v.sign_in_window_onboarding().cloned())
        .unwrap();
    assert_eq!(ob.read_with(&wcx, |o, _| o.step()), OnboardingStep::SignIn);
    let screen = view.read_with(vcx, |v, _| v.sign_in_screen().unwrap().clone());
    assert_ne!(screen.entity_id(), first.entity_id(), "the chosen provider");
    // The new screen holds the focus, not the dropped one.
    wcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(screen.read(cx), cx);
        assert!(focus.contains_focused(window, cx));
    });
    screen.update_in(&mut wcx, |s, window, cx| s.choose("window", window, cx));
    wcx.run_until_parked();
    assert_eq!(*f.runs.borrow(), 1);
    assert_eq!(vcx.update(|_, cx| cx.windows().len()), 1, "window closed");
    view.read_with(vcx, |v, _| {
        assert_eq!(*v.phase(), Phase::Ready);
        assert!(v.session().is_some(), "account started");
        assert!(v.sign_in_screen().is_none());
        assert!(v.sign_in_window_onboarding().is_none());
        assert_eq!(v.first_sync_pending(), 0, "no first sync");
        assert!(v.onboarding().is_none());
        assert!(v.tour().is_none(), "no tour");
    });
}
