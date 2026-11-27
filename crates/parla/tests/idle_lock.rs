//! Lock store when idle (`storage.lock_after_minutes`): after that long
//! without input every workspace closes its encrypted store, the cached store
//! keys are forgotten, and Unlock reopens the active one with the key read
//! again. One timer while the setting is on, none while it is off.

mod support;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use gpui::{AppContext as _, Entity, TestAppContext};
use parla::idle_lock::IdleLock;
use parla::workspaces::Workspaces;
use store::StoreError;
use store_sqlite::{DbId, Key, KeySource, MemoryKeySource};
use support::*;

const FIFTEEN: &str = r#"{ "storage": { "lock_after_minutes": 15 } }"#;

fn minutes(cx: &mut TestAppContext, m: u64) {
    cx.executor().advance_clock(Duration::from_secs(m * 60));
    settle(cx);
}

/// Counts how often the lock asked to forget the cached keys, and fails key
/// reads while `fail` is set (a keychain error or a cancelled prompt).
#[derive(Default)]
struct Forgets {
    inner: MemoryKeySource,
    forgets: AtomicUsize,
    fail: std::sync::atomic::AtomicBool,
}

impl KeySource for Forgets {
    fn get_or_create(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        if self.fail.load(Ordering::SeqCst) {
            return Box::pin(async { Err(StoreError::Other("keychain: cancelled".into())) });
        }
        self.inner.get_or_create(ws)
    }
    fn replace(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        self.inner.replace(ws)
    }
    fn delete(&self, ws: &DbId) -> BoxFuture<'_, Result<(), StoreError>> {
        self.inner.delete(ws)
    }
    fn forget_cached(&self) -> BoxFuture<'_, ()> {
        self.forgets.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }
}

/// No OS idle clock: the lock counts from the last input it was told about.
fn app_input_only() -> parla::idle_lock::IdleSource {
    Rc::new(|_| None)
}

fn install(
    cx: &mut TestAppContext,
    w: &Entity<Workspaces>,
    keys: Arc<Forgets>,
    source: parla::idle_lock::IdleSource,
) -> Entity<IdleLock> {
    let w = w.clone();
    // The lock postpones itself a full period while a store is still loading
    // (`ready_to_lock`), so a test that moves the clock must start from ready
    // (unless it checks the not-yet-started case).
    if w.read_with(cx, |w, _| w.is_started()) {
        wait_ready(cx, &w);
    }
    let lock = cx.update(|cx| parla::idle_lock::install(cx, w, Some(keys), source));
    settle(cx);
    lock
}

/// Wait (real time, bounded) until every workspace has loaded its store: how
/// long that takes depends on machine load, the fake clock does not.
fn wait_ready(cx: &mut TestAppContext, w: &Entity<Workspaces>) {
    let w = w.clone();
    settle_until(cx, move |cx| w.read_with(cx, |w, cx| w.ready_to_lock(cx)));
}

/// After the clock passed the deadline: wait until the lock landed and every
/// workspace closed its store (the close answers from other threads).
fn wait_locked(cx: &mut TestAppContext, lock: &Entity<IdleLock>, w: &Entity<Workspaces>, e: &Env) {
    let (lock, w) = (lock.clone(), w.clone());
    settle_until(cx, move |cx| {
        lock.read_with(cx, |l, _| l.is_locked()) && all_lean(&w, cx)
    });
    for id in ["w1", "w2", "w3"] {
        wait_store_closed(cx, e, id);
    }
}

/// After Unlock: wait until the lock lifted and the workspace looked at woke
/// (its store reopened on its thread) and every store is settled again.
fn wait_unlocked(
    cx: &mut TestAppContext,
    lock: &Entity<IdleLock>,
    w: &Entity<Workspaces>,
    e: &Env,
) {
    let (lock, w) = (lock.clone(), w.clone());
    settle_until(cx, move |cx| {
        !lock.read_with(cx, |l, _| l.is_locked())
            && w.read_with(cx, |w, cx| {
                w.active_session().is_some_and(|s| !s.read(cx).is_lean()) && w.ready_to_lock(cx)
            })
    });
    wait_store_open(cx, e, "w1");
}

