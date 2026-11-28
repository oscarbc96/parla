//! The `parla` desktop app: opens the window, wires the providers, stores and
//! settings, and runs the GPUI event loop.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Bounds, WindowHandle, prelude::*, size};
use gpui_platform::application;
use parla::providers::{AppCredentials, CachingCredentials, KeychainCredentials, registry};
use parla::root_view::{RootView, SqliteHandle};
use parla::store_keys::GpuiKeySource;
use parla::workspaces::{Workspaces, WorkspacesHandle};
use settings::{ConfigPaths, SettingsStore};
use store::Store;
use store_sqlite::{CachingKeySource, KeySource, SqliteConfig, SqliteStore};

#[cfg(feature = "fixture")]
#[global_allocator]
static ALLOC: parla::perf::CountingAlloc = parla::perf::CountingAlloc;

fn main() {
    if std::env::args_os().any(|a| a == "--print-actions") {
        print!("{}", parla::actions::print_actions_json());
        return;
    }
    // `parla __plugin-precompile <wasm> <out>` compiles a plugin component for
    // the plugin engine in this short-lived process (the app's precompile
    // cache runs it). Before the crash hook, settings, store and GPUI.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|a| a == plugin_host::cache::PRECOMPILE_SUBCOMMAND)
    {
        std::process::exit(precompile_child());
    }
    // `parla mcp` is the stdio shim: it runs before any GPUI, settings or log init.
    if std::env::args_os().nth(1).is_some_and(|a| a == "mcp") {
        std::process::exit(i32::from(mcp::shim::main_code()));
    }
    // `parla call-window` is the call helper: before settings, store, providers
    // or the single-instance check; it exits with its own code.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|a| a == "call-window")
    {
        std::process::exit(
            if calls::helper::run_helper() == std::process::ExitCode::SUCCESS {
                0
            } else {
                2
            },
        );
    }
    let crashes_dir = parla::crash::dir(&settings::paths::data_dir());
    parla::crash::install_panic_hook(crashes_dir, parla::providers::redaction_rules());
    #[cfg(feature = "fixture")]
    parla::perf::mark_process_start();
    #[cfg(feature = "fixture")]
    let perf = {
        let args: Vec<String> = std::env::args().collect();
        let opts = parla::perf::driver::Options::from_args(&args);
        if let Some(code) = opts
            .as_ref()
            .and_then(parla::perf::driver::parent_if_needed)
        {
            std::process::exit(code);
        }
        if opts.is_some() {
            parla::perf::driver::prepare_env(opts.as_ref().map_or(1, |o| o.workspaces));
        }
        opts
    };
    // One instance: a running app owns the endpoint; ask it to show its window.
    let mode = parla::instance::LaunchMode::from_args(std::env::args_os().skip(1));
    if parla::instance::handoff(&settings::paths::mcp_socket_path()) {
        return;
    }
    let app = application();
    // A Dock click shows the hidden window, or opens one.
    app.on_reopen(parla::instance::Instance::reopen);
    let shared_window: Rc<RefCell<Option<WindowHandle<RootView>>>> = Rc::default();
    type ShowFn = Rc<dyn Fn(&mut App)>;
    let shared_show: Rc<RefCell<Option<ShowFn>>> = Rc::default();
    let shared_async: Arc<std::sync::Mutex<Option<gpui::AsyncApp>>> = Arc::default();
    {
        let window = shared_window.clone();
        let show = shared_show.clone();
        let async_app = shared_async.clone();
        app.on_open_urls(move |urls| {
            let Some(app) = async_app.lock().unwrap().clone() else {
                return;
            };
            let window = window.clone();
            let show = show.clone();
            app.update(|cx| {
                for url in urls {
                    if let Some(show) = show.borrow().as_ref() {
                        parla::links::handle_open_url(url, show.as_ref(), *window.borrow(), cx);
                    }
                }
            });
        });
    }
    app.run(move |cx: &mut App| {
        *shared_async.lock().unwrap() = Some(cx.to_async());
        parla::init(cx, ConfigPaths::discover());
        // A hidden launch with no menu-bar icon has no way back: open a window.
        let mode = mode.with_tray(cx.global::<SettingsStore>().current().tray);
        parla::log::init(&settings::paths::data_dir());
        parla::log::set_level(cx.global::<SettingsStore>().current().log.level);
        // Before any window: "Inter" resolves to the bundled design weights.
        parla::fonts::install(cx);
        parla::crash::set_mode(cx.global::<SettingsStore>().current().crash_reports);
        parla::crash::show_pending_notice(&parla::crash::dir(&settings::paths::data_dir()), cx);
        parla::crash::install_actions(cx);
        parla::log::info(&format!(
            "started, log file {}",
            parla::log::path_in(&settings::paths::data_dir()).display()
        ));
        parla::login_item::LoginItems::install_real(cx);
        cx.set_app_identity("com.justparla.parla", "Parla");
        parla::notifications::Notifications::new(parla::notifications::notifier_for(
            parla::notifications::can_notify(),
        ))
        .with_sound(std::rc::Rc::new(parla::notifications::SystemSound))
        .install(cx);
        parla::ringer::Ringer::new(std::rc::Rc::new(parla::ringer::SystemPlayer), cx).install(cx);
        // The menu-bar icon: created here, on the main thread after GPUI starts.
        // Both a shim and a login launch start with no window, so both hide the
        // Dock icon until the first window shows.
        let headless = mode == parla::instance::LaunchMode::Headless;
        let hidden = mode == parla::instance::LaunchMode::Hidden;
        if headless || hidden {
            parla::platform::set_dock_visible(false);
        }
        parla::tray::install_system(cx);
        if hidden {
            // A login launch is a window hidden behind the icon, not a shim:
            // keep the tray's Dock state in step until the window first shows.
            parla::tray::Tray::set_window_hidden(true, cx);
        }
        let cache = settings::paths::cache_dir();
        cx.background_executor()
            .spawn(async move { parla::media::prune_at_startup(&cache) })
            .detach();
        let rt = parla::runtime::init_runtime(cx);
        // The perf fixture never touches the keychain, even in a release build.
        #[cfg(feature = "fixture")]
        let backend = if std::env::var_os("PARLA_FIXTURE").is_some() {
            parla::dev_credentials::Backend::DevFile
        } else {
            parla::dev_credentials::current()
        };
        #[cfg(not(feature = "fixture"))]
        let backend = parla::dev_credentials::current();
        let dev_file = parla::dev_credentials::DevFile::new(
            settings::paths::data_dir().join(parla::dev_credentials::FILE_NAME),
        );
        if backend == parla::dev_credentials::Backend::DevFile {
            parla::log::stderr_line(&parla::dev_credentials::startup_notice(dev_file.path()));
        }
        let creds = CachingCredentials::new(parla::dev_credentials::choose(
            backend,
            || AppCredentials::Keychain(KeychainCredentials::new(cx)),
            || AppCredentials::Dev(parla::providers::DevCredentials(dev_file.clone())),
        ));
        let registry = registry(rt.clone(), creds);
        let cache_mb = cx.global::<SettingsStore>().current().storage.cache_mb;
        let store_dir = settings::paths::data_dir().join("store");
        // Shared with the idle lock, which empties its cache when it locks.
        let store_keys: Arc<dyn KeySource> =
            Arc::new(CachingKeySource::new(parla::dev_credentials::choose::<
                Arc<dyn KeySource>,
            >(
                backend,
                || GpuiKeySource::new(cx),
                || Arc::new(parla::dev_credentials::DevKeys(dev_file.clone())),
            )));
        let sqlite = SqliteStore::start(
            SqliteConfig {
                dir: store_dir,
                cache_mb,
            },
            store_keys.clone(),
        );
        cx.set_global(SqliteHandle(sqlite.clone()));
        cx.set_global(parla::removal::AppDirs::discover());
        // Close the databases and join the store threads before exit: SQLCipher's
        // `atexit` hook frees its crypto state under any thread still running.
        let (quit_store, executor) = (sqlite.clone(), cx.background_executor().clone());
        cx.on_app_quit(move |_| {
            let (s, ex) = (quit_store.clone(), executor.clone());
            async move { ex.spawn(async move { s.shutdown() }).await }
        })
        .detach();
        let store: Arc<dyn Store> = Arc::new(sqlite);
        parla::store_banner::warn_if_unencrypted_later(cx, parla::store_banner::disk_encrypted);
        let workspaces = cx.new(|cx| Workspaces::new(&registry, store, cx));
        cx.set_global(WorkspacesHandle(workspaces.clone()));
        let idle_lock = parla::idle_lock::install(
            cx,
            workspaces.clone(),
            Some(store_keys),
            parla::idle_lock::system_source(),
        );
        parla::idle_lock::watch_power(&idle_lock, cx);
        let window = shared_window.clone();
        let show_impl = {
            let (registry, workspaces, window) =
                (registry.clone(), workspaces.clone(), window.clone());
            let dock_pending = Cell::new(headless || hidden);
            move |cx: &mut App| {
                if dock_pending.replace(false) {
                    // The shim or login launch hid the icon: bring it back as
                    // the setting wants it with a window open.
                    parla::platform::set_dock_visible(parla::tray::dock_for_window(cx));
                }
                parla::tray::Tray::set_window_hidden(false, cx);
                let existing = *window.borrow();
                if let Some(h) = existing
                    && h.update(cx, |_, w, _| w.activate_window()).is_ok()
                {
                    cx.activate(true);
                    return;
                }
                let state_path = parla::window_state::main_path_in(&settings::paths::data_dir());
                let restore = cx
                    .try_global::<settings::SettingsStore>()
                    .is_none_or(|s| s.current().windows.restore);
                let min = size(
                    themes::metrics::LAYOUT_WINDOW_MIN_WIDTH,
                    themes::metrics::LAYOUT_WINDOW_MIN_HEIGHT,
                );
                // Bounds are relative to a display: reopen on the one the
                // window was saved on (else the primary) and clamp to it only.
                let platform_displays = cx.displays();
                let displays: Vec<parla::window_state::DisplayInfo> = platform_displays
                    .iter()
                    .map(|d| parla::window_state::DisplayInfo {
                        uuid: d.uuid().ok().map(|u| u.to_string()),
                        visible: d.visible_bounds(),
                    })
                    .collect();
                let primary = cx
                    .primary_display()
                    .and_then(|p| platform_displays.iter().position(|d| d.id() == p.id()))
                    .unwrap_or(0);
                let restored = restore
                    .then(|| parla::window_state::load_main(&state_path))
                    .flatten()
                    .and_then(|w| {
                        let (i, d) = parla::window_state::restore_display(
                            w.display.as_deref(),
                            &displays,
                            primary,
                        )?;
                        let b = parla::window_state::clamp_to_display(w.bounds, d.visible, min)?;
                        Some((b, platform_displays[i].id()))
                    });
                let (bounds, display) = match restored {
                    Some((b, id)) => (b, Some(id)),
                    None => (
                        Bounds::centered(
                            None,
                            size(
                                themes::metrics::LAYOUT_WINDOW_WIDTH,
                                themes::metrics::LAYOUT_WINDOW_HEIGHT,
                            ),
                            cx,
                        ),
                        None,
                    ),
                };
                let (registry, workspaces) = (registry.clone(), workspaces.clone());
                let opened = cx
                    .open_window(
                        parla::window_options::main_on(bounds, display),
                        |window, cx| {
                            cx.new(|cx| {
                                let view =
                                    RootView::with_workspaces(&registry, workspaces, window, cx);
                                if restore {
                                    parla::window_state::track_main(state_path, window, cx);
                                }
                                view
                            })
                        },
                    )
                    .inspect_err(|e| parla::log::error(&format!("failed to open window: {e}")))
                    .ok();
                let Some(opened) = opened else {
                    return;
                };
                *window.borrow_mut() = Some(opened);
                cx.activate(true);
            }
        };
        let show: Rc<dyn Fn(&mut App)> = Rc::new(show_impl);
        *shared_show.borrow_mut() = Some(show.clone());
        // Single instance: the endpoint also serves `show` with MCP off. A lost
        // race (another instance bound first) hands off and exits.
        let socket = settings::paths::mcp_socket_path();
        let hooks = parla::instance::Hooks {
            show: show.clone(),
            quit: Rc::new(|cx| cx.quit()),
        };
        match parla::instance::Instance::install(cx, &rt, &socket, mode, hooks) {
            Ok(()) => {}
            Err(mcp::ipc::IpcError::InUse) => {
                parla::instance::handoff(&socket);
                cx.quit();
                return;
            }
            Err(e) => {
                parla::log::error(&format!("single-instance endpoint unavailable: {e}"));
                cx.set_global(parla::instance::ShowFallback(show.clone()));
                parla::mcp_host::report_bind_error(cx, &e);
            }
        }
        parla::call_manager::CallManager::install(
            Box::new(calls::controller::ExeSpawner::new(rt.clone())),
            cx,
        );
        parla::mcp_host::McpHost::install(cx, rt.clone());
        parla::updates::install(cx);
        let catalogs = registry.catalogs();
        {
            let catalogs = catalogs.clone();
            cx.on_action(move |_: &parla::actions::About, cx| {
                parla::about::open(cx, catalogs.clone());
            });
        }
        parla::menus::install(cx);
        if headless || hidden {
            workspaces.update(cx, |w, cx| mode.start_workspaces(w, cx));
            return;
        }
        show(cx);
        cx.defer(|cx| {
            parla::popout::restore_default(cx);
        });
        #[cfg(feature = "fixture")]
        if let (Some(opts), Some(provider), Some(window)) = (
            perf.clone(),
            parla::providers::fixture_provider(),
            *window.borrow(),
        ) {
            parla::perf::driver::spawn(opts, window, provider, cx);
        }
    });
}

/// Body of `parla __plugin-precompile <wasm> <out>`: 0 when the `.cwasm` was
/// written, 1 with a message on stderr otherwise.
fn precompile_child() -> i32 {
    let mut args = std::env::args_os().skip(2);
    let (Some(wasm), Some(out), None) = (args.next(), args.next(), args.next()) else {
        eprintln!(
            "usage: parla {} <wasm> <out>",
            plugin_host::cache::PRECOMPILE_SUBCOMMAND
        );
        return 1;
    };
    match plugin_host::cache::precompile_child(
        std::path::Path::new(&wasm),
        std::path::Path::new(&out),
    ) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
