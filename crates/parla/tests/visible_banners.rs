//! The sign-in screen and every banner must be in the painted view tree and
//! inside the window, not only present in the view's state.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use components::sign_in::{Candidate, SignIn, SignInError, SignInOption, SignInResult, SignInText};
use components::workspace_picker::RefreshStep;
use domain::test_support::{FakeFactory, FakeProvider, conv, member, uid};
use domain::{ChatProvider, ContentParser, ExtCodec, ProviderError, ProviderFactory};
use futures::channel::mpsc::UnboundedSender;
use futures::future::BoxFuture;
use gpui::{App, Bounds, Pixels, Task, TestAppContext, VisualTestContext, Window};
use parla::providers::ProviderRegistry;
use parla::root_view::{Phase, RootView, SqliteHandle};
use parla::toast::ConfigBanner;
use settings::{ConfigPaths, ProviderId, WorkspaceKey};
use store::{MemoryStore, Store, StoreError};
use store_sqlite::{DbId, Key, KeySource, SqliteConfig, SqliteStore};

/// One account whose `start` fails with `err` (or hands out the provider).
struct Factory {
    inner: FakeFactory,
    provider: Arc<FakeProvider>,
    err: Option<fn() -> ProviderError>,
    /// While set, `start` fails with an expired session.
    expired: Arc<AtomicBool>,
}

impl ProviderFactory for Factory {
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
        vec![self.provider.workspace().clone()]
    }
    fn start(
        &self,
        _: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        let r = match self.err {
            _ if self.expired.load(Ordering::SeqCst) => Err(ProviderError::SessionExpired),
            Some(e) => Err(e()),
            None => Ok(self.provider.clone() as Arc<dyn ChatProvider>),
        };
        Box::pin(async move { r })
    }
    fn ext_codec(&self) -> Arc<dyn ExtCodec> {
        self.inner.ext_codec()
    }
    fn content_parser(&self) -> Arc<dyn ContentParser> {
        self.inner.content_parser()
    }
}

struct NoSignIn(Arc<AtomicBool>);

impl SignIn for NoSignIn {
    fn provider_name(&self) -> String {
        "Fake".into()
    }
    fn notice(&self) -> Vec<SignInText> {
        vec![]
    }
    fn options(&self) -> Vec<SignInOption> {
        vec![SignInOption {
            id: "window".into(),
            label: "Sign in".into(),
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
        self.0.store(false, Ordering::SeqCst);
        Task::ready(Ok(SignInResult::Workspaces(vec![Candidate {
            key: WorkspaceKey::new("fake", "w1"),
            name: "w1".into(),
            ready: true,
        }])))
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        Task::ready(Ok(()))
    }
}

/// A key source that never yields a key: the store cannot be opened.
struct NoKeys;

impl KeySource for NoKeys {
    fn get_or_create(&self, _: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        Box::pin(async { Err(StoreError::Unreadable) })
    }
    fn replace(&self, _: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        Box::pin(async { Err(StoreError::Unreadable) })
    }
    fn delete(&self, _: &DbId) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Ok(()) })
    }
}

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        parla::init_without_watcher(cx, ConfigPaths::at(dir.path()));
        // Tests never depend on the OS language.
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn registry(accounts: bool, err: Option<fn() -> ProviderError>) -> ProviderRegistry {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    registry_with(provider, accounts, err)
}

fn registry_with(
    provider: Arc<FakeProvider>,
    accounts: bool,
    err: Option<fn() -> ProviderError>,
) -> ProviderRegistry {
    let expired = Arc::new(AtomicBool::new(
        matches!(err, Some(e) if matches!(e(), ProviderError::SessionExpired)),
    ));
    let err = err.filter(|e| !matches!(e(), ProviderError::SessionExpired));
    let mut r = ProviderRegistry::new();
    if accounts {
        r.add(Factory {
            inner: FakeFactory::default(),
            provider,
            err,
            expired: expired.clone(),
        });
    } else {
        r.add(FakeFactory::default());
    }
    r.add_sign_in(Rc::new(NoSignIn(expired)));
    r
}

fn store() -> Arc<dyn Store> {
    Arc::new(MemoryStore::default())
}

fn window_bounds(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.update(|window, _| window.bounds())
}

