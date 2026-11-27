//! The macOS menu bar: structure, action wiring, translations and where Close
//! Window goes.

use gpui::{
    Context, IntoElement, OsAction, OwnedMenu, OwnedMenuItem, Render, TestAppContext, Window, div,
};
use i18n::{LocalePref, LocaleSet, Localizer};
use parla::actions::CloseWindow;
use parla::menus::{CloseAction, build, close_plan};

fn localizer(code: &str) -> Localizer {
    let chain = i18n::chain(
        &LocalePref::Code(code.parse().unwrap()),
        &[],
        LocaleSet::embedded(),
    );
    Localizer::new(chain, &[components::catalog(), parla::catalog()]).unwrap()
}

fn menus(code: &str, mac: bool) -> Vec<OwnedMenu> {
    build(&localizer(code), mac)
        .into_iter()
        .map(|m| m.owned())
        .collect()
}

/// `(label, action name, os action)` of every action item, separators skipped.
fn os_name(os: Option<OsAction>) -> Option<&'static str> {
    os.map(|o| match o {
        OsAction::Cut => "cut",
        OsAction::Copy => "copy",
        OsAction::Paste => "paste",
        OsAction::SelectAll => "select-all",
        OsAction::Undo => "undo",
        OsAction::Redo => "redo",
    })
}

fn actions(menu: &OwnedMenu) -> Vec<(String, String, Option<&'static str>)> {
    menu.items
        .iter()
        .filter_map(|i| match i {
            OwnedMenuItem::Action {
                name,
                action,
                os_action,
                ..
            } => Some((name.clone(), action.name().to_string(), os_name(*os_action))),
            _ => None,
        })
        .collect()
}

fn names(menu: &OwnedMenu) -> Vec<String> {
    actions(menu).into_iter().map(|a| a.1).collect()
}

fn menu<'a>(all: &'a [OwnedMenu], name: &str) -> &'a OwnedMenu {
    all.iter().find(|m| m.name == name).unwrap_or_else(|| {
        panic!(
            "no {name} menu in {:?}",
            all.iter().map(|m| &m.name).collect::<Vec<_>>()
        )
    })
}

#[test]
fn top_level_menus_in_order() {
    let all = menus("en", true);
    let titles: Vec<_> = all.iter().map(|m| m.name.to_string()).collect();
    assert_eq!(titles, ["Parla", "Edit", "View", "Window", "Help"]);
}

#[test]
fn app_menu_wiring() {
    let all = menus("en", true);
    assert_eq!(
        names(menu(&all, "Parla")),
        [
            "app::About",
            "app::CheckForUpdates",
            "app::OpenSettings",
            "app::Hide",
            "app::HideOthers",
            "app::ShowAll",
            "app::Quit",
        ]
    );
    let labels: Vec<_> = actions(menu(&all, "Parla"))
        .into_iter()
        .map(|a| a.0)
        .collect();
    assert_eq!(labels[2], "Settings\u{2026}");
    assert_eq!(labels[3], "Hide Parla");
}

#[test]
fn edit_menu_uses_os_actions() {
    let all = menus("en", true);
    let got = actions(menu(&all, "Edit"));
    let want = [
        ("text_input::Undo", "undo"),
        ("text_input::Redo", "redo"),
        ("text_input::Cut", "cut"),
        ("text_input::Copy", "copy"),
        ("text_input::Paste", "paste"),
        ("text_input::SelectAll", "select-all"),
    ];
    assert_eq!(got.len(), want.len());
    for ((_, name, os), (want_name, want_os)) in got.iter().zip(want) {
        assert_eq!(name, want_name);
        assert_eq!(*os, Some(want_os));
    }
}

#[test]
fn view_window_and_help_wiring() {
    let all = menus("en", true);
    let view = names(menu(&all, "View"));
    for a in [
        "theme::ToggleMode",
        "app::CommandPalette",
        "workspace::QuickSwitch",
    ] {
        assert!(view.contains(&a.to_string()), "{view:?}");
    }
    assert_eq!(
        names(menu(&all, "Window")),
        [
            "app::Minimize",
            "app::Zoom",
            "app::CloseWindow",
            "app::BringAllToFront"
        ]
    );
    assert_eq!(
        names(menu(&all, "Help")),
        [
            "app::OpenDocs",
            "app::OpenKeyboardShortcuts",
            "app::ReportIssue"
        ]
    );
}

#[test]
fn every_action_is_registered() {
    for mac in [true, false] {
        for m in menus("en", mac) {
            for name in names(&m) {
                assert!(parla::actions::is_known(&name), "{name} is not in KNOWN");
            }
        }
    }
}

#[test]
fn other_platforms_have_no_hide_items() {
    let all = menus("en", false);
    let app = names(menu(&all, "Parla"));
    for gone in ["app::Hide", "app::HideOthers", "app::ShowAll"] {
        assert!(!app.contains(&gone.to_string()), "{app:?}");
    }
    assert!(app.contains(&"app::Quit".to_string()));
}

#[test]
fn labels_are_translated_in_every_locale() {
    let en = menus("en", true);
    for locale in ["es", "ca"] {
        let other = menus(locale, true);
        for (a, b) in en.iter().zip(&other) {
            assert!(!b.name.is_empty());
            for (x, y) in actions(a).iter().zip(actions(b).iter()) {
                assert!(
                    !y.0.is_empty() && !y.0.starts_with("menu-"),
                    "{locale}: {y:?}"
                );
                assert_eq!(x.1, y.1, "same wiring in {locale}");
            }
        }
    }
    let es = menus("es", true);
    assert_eq!(es[1].name, "Edición");
    let labels: Vec<_> = actions(menu(&es, "Parla"))
        .into_iter()
        .map(|a| a.0)
        .collect();
    assert_eq!(labels[3], "Ocultar Parla");
}

#[test]
fn close_plan_routes_by_window_kind() {
    // A secondary window always closes; the main window hides with the icon on.
    assert_eq!(close_plan(false, true), CloseAction::Close);
    assert_eq!(close_plan(false, false), CloseAction::Close);
    assert_eq!(close_plan(true, true), CloseAction::HideApp);
    assert_eq!(close_plan(true, false), CloseAction::Close);
}

struct Secondary;

impl Render for Secondary {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui::test]
fn close_window_closes_the_focused_secondary_window(cx: &mut TestAppContext) {
    cx.update(parla::menus::register_actions);
    let other = cx.add_window(|_, _| Secondary);
    let (_, vcx) = cx.add_window_view(|_, _| Secondary);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| cx.windows().len()), 2);
    // The menu item dispatches the action at the window in front.
    vcx.dispatch_action(CloseWindow);
    let left = vcx.cx.update(|cx| cx.windows());
    assert_eq!(left.len(), 1, "only the focused window closes");
    assert_eq!(left[0].window_id(), other.window_id());
}

#[test]
fn mac_only_actions_stay_out_of_the_palette_elsewhere() {
    for name in ["app::Hide", "app::HideOthers", "app::ShowAll"] {
        assert!(parla::actions::unsupported_here(name, false), "{name}");
        assert!(!parla::actions::unsupported_here(name, true), "{name}");
    }
    for name in ["app::Minimize", "app::Zoom", "app::Quit"] {
        assert!(!parla::actions::unsupported_here(name, false), "{name}");
    }
}
