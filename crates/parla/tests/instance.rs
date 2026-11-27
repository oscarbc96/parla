//! Single instance and headless mode: a second launch sends `show`
//! to the running app, headless opens no window and runs lean, and a headless
//! app with no MCP client quits by itself.

mod support;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext};
use mcp::ipc::IpcError;
use parla::instance::{Hooks, Instance, LaunchMode, handoff};
use parla::workspaces::WorkspacesHandle;
use support::*;

/// A short temp dir: socket paths must fit `sun_path`.
fn run_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("t")
        .tempdir_in("/tmp")
        .unwrap()
}

fn sock(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("run").join("mcp.sock")
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

struct Counts {
    shown: Rc<Cell<u32>>,
    quit: Rc<Cell<u32>>,
}

fn hooks() -> (Hooks, Counts) {
    let (shown, quit) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (s, q) = (shown.clone(), quit.clone());
    let hooks = Hooks {
        show: Rc::new(move |_| s.set(s.get() + 1)),
        quit: Rc::new(move |_| q.set(q.get() + 1)),
    };
    (hooks, Counts { shown, quit })
}

fn install(
    cx: &mut TestAppContext,
    rt: &tokio::runtime::Runtime,
    path: &std::path::Path,
    mode: LaunchMode,
) -> Result<Counts, IpcError> {
    let (hooks, counts) = hooks();
    let handle = rt.handle().clone();
    let path = path.to_path_buf();
    cx.update(|cx| Instance::install(cx, &handle, &path, mode, hooks))?;
    Ok(counts)
}

fn minutes(cx: &mut TestAppContext, secs: u64) {
    cx.executor().advance_clock(Duration::from_secs(secs));
    settle(cx);
}

#[gpui::test]
async fn second_launch_sends_show_and_exits(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let path = sock(&dir);
    assert!(!handoff(&path), "nothing is running yet");
    let counts = install(cx, &rt, &path, LaunchMode::Window).unwrap();
    // A second instance connects from another thread, as another process would.
    let p = path.clone();
    let sent = std::thread::spawn(move || handoff(&p)).join().unwrap();
    assert!(sent, "the running instance answered the show request");
    settle(cx);
    assert_eq!(counts.shown.get(), 1, "the running window is shown");
    // A second bind on the same endpoint is refused: that instance hands off too.
    let (hooks, _) = hooks();
    let h = rt.handle().clone();
    let err = cx
        .update(|cx| Instance::install(cx, &h, &path, LaunchMode::Window, hooks))
        .unwrap_err();
    assert!(matches!(err, IpcError::InUse), "{err:?}");
}

#[gpui::test]
async fn headless_opens_no_window_and_uses_lean_mode(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces_in_mode(cx, &e, LaunchMode::Headless);
    w.read_with(cx, |w, cx| {
        assert_eq!(w.sessions().count(), 3);
        for s in w.sessions() {
            assert!(s.read(cx).is_lean(), "every workspace starts lean");
        }
        assert_eq!(w.lean_timers(), 0, "no lean timer is armed in headless");
    });
    assert!(cx.windows().is_empty(), "no window is opened");
    assert!(!store_open(&e, "w1").await, "lean closes the store");
}

#[gpui::test]
async fn show_turns_headless_into_window(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces_in_mode(cx, &e, LaunchMode::Headless);
    let (dir, rt) = (run_dir(), runtime());
    let path = sock(&dir);
    let counts = install(cx, &rt, &path, LaunchMode::Headless).unwrap();
    assert!(cx.update(|cx| Instance::is_headless(cx)));
    let p = path.clone();
    assert!(std::thread::spawn(move || handoff(&p)).join().unwrap());
    // The runtime thread delivers "show" to the app; wait for its effects.
    settle_until(cx, |cx| {
        counts.shown.get() == 1
            && w.read_with(cx, |w, cx| {
                w.active_session().is_some_and(|s| !s.read(cx).is_lean())
            })
    });
    assert_eq!(counts.shown.get(), 1);
    assert!(
        !cx.update(|cx| Instance::is_headless(cx)),
        "headless is left for good"
    );
    // Idle quit never fires after a window was shown.
    minutes(cx, 3600);
    assert_eq!(counts.quit.get(), 0);
}

#[gpui::test]
async fn headless_quits_after_idle_without_clients(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let counts = install(cx, &rt, &sock(&dir), LaunchMode::Headless).unwrap();
    minutes(cx, 299);
    assert_eq!(counts.quit.get(), 0);
    minutes(cx, 2);
    assert_eq!(counts.quit.get(), 1);
}

#[gpui::test]
async fn headless_stays_while_client_connected(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let counts = install(cx, &rt, &sock(&dir), LaunchMode::Headless).unwrap();
    minutes(cx, 240);
    cx.update(Instance::mcp_connected);
    minutes(cx, 3600);
    assert_eq!(counts.quit.get(), 0, "a connected client keeps it alive");
    cx.update(Instance::mcp_disconnected);
    minutes(cx, 299);
    assert_eq!(
        counts.quit.get(),
        0,
        "the 5 minutes count from the last close"
    );
    minutes(cx, 2);
    assert_eq!(counts.quit.get(), 1);
}

#[gpui::test]
async fn window_mode_never_arms_the_idle_timer(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let counts = install(cx, &rt, &sock(&dir), LaunchMode::Window).unwrap();
    minutes(cx, 3600);
    assert_eq!(counts.quit.get(), 0);
}

#[gpui::test]
async fn ipc_binds_for_show_when_mcp_off(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let path = sock(&dir);
    install(cx, &rt, &path, LaunchMode::Window).unwrap();
    let p = path.clone();
    let off = std::thread::spawn(move || {
        let rt = runtime();
        rt.block_on(mcp::ipc::connect(&p, "mcp")).err()
    })
    .join()
    .unwrap();
    assert!(matches!(off, Some(IpcError::McpOff)), "{off:?}");
    let p = path.clone();
    assert!(std::thread::spawn(move || handoff(&p)).join().unwrap());
}

#[test]
fn launch_mode_parses_the_headless_flag() {
    assert_eq!(LaunchMode::from_args(["parla"]), LaunchMode::Window);
    assert_eq!(LaunchMode::from_args(["--headless"]), LaunchMode::Headless);
    assert_eq!(
        LaunchMode::from_args(["--perf-scenario", "idle"]),
        LaunchMode::Window
    );
}

#[test]
fn launch_mode_parses_the_hidden_flag() {
    assert_eq!(
        LaunchMode::from_args(["parla", "--hidden"]),
        LaunchMode::Hidden
    );
    assert_eq!(LaunchMode::from_args(["--hidden"]), LaunchMode::Hidden);
    // A shim launch stays MCP-headless, even if both flags are present.
    assert_eq!(
        LaunchMode::from_args(["--hidden", "--headless"]),
        LaunchMode::Headless
    );
}

#[test]
fn hidden_launch_without_tray_starts_with_a_window() {
    // Hidden has no way back without the menu-bar icon: open a window instead.
    assert_eq!(LaunchMode::Hidden.with_tray(false), LaunchMode::Window);
    assert_eq!(LaunchMode::Hidden.with_tray(true), LaunchMode::Hidden);
    // Other launch modes do not depend on the tray setting.
    assert_eq!(LaunchMode::Window.with_tray(false), LaunchMode::Window);
    assert_eq!(LaunchMode::Headless.with_tray(false), LaunchMode::Headless);
}

#[gpui::test]
async fn hidden_mode_never_quits_after_an_hour(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let counts = install(cx, &rt, &sock(&dir), LaunchMode::Hidden).unwrap();
    assert!(
        !cx.update(|cx| Instance::is_headless(cx)),
        "a login launch is not MCP-headless"
    );
    minutes(cx, 3600);
    assert_eq!(
        counts.quit.get(),
        0,
        "a login launch stays up with no MCP client"
    );
}

#[gpui::test]
async fn tray_open_shows_the_hidden_window(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let counts = install(cx, &rt, &sock(&dir), LaunchMode::Hidden).unwrap();
    cx.update(|cx| parla::tray::Tray::dispatch(parla::tray::TrayAction::Open, cx));
    assert_eq!(counts.shown.get(), 1, "the tray opens the window");
    minutes(cx, 3600);
    assert_eq!(counts.quit.get(), 0);
}

#[gpui::test]
async fn second_launch_shows_the_hidden_window(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let path = sock(&dir);
    let counts = install(cx, &rt, &path, LaunchMode::Hidden).unwrap();
    let p = path.clone();
    assert!(std::thread::spawn(move || handoff(&p)).join().unwrap());
    settle_until(cx, |_cx| counts.shown.get() == 1);
    minutes(cx, 3600);
    assert_eq!(counts.quit.get(), 0);
}

fn workspaces_in_mode(
    cx: &mut TestAppContext,
    e: &Env,
    mode: LaunchMode,
) -> gpui::Entity<parla::workspaces::Workspaces> {
    let (registry, data, cache) = (e.registry.clone(), e.data.clone(), e.cache.clone());
    let w = cx.new(|cx| {
        let store: std::sync::Arc<dyn store::Store> =
            std::sync::Arc::new(store::MemoryStore::default());
        let mut w = parla::workspaces::Workspaces::new(&registry, store, cx).with_dirs(data, cache);
        mode.start_workspaces(&mut w, cx);
        w
    });
    let h = w.clone();
    cx.update(|cx| cx.set_global(WorkspacesHandle(h)));
    settle(cx);
    w
}

#[gpui::test]
async fn hidden_workspaces_behave_like_a_hidden_window(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces_in_mode(cx, &e, LaunchMode::Hidden);
    w.read_with(cx, |w, cx| {
        assert_eq!(
            w.sessions().count(),
            3,
            "sync and notifications keep running"
        );
        for s in w.sessions() {
            assert!(!s.read(cx).is_lean(), "no workspace goes lean at once");
        }
        assert_eq!(
            w.lean_timers(),
            3,
            "each workspace arms a hidden-window timer"
        );
    });
}

#[gpui::test]
async fn tray_open_leaves_headless_and_cancels_the_idle_quit(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let counts = install(cx, &rt, &sock(&dir), LaunchMode::Headless).unwrap();
    minutes(cx, 200);
    cx.update(|cx| parla::tray::Tray::dispatch(parla::tray::TrayAction::Open, cx));
    assert_eq!(counts.shown.get(), 1, "the tray asks for the window");
    assert!(!cx.update(|cx| Instance::is_headless(cx)));
    minutes(cx, 3600);
    assert_eq!(counts.quit.get(), 0, "the idle quit was cancelled");
}

#[gpui::test]
async fn dock_reopen_shows_the_window(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let counts = install(cx, &rt, &sock(&dir), LaunchMode::Window).unwrap();
    cx.update(Instance::reopen);
    assert_eq!(counts.shown.get(), 1);
}

#[gpui::test]
async fn installing_the_tray_does_not_cancel_the_idle_quit(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let (dir, rt) = (run_dir(), runtime());
    let counts = install(cx, &rt, &sock(&dir), LaunchMode::Headless).unwrap();
    struct Port;
    impl parla::tray::TrayPort for Port {
        fn show(&self, _: &parla::tray::TrayView, _: &mut gpui::App) {}
        fn remove(&self) {}
    }
    cx.update(|cx| parla::tray::Tray::new(Rc::new(Port)).install(cx));
    settle(cx);
    assert!(cx.update(|cx| Instance::is_headless(cx)));
    minutes(cx, 301);
    assert_eq!(counts.quit.get(), 1, "the tray does not count as shown");
    assert_eq!(counts.shown.get(), 0);
}

#[gpui::test]
async fn tray_open_without_an_instance_uses_the_fallback(cx: &mut TestAppContext) {
    let _e = env(cx, "{}");
    let shown = Rc::new(Cell::new(0));
    let s = shown.clone();
    cx.update(|cx| {
        cx.set_global(parla::instance::ShowFallback(Rc::new(move |_| {
            s.set(s.get() + 1)
        })));
        parla::tray::Tray::dispatch(parla::tray::TrayAction::Open, cx);
    });
    assert_eq!(shown.get(), 1);
}