/// Draws the window and asserts `selector` was painted, non-empty and inside it.
fn assert_painted(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let win = window_bounds(cx);
    let b = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("`{selector}` is not in the painted view tree"));
    assert!(
        b.size.width > gpui::px(0.0) && b.size.height > gpui::px(0.0),
        "`{selector}` has no area: {b:?}"
    );
    assert!(
        b.origin.x >= win.origin.x
            && b.origin.y >= win.origin.y
            && b.bottom_right().x <= win.bottom_right().x
            && b.bottom_right().y <= win.bottom_right().y,
        "`{selector}` {b:?} is outside the window {win:?}"
    );
}

#[gpui::test]
async fn no_accounts_paints_the_sign_in_screen(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let r = registry(false, None);
    let (_view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    assert_painted(cx, "sign-in-screen");
}

#[gpui::test]
async fn expired_at_start_paints_the_expiry_banner(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let r = registry(true, Some(|| ProviderError::SessionExpired));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.phase().clone()), Phase::Expired);
    assert_painted(cx, "expiry-banner");
    assert_painted(cx, "sign-in-again");
}

#[gpui::test]
async fn sign_in_again_from_the_expired_start_paints_the_sign_in_screen(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let r = registry(true, Some(|| ProviderError::SessionExpired));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| v.sign_in_again(window, cx));
    cx.run_until_parked();
    // Sign-in opens in its own window; the main window keeps the banner.
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    assert_painted(cx, "expiry-banner");
}

/// The sign-in-again button is a Tab stop, so Enter and Space must activate it.
async fn expired_button_activates_on(cx: &mut TestAppContext, key: &str) {
    let _dir = init(cx);
    let r = registry(true, Some(|| ProviderError::SessionExpired));
    let (_view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 1);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    // The rail comes first in Tab order: its workspace tile (one account),
    // then whichever of add, Settings and the own avatar are drawn.
    let rail_stops = 1 + ["rail-add", "rail-settings", "rail-self"]
        .into_iter()
        .filter(|s| cx.debug_bounds(s).is_some())
        .count();
    for _ in 0..=rail_stops {
        cx.update(|window, cx| window.focus_next(cx));
    }
    assert!(
        cx.update(|w, cx| w.focused(cx).is_some()),
        "Tab reaches the button"
    );
    cx.simulate_keystrokes(key);
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| cx.windows().len()),
        2,
        "{key} did nothing"
    );
}

#[gpui::test]
async fn expired_button_activates_on_enter(cx: &mut TestAppContext) {
    expired_button_activates_on(cx, "enter").await;
}

#[gpui::test]
async fn expired_button_activates_on_space(cx: &mut TestAppContext) {
    expired_button_activates_on(cx, "space").await;
}

