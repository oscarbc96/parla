//! The application menu bar (spec 11a: the menu bar relocalises on a language
//! change). The menus are a pure function of the localizer, so tests read the
//! model; [`install`] sets it on the platform and rebuilds it on a locale switch.
//! GPUI has no list of open windows to show in the Window menu.

use gpui::{App, Menu, MenuItem, OsAction};

use crate::actions::{
    About, BringAllToFront, CheckForUpdates, CloseWindow, CommandPalette, Hide, HideOthers,
    Minimize, OpenDocs, OpenKeyboardShortcuts, OpenSettings, Quit, ReportIssue, ShowAll,
    ToggleMode, Zoom,
};

/// The product name; macOS replaces the app menu title with the bundle name.
pub const APP_NAME: &str = "Parla";
pub const DOCS_URL: &str = "https://oscarbc96.github.io/parla/docs/";
pub const KEYMAP_DOCS_URL: &str = "https://oscarbc96.github.io/parla/docs/keymap/";
pub const ISSUES_URL: &str = "https://github.com/oscarbc96/parla/issues";

/// The menus in menu-bar order. `mac` adds the items only macOS has (Hide,
/// Hide others, Show all); elsewhere GPUI shows no menu bar, so the model is
/// kept small rather than wrong.
pub fn build(l: &i18n::Localizer, mac: bool) -> Vec<Menu> {
    let title = |name: &str| crate::actions::title(l, name).unwrap_or_default();
    let item = |name: &str, action: &dyn Fn(String) -> MenuItem| action(title(name));

    let mut app = vec![
        item("app::About", &|n| MenuItem::action(n, About)),
        MenuItem::separator(),
        item("app::CheckForUpdates", &|n| {
            MenuItem::action(n, CheckForUpdates)
        }),
        MenuItem::separator(),
        MenuItem::action(t!(l, "menu-settings").into_owned(), OpenSettings),
        MenuItem::separator(),
    ];
    if mac {
        app.extend([
            item("app::Hide", &|n| MenuItem::action(n, Hide)),
            item("app::HideOthers", &|n| MenuItem::action(n, HideOthers)),
            item("app::ShowAll", &|n| MenuItem::action(n, ShowAll)),
            MenuItem::separator(),
        ]);
    }
    app.push(item("app::Quit", &|n| MenuItem::action(n, Quit)));

    use components::text_input as ti;
    let edit = vec![
        MenuItem::os_action(t!(l, "menu-undo").into_owned(), ti::Undo, OsAction::Undo),
        MenuItem::os_action(t!(l, "menu-redo").into_owned(), ti::Redo, OsAction::Redo),
        MenuItem::separator(),
        MenuItem::os_action(t!(l, "menu-cut").into_owned(), ti::Cut, OsAction::Cut),
        MenuItem::os_action(t!(l, "menu-copy").into_owned(), ti::Copy, OsAction::Copy),
        MenuItem::os_action(t!(l, "menu-paste").into_owned(), ti::Paste, OsAction::Paste),
        MenuItem::separator(),
        MenuItem::os_action(
            t!(l, "menu-select-all").into_owned(),
            ti::SelectAll,
            OsAction::SelectAll,
        ),
    ];

    let view = vec![
        item("theme::ToggleMode", &|n| MenuItem::action(n, ToggleMode)),
        MenuItem::separator(),
        item("app::CommandPalette", &|n| {
            MenuItem::action(n, CommandPalette)
        }),
        item("workspace::QuickSwitch", &|n| {
            MenuItem::action(n, components::quick_switcher::QuickSwitch)
        }),
        item("workspace::Search", &|n| {
            MenuItem::action(n, components::search_panel::Search)
        }),
        MenuItem::separator(),
        item("workspace::GoBack", &|n| {
            MenuItem::action(n, components::quick_switcher::GoBack)
        }),
        item("workspace::GoForward", &|n| {
            MenuItem::action(n, components::quick_switcher::GoForward)
        }),
    ];

    let window = vec![
        item("app::Minimize", &|n| MenuItem::action(n, Minimize)),
        item("app::Zoom", &|n| MenuItem::action(n, Zoom)),
        item("app::CloseWindow", &|n| MenuItem::action(n, CloseWindow)),
        MenuItem::separator(),
        item("app::BringAllToFront", &|n| {
            MenuItem::action(n, BringAllToFront)
        }),
    ];

    let help = vec![
        item("app::OpenDocs", &|n| MenuItem::action(n, OpenDocs)),
        item("app::OpenKeyboardShortcuts", &|n| {
            MenuItem::action(n, OpenKeyboardShortcuts)
        }),
        MenuItem::separator(),
        item("app::ReportIssue", &|n| MenuItem::action(n, ReportIssue)),
    ];

    vec![
        Menu::new(APP_NAME).items(app),
        Menu::new(t!(l, "menu-edit").into_owned()).items(edit),
        Menu::new(t!(l, "menu-view").into_owned()).items(view),
        Menu::new(t!(l, "menu-window").into_owned()).items(window),
        Menu::new(t!(l, "menu-help").into_owned()).items(help),
    ]
}