fn all_lean(w: &Entity<Workspaces>, cx: &mut TestAppContext) -> bool {
    w.read_with(cx, |w, cx| w.sessions().all(|s| s.read(cx).is_lean()))
}

#[gpui::test]
async fn idle_past_the_setting_locks_and_unlock_reopens_the_active_store(cx: &mut TestAppContext) {
    let e = env(cx, FIFTEEN);
    let w = workspaces(cx, &e);
    let keys = Arc::new(Forgets::default());
    let lock = install(cx, &w, keys.clone(), app_input_only());
    minutes(cx, 14);
    assert!(!lock.read_with(cx, |l, _| l.is_locked()));
    assert!(store_open(&e, "w1").await);
    minutes(cx, 1);
    wait_locked(cx, &lock, &w, &e);
    assert!(lock.read_with(cx, |l, _| l.is_locked()), "locked at 15 min");
    assert!(all_lean(&w, cx), "every workspace closed its store");
    assert!(
        !store_open(&e, "w1").await,
        "the active store is closed too"
    );
    // The key source answers from the background executor.
    settle_until(cx, |_| keys.forgets.load(Ordering::SeqCst) == 1);
    assert_eq!(
        keys.forgets.load(Ordering::SeqCst),
        1,
        "cached keys dropped"
    );
    assert!(
        !lock.read_with(cx, |l, _| l.timer_armed()),
        "no timer while locked"
    );

    lock.update(cx, |l, cx| l.unlock(cx));
    wait_unlocked(cx, &lock, &w, &e);
    assert!(!lock.read_with(cx, |l, _| l.is_locked()));
    assert!(
        store_open(&e, "w1").await,
        "unlock reopens the active store"
    );
    w.read_with(cx, |w, cx| {
        assert!(!w.session(&key("w1")).unwrap().read(cx).is_lean());
    });
    assert!(lock.read_with(cx, |l, _| l.timer_armed()), "armed again");
}

#[gpui::test]
async fn input_postpones_the_lock(cx: &mut TestAppContext) {
    let e = env(cx, FIFTEEN);
    let w = workspaces(cx, &e);
    let lock = install(cx, &w, Arc::new(Forgets::default()), app_input_only());
    minutes(cx, 10);
    lock.update(cx, |l, cx| l.note_input(cx));
    minutes(cx, 10);
    assert!(
        !lock.read_with(cx, |l, _| l.is_locked()),
        "10 min since the input"
    );
    minutes(cx, 5);
    assert!(lock.read_with(cx, |l, _| l.is_locked()));
}

#[gpui::test]
async fn the_os_idle_clock_wins_when_there_is_one(cx: &mut TestAppContext) {
    let e = env(cx, FIFTEEN);
    let w = workspaces(cx, &e);
    // The user is busy in another app: the OS saw input a minute ago.
    let lock = install(
        cx,
        &w,
        Arc::new(Forgets::default()),
        Rc::new(|_| Some(Duration::from_secs(60))),
    );
    minutes(cx, 30);
    assert!(!lock.read_with(cx, |l, _| l.is_locked()));
    assert!(lock.read_with(cx, |l, _| l.timer_armed()));
}

#[gpui::test]
async fn off_arms_no_timer_and_never_locks(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);
    let lock = install(cx, &w, Arc::new(Forgets::default()), app_input_only());
    assert!(!lock.read_with(cx, |l, _| l.timer_armed()));
    minutes(cx, 120);
    assert!(!lock.read_with(cx, |l, _| l.is_locked()));
    assert!(!all_lean(&w, cx));
}