#[gpui::test]
async fn unreadable_store_paints_the_rebuild_banner(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let db_dir = tempfile::tempdir().unwrap();
    let db = SqliteStore::start(
        SqliteConfig {
            dir: db_dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(NoKeys),
    );
    cx.update(|cx| cx.set_global(SqliteHandle(db.clone())));
    let r = registry(true, None);
    let (_view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.executor().allow_parking();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        if cx.debug_bounds("store-banner").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_painted(cx, "store-banner");
    assert_painted(cx, "rebuild-cache");
    db.shutdown();
}

#[gpui::test]
async fn every_banner_paints_inside_the_window(cx: &mut TestAppContext) {
    let _dir = init(cx);
    cx.update(|cx| {
        cx.global_mut::<ConfigBanner>()
            .warnings
            .push("a configuration warning".to_string());
    });
    let r = registry(true, Some(|| ProviderError::Other("boom".into())));
    let (_view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    // Failed start: error strip over the failure message, plus the config banner.
    assert_painted(cx, "error-banner");
    assert_painted(cx, "config-banner");
}

#[gpui::test]
async fn signing_in_again_over_an_unreadable_store_paints_the_rebuild_banner(
    cx: &mut TestAppContext,
) {
    let _dir = init(cx);
    let db_dir = tempfile::tempdir().unwrap();
    let db = SqliteStore::start(
        SqliteConfig {
            dir: db_dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(NoKeys),
    );
    cx.update(|cx| cx.set_global(SqliteHandle(db.clone())));
    let r = registry(true, Some(|| ProviderError::SessionExpired));
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.run_until_parked();
    assert_painted(cx, "expiry-banner");
    view.update_in(cx, |v, window, cx| v.sign_in_again(window, cx));
    let screen = view.read_with(cx, |v, _| v.sign_in_screen().unwrap().clone());
    screen.update_in(cx, |s, window, cx| s.choose("window", window, cx));
    cx.executor().allow_parking();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        if cx.debug_bounds("store-banner").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(view.read_with(cx, |v, _| v.phase().clone()), Phase::Ready);
    assert_painted(cx, "store-banner");
    db.shutdown();
}

#[gpui::test]
async fn long_banner_text_wraps_inside_a_narrow_window(cx: &mut TestAppContext) {
    let _dir = init(cx);
    cx.update(|cx| {
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("ca".parse().unwrap()));
        cx.global_mut::<ConfigBanner>().warnings.push(
            "una advertencia de ajustes bastante larga que no cabe en una sola linea".to_string(),
        );
    });
    let db_dir = tempfile::tempdir().unwrap();
    let db = SqliteStore::start(
        SqliteConfig {
            dir: db_dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(NoKeys),
    );
    cx.update(|cx| cx.set_global(SqliteHandle(db.clone())));
    cx.executor().allow_parking();
    let r = registry(true, None);
    let (_view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(800.0), gpui::px(600.0)));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        if cx.debug_bounds("store-banner").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_painted(cx, "store-banner");
    assert_painted(cx, "rebuild-cache");
    assert_painted(cx, "config-banner");
    db.shutdown();
}

#[gpui::test]
async fn offline_paints_the_banner_and_retry_now_reconnects(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    let r = registry_with(provider.clone(), true, None);
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.phase().clone()), Phase::Ready);
    provider.push_event(domain::ProviderEvent::Connected { reconnect: false });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("offline-banner").is_none());
    provider.push_event(domain::ProviderEvent::Disconnected);
    cx.run_until_parked();
    assert_painted(cx, "offline-banner");
    assert_painted(cx, "offline-retry-now");
    let b = cx.debug_bounds("offline-retry-now").unwrap();
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(
        provider
            .calls()
            .iter()
            .any(|c| matches!(c, domain::test_support::Call::ReconnectNow))
    );
    // The connection is back: the banner goes away.
    provider.push_event(domain::ProviderEvent::Connected { reconnect: true });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("offline-banner").is_none());
}

#[gpui::test]
async fn low_disk_space_paints_its_banner_until_space_returns(cx: &mut TestAppContext) {
    use parla::store_usage::StoreUsage;
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    let r = registry_with(provider.clone(), true, None);
    let (_view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("store-full-banner").is_none());
    // Under 1 GiB free.
    cx.update(|_, cx| StoreUsage::record(100 * 1024 * 1024, cx));
    cx.run_until_parked();
    assert_painted(cx, "store-full-banner");
    assert_painted(cx, "store-full-manage");
    cx.update(|_, cx| StoreUsage::record(4 * 1024 * 1024 * 1024, cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("store-full-banner").is_none());
}

#[gpui::test]
async fn an_expired_session_hides_the_offline_banner(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    let r = registry_with(provider.clone(), true, None);
    let (_view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.run_until_parked();
    provider.push_event(domain::ProviderEvent::Disconnected);
    provider.push_event(domain::ProviderEvent::SessionExpired);
    cx.run_until_parked();
    assert_painted(cx, "expiry-banner");
    assert!(cx.debug_bounds("offline-banner").is_none());
}

#[gpui::test]
async fn a_failed_bootstrap_shows_its_own_retry_not_the_offline_banner(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    provider.fail_next(
        domain::test_support::Op::Bootstrap,
        domain::ProviderError::Network("down".into()),
    );
    let r = registry_with(provider.clone(), true, None);
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, store(), window, cx));
    cx.run_until_parked();
    assert_painted(cx, "retry-bootstrap");
    assert!(
        cx.debug_bounds("offline-banner").is_none(),
        "its Retry now would do nothing before a bootstrap"
    );
    assert!(!view.read_with(cx, |v, cx| v.offline_banner_visible(cx)));
}