fn apply(cx: &mut App) {
    let l = i18n::gpui::localizer(cx);
    cx.set_menus(build(&l, cfg!(target_os = "macos")));
}

/// Sets the menu bar and rebuilds it whenever the language changes.
pub fn install(cx: &mut App) {
    apply(cx);
    i18n::gpui::on_relocalize(cx, apply);
}

/// What Close Window does to the window in front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    Close,
    HideApp,
}

/// A secondary window (settings, about, pop-outs, sign-in) always closes. The
/// main window hides the app while the tray icon can bring it back, as its
/// close button does, and closes otherwise.
pub fn close_plan(is_main: bool, tray_hides: bool) -> CloseAction {
    if is_main && tray_hides {
        CloseAction::HideApp
    } else {
        CloseAction::Close
    }
}

/// Close Window: the focused window, by [`close_plan`].
pub fn close_window(cx: &mut App) {
    let Some(handle) = cx.active_window() else {
        return;
    };
    let is_main = handle.downcast::<crate::root_view::RootView>().is_some();
    match close_plan(is_main, crate::tray::Tray::hides_on_close(cx)) {
        CloseAction::HideApp => crate::tray::Tray::hide_for_close(cx),
        // Deferred: a menu action runs while the focused window is being
        // updated, and that window cannot be updated again until it returns.
        CloseAction::Close => cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }),
    }
}

/// Runs `f` on the window in front once the action has returned (see
/// [`close_window`]).
fn active(cx: &mut App, f: impl FnOnce(&mut gpui::Window, &mut App) + 'static) {
    if let Some(handle) = cx.active_window() {
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| f(window, cx));
        });
    }
}

/// Handlers of the window and help actions (the others live next to the code
/// they drive).
pub fn register_actions(cx: &mut App) {
    // The platform implements these on macOS only (GPUI panics elsewhere).
    #[cfg(target_os = "macos")]
    {
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    }
    cx.on_action(|_: &Minimize, cx| active(cx, |w, _| w.minimize_window()));
    cx.on_action(|_: &Zoom, cx| active(cx, |w, _| w.zoom_window()));
    cx.on_action(|_: &CloseWindow, cx| close_window(cx));
    cx.on_action(|_: &BringAllToFront, cx| {
        cx.defer(|cx| {
            for handle in cx.windows() {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
            cx.activate(true);
        });
    });
    cx.on_action(|_: &OpenDocs, cx| cx.open_url(DOCS_URL));
    // The main window handles it itself; from another window (Settings,
    // About) the sheet opens on the main window, else the keymap docs.
    cx.on_action(|_: &OpenKeyboardShortcuts, cx| {
        let main = cx
            .windows()
            .into_iter()
            .find_map(|w| w.downcast::<crate::root_view::RootView>());
        let shown = main.is_some_and(|h| {
            h.update(cx, |v, window, cx| {
                window.activate_window();
                v.open_shortcuts_sheet(window, cx);
            })
            .is_ok()
        });
        if !shown {
            cx.open_url(KEYMAP_DOCS_URL);
        }
    });
    cx.on_action(|_: &ReportIssue, cx| cx.open_url(ISSUES_URL));
}