#[gpui::test]
async fn turning_the_setting_on_arms_the_timer(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);
    let lock = install(cx, &w, Arc::new(Forgets::default()), app_input_only());
    std::fs::write(
        e.config.join("settings.json"),
        r#"{ "storage": { "lock_after_minutes": 5 } }"#,
    )
    .unwrap();
    cx.update(|cx| parla::handle_change(cx, settings::ChangedFile::Settings));
    let l = lock.clone();
    settle_until(cx, move |cx| l.read_with(cx, |l, _| l.timer_armed()));
    minutes(cx, 5);
    assert!(lock.read_with(cx, |l, _| l.is_locked()));
}

fn locked(lock: &Entity<IdleLock>, cx: &mut TestAppContext) -> bool {
    lock.read_with(cx, |l, _| l.is_locked())
}

#[gpui::test]
async fn a_keychain_failure_at_unlock_stays_locked_and_unlock_retries(cx: &mut TestAppContext) {
    let e = env(cx, FIFTEEN);
    let w = workspaces(cx, &e);
    let keys = Arc::new(Forgets::default());
    let lock = install(cx, &w, keys.clone(), app_input_only());
    minutes(cx, 15);
    wait_locked(cx, &lock, &w, &e);
    assert!(locked(&lock, cx));
    keys.fail.store(true, Ordering::SeqCst);
    lock.update(cx, |l, cx| l.unlock(cx));
    // The key read answers on the background executor: wait for its result.
    settle_until(cx, |cx| {
        lock.read_with(cx, |l, _| l.unlock_error().is_some())
    });
    assert!(locked(&lock, cx), "a failed key read keeps the lock");
    assert!(
        lock.read_with(cx, |l, _| l.unlock_error().is_some()),
        "an inline error"
    );
    w.read_with(cx, |w, cx| {
        for s in w.sessions() {
            assert_eq!(
                s.read(cx).store_health(),
                parla::session::StoreHealth::Ok,
                "no Unreadable, no Rebuild cache"
            );
        }
    });
    assert!(!store_open(&e, "w1").await);
    keys.fail.store(false, Ordering::SeqCst);
    lock.update(cx, |l, cx| l.unlock(cx));
    wait_unlocked(cx, &lock, &w, &e);
    assert!(!locked(&lock, cx), "Unlock retried");
    assert!(lock.read_with(cx, |l, _| l.unlock_error().is_none()));
    assert!(store_open(&e, "w1").await);
}

#[gpui::test]
async fn a_reopen_in_flight_when_the_lock_lands_ends_closed(cx: &mut TestAppContext) {
    let e = env(cx, FIFTEEN);
    let w = workspaces(cx, &e);
    let s2 = w.read_with(cx, |w, _| w.session(&key("w2")).unwrap());
    s2.update(cx, |s, cx| s.set_lean(true, cx));
    settle(cx);
    assert!(!store_open(&e, "w2").await);
    // The wake starts a reopen; the lock lands before it finishes.
    s2.update(cx, |s, cx| {
        s.set_lean(false, cx);
        s.set_lean(true, cx);
    });
    settle(cx);
    assert!(s2.read_with(cx, |s, _| s.is_lean()), "the lock wins");
    assert!(!store_open(&e, "w2").await, "and the store is closed again");
}

#[gpui::test]
async fn a_workspace_started_while_locked_opens_its_store_after_unlock(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let (registry, data, cache) = (e.registry.clone(), e.data.clone(), e.cache.clone());
    let w = cx.new(|cx| {
        let store: Arc<dyn store::Store> = Arc::new(store::MemoryStore::default());
        Workspaces::new(&registry, store, cx).with_dirs(data, cache)
    });
    w.update(cx, |w, cx| {
        w.set_locked(true, cx);
        w.start_all(cx);
    });
    // Every start lands while locked: deferred, no session and no store.
    wait_ready(cx, &w);
    assert!(!store_open(&e, "w1").await, "no store opens while locked");
    assert_eq!(w.read_with(cx, |w, _| w.sessions().count()), 0, "deferred");
    w.update(cx, |w, cx| w.set_locked(false, cx));
    // Unlock opens the deferred sessions and their stores on their threads.
    wait_ready(cx, &w);
    assert_eq!(w.read_with(cx, |w, _| w.sessions().count()), 3);
    for id in ["w1", "w2", "w3"] {
        assert!(store_open(&e, id).await, "{id} opened after unlock");
    }
}

#[gpui::test]
async fn the_lock_waits_for_the_workspaces_to_start(cx: &mut TestAppContext) {
    let e = env(cx, FIFTEEN);
    let (registry, data, cache) = (e.registry.clone(), e.data.clone(), e.cache.clone());
    let w = cx.new(|cx| {
        let store: Arc<dyn store::Store> = Arc::new(store::MemoryStore::default());
        Workspaces::new(&registry, store, cx).with_dirs(data, cache)
    });
    // The OS says the user has been away for two hours already.
    let lock = install(
        cx,
        &w,
        Arc::new(Forgets::default()),
        Rc::new(|_| Some(Duration::from_secs(7200))),
    );
    assert!(!locked(&lock, cx), "nothing locks before the start");
    w.update(cx, |w, cx| w.start_all(cx));
    settle(cx);
    assert!(!locked(&lock, cx), "nor while the stores load");
    // Every store opens on its own thread: wait for the loaded state, then
    // check that one really opened before moving the clock.
    wait_ready(cx, &w);
    assert!(store_open(&e, "w1").await);
    minutes(cx, 15);
    wait_locked(cx, &lock, &w, &e);
    assert!(locked(&lock, cx));
}

#[gpui::test]
async fn drafts_and_sidebar_survive_a_lock_and_unlock(cx: &mut TestAppContext) {
    use domain::DraftKey;
    use domain::test_support::cid;
    let e = env(cx, FIFTEEN);
    let w = workspaces(cx, &e);
    let lock = install(cx, &w, Arc::new(Forgets::default()), app_input_only());
    let s1 = w.read_with(cx, |w, _| w.session(&key("w1")).unwrap());
    s1.update(cx, |s, cx| {
        s.open(cid("D1"), cx);
        s.focus(cid("D1"), true, true, cx);
    });
    settle(cx);
    let dk = DraftKey {
        conversation: cid("D1"),
        thread: None,
    };
    s1.update(cx, |s, cx| s.set_draft(dk.clone(), "half typed", cx));
    settle(cx);
    minutes(cx, 15);
    wait_locked(cx, &lock, &w, &e);
    lock.update(cx, |l, cx| l.unlock(cx));
    // Unlock reopens on the store's thread, then reloads the open window.
    {
        let s1 = s1.clone();
        settle_until(cx, move |cx| {
            s1.read_with(cx, |s, _| {
                !s.is_lean()
                    && s.state()
                        .windows
                        .get(&cid("D1"))
                        .is_some_and(|w| w.messages().len() == 1)
            })
        });
    }
    s1.read_with(cx, |s, _| {
        assert!(!s.is_lean());
        assert!(s.state().conversations.contains_key(&cid("D1")), "sidebar");
        assert_eq!(s.selected(), Some(&cid("D1")));
        assert_eq!(
            s.draft(&dk).map(|d| d.text.to_string()).as_deref(),
            Some("half typed")
        );
        let win = s.state().windows.get(&cid("D1")).expect("window reloaded");
        assert_eq!(win.messages().len(), 1);
    });
}

#[test]
fn a_sleep_longer_than_the_setting_is_due_at_wake() {
    use parla::idle_lock::due_after_sleep;
    let m = |n: u64| Duration::from_secs(n * 60);
    assert!(due_after_sleep(m(0), m(30), m(15)), "slept past the limit");
    assert!(
        due_after_sleep(m(10), m(5), m(15)),
        "idle before plus the sleep"
    );
    assert!(!due_after_sleep(m(1), m(5), m(15)));
}

#[gpui::test]
async fn mouse_down_and_scroll_count_as_input(cx: &mut TestAppContext) {
    struct Area;
    impl gpui::Render for Area {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{InteractiveElement as _, ParentElement as _, Styled as _};
            parla::idle_lock::track_input(gpui::div().size_full())
                .debug_selector(|| "area".into())
                .child("x")
        }
    }
    let e = env(cx, FIFTEEN);
    let w = workspaces(cx, &e);
    let lock = install(cx, &w, Arc::new(Forgets::default()), app_input_only());
    let (_v, vcx) = cx.add_window_view(|_, _| Area);
    vcx.run_until_parked();
    vcx.executor().advance_clock(Duration::from_secs(10 * 60));
    let b = vcx.debug_bounds("area").expect("drawn");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    minutes(cx, 10);
    assert!(!locked(&lock, cx), "the click counted as input");
    minutes(cx, 5);
    assert!(locked(&lock, cx));
}

#[gpui::test]
async fn the_settings_window_is_covered_and_enter_unlocks(cx: &mut TestAppContext) {
    let e = env(cx, FIFTEEN);
    let w = workspaces(cx, &e);
    let lock = install(cx, &w, Arc::new(Forgets::default()), app_input_only());
    let path = e.config.join("settings.json");
    let (_v, vcx) = cx.add_window_view(move |_, _| {
        parla::settings_window::view_on(
            path,
            "1.2.3".into(),
            parla::settings_window::Page::Workspaces,
        )
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("settings-lock-off").is_some());
    minutes(cx, 15);
    assert!(locked(&lock, cx));
    let (_v2, vcx) = cx.add_window_view(|_, _| {
        parla::settings_window::view_on(
            std::path::PathBuf::from("/example/settings.json"),
            "1.2.3".into(),
            parla::settings_window::Page::Workspaces,
        )
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("lock-screen").is_some(), "covered");
    assert!(
        vcx.debug_bounds("settings-lock-off").is_none(),
        "nothing under it is drawn"
    );
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    settle(cx);
    assert!(!locked(&lock, cx), "Enter on the focused Unlock unlocks");
}

/// Never answers: the viewer shows the preview it was opened with.
struct NoFetch;

impl components::media::MediaFetcher for NoFetch {
    fn fetch(
        &self,
        _: &domain::ImageRef,
        _: u64,
    ) -> BoxFuture<'static, Result<Vec<u8>, components::media::MediaError>> {
        Box::pin(futures::future::pending())
    }
}

struct NoHost;

impl parla::code_viewer::CodeHost for NoHost {
    fn download(&self, _: &domain::FileRef, _: &mut gpui::App) {}
    fn fetcher(&self, _: &gpui::App) -> Option<Arc<dyn components::media::MediaFetcher>> {
        Some(Arc::new(NoFetch))
    }
}

#[gpui::test]
async fn the_lock_closes_the_code_viewer(cx: &mut TestAppContext) {
    let e = env(cx, FIFTEEN);
    let w = workspaces(cx, &e);
    let lock = install(cx, &w, Arc::new(Forgets::default()), app_input_only());
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_path_buf();
    cx.update(|cx| {
        cx.set_global(components::media::ImageCache::new(
            Arc::new(NoFetch),
            d,
            1 << 20,
        ));
        let file = domain::FileRef {
            id: domain::FileId(Arc::from("F1")),
            name: Arc::from("a.rs"),
            title: None,
            mime: Arc::from("text/plain"),
            size: 2,
            kind: domain::FileKind::Snippet,
            thumbs: Arc::from([]),
            download: None,
            preview: Some(Arc::from("a\n")),
            video: None,
            tombstone: false,
            lines: None,
            ext: domain::ProviderExt::none(),
        };
        let item = components::media_viewer::ViewerItem {
            file,
            message: domain::test_support::msg("C1", "1000.000001", "").id,
            sender: "Bob".into(),
        };
        let g = components::media_viewer::Gallery::new(vec![item], 0);
        parla::code_viewer::open(g, Rc::new(NoHost), cx);
    });
    settle(cx);
    let viewer = cx
        .update(|cx| parla::code_viewer::open_window(cx))
        .expect("viewer open");
    lock.update(cx, |l, cx| l.lock(cx));
    settle(cx);
    assert!(viewer.update(cx, |_, _, _| ()).is_err(), "closed on lock");
}
