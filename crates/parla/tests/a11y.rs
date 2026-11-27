//! Accessibility and keyboard: focus cycling, action coverage and labels.

mod support;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use gpui::TestAppContext;
use parla::root_view::RootView;
use settings::parse_keymap;
use store::{MemoryStore, Store};

use components::AggregateKind;
use support::*;

/// Actions bound in every default keymap (union across platforms).
fn default_bound_actions() -> BTreeSet<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/keymaps");
    let mut out = BTreeSet::new();
    for file in [
        "default-macos.json",
        "default-windows.json",
        "default-linux.json",
    ] {
        let path = dir.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        for block in parse_keymap(&path, &text).unwrap() {
            out.extend(block.bindings.into_values());
        }
    }
    out
}

#[gpui::test]
async fn every_registered_action_is_bound_or_in_palette(cx: &mut TestAppContext) {
    // A window initializes the app's action handlers and keymap context.
    let e = env(cx, "{}");
    let dirs = parla::removal::AppDirs {
        data: e.data.clone(),
        cache: e.cache.clone(),
    };
    cx.update(|cx| {
        cx.set_global(dirs);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    let r = e.registry.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (_view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, st, window, cx));
    cx.run_until_parked();

    let bound = default_bound_actions();
    let palette_items = _view.update_in(cx, |_, window, cx| {
        RootView::command_palette_items(window, cx)
    });
    let palette_names: BTreeSet<String> =
        palette_items.iter().map(|i| i.name.to_string()).collect();
    assert!(
        palette_items.iter().all(|i| !i.title.is_empty()),
        "palette items must have a non-empty localized title"
    );
    // Pane-specific editing and internal actions are excluded from the palette.
    let excluded = |name: &str| {
        parla::actions::unsupported_here(name, cfg!(target_os = "macos"))
            || name.starts_with("zed::")
            || name.starts_with("editor::")
            || name.starts_with("buffer::")
            || name.starts_with("text_input::")
            || name.starts_with("picker::")
            // GPUI Base primitives' own actions (registered by `gpui_base::init`).
            || name.starts_with("grid_layout::")
            || name.starts_with("input::")
            || name.starts_with("number_input::")
            || name.starts_with("root::")
            || name.starts_with("ui::")
            || matches!(
                name,
                "composer::Bold"
                    | "composer::Italic"
                    | "composer::Strike"
                    | "composer::Code"
                    | "composer::CodeBlock"
                    | "composer::Quote"
                    | "composer::BulletList"
                    | "composer::NumberedList"
                    | "composer::Link"
            )
    };

    let all_names = cx.update(|_, cx| cx.all_action_names().to_vec());
    let mut missing = Vec::new();
    let mut wrongly_listed = Vec::new();
    for name in all_names {
        let name_s = name.to_string();
        if excluded(&name_s) {
            if palette_names.contains(&name_s) {
                wrongly_listed.push(name_s);
            }
            continue;
        }
        if !bound.contains(&name_s) && !palette_names.contains(&name_s) {
            missing.push(name_s);
        }
    }
    missing.sort();
    wrongly_listed.sort();
    assert!(
        wrongly_listed.is_empty(),
        "internal or editing actions must not appear in the command palette: {wrongly_listed:?}"
    );
    assert!(
        missing.is_empty(),
        "actions neither bound in a default keymap nor in the command palette: {missing:?}"
    );
}

#[gpui::test]
async fn focus_cycles_through_every_pane(cx: &mut TestAppContext) {
    // This test exercises focus cycling (cmd-f6/ctrl-f6 forward, shift for back)
    // through rail, sidebar, timeline, composer and the right panel when open.
    // Implemented as part of task 10.
    let e = env(cx, "{}");
    for (i, p) in e.providers.iter().enumerate() {
        p.add_conversation(domain::test_support::conv(
            &format!("C{}", i + 1),
            &format!("chan{}", i + 1),
        ));
    }
    let dirs = parla::removal::AppDirs {
        data: e.data.clone(),
        cache: e.cache.clone(),
    };
    cx.update(|cx| {
        cx.set_global(dirs);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    let r = e.registry.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, st, window, cx));
    // Settle so a conversation can be opened.
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Open a conversation so timeline and composer are present.
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(domain::test_support::cid("C1"), cx));
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let focused = |view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext| -> String {
        view.update_in(cx, |v, window, cx| {
            v.focused_pane_name(window, cx)
                .unwrap_or("none")
                .to_string()
        })
    };

    // Cycle forward through the four always-present panes.
    let forward = if cfg!(target_os = "macos") {
        "cmd-f6"
    } else {
        "ctrl-f6"
    };
    let backward = if cfg!(target_os = "macos") {
        "cmd-shift-f6"
    } else {
        "ctrl-shift-f6"
    };

    // Start from a known pane.
    view.update_in(cx, |v, window, cx| {
        v.focus_pane(parla::root_view::Pane::Rail, window, cx)
    });
    assert_eq!(focused(&view, cx), "rail");

    cx.simulate_keystrokes(forward);
    cx.run_until_parked();
    assert_eq!(focused(&view, cx), "sidebar");

    cx.simulate_keystrokes(forward);
    cx.run_until_parked();
    assert_eq!(focused(&view, cx), "timeline");

    cx.simulate_keystrokes(forward);
    cx.run_until_parked();
    assert_eq!(focused(&view, cx), "composer");

    // No right panel open: wraps to rail.
    cx.simulate_keystrokes(forward);
    cx.run_until_parked();
    assert_eq!(focused(&view, cx), "rail");

    // Backward wraps to composer.
    cx.simulate_keystrokes(backward);
    cx.run_until_parked();
    assert_eq!(focused(&view, cx), "composer");

    // Open a right panel (search) and verify it joins the cycle.
    view.update_in(cx, |v, window, cx| v.open_search(window, cx));
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    view.update_in(cx, |v, window, cx| {
        v.focus_pane(parla::root_view::Pane::Composer, window, cx)
    });
    cx.simulate_keystrokes(forward);
    cx.run_until_parked();
    assert_eq!(focused(&view, cx), "right_panel");

    cx.simulate_keystrokes(forward);
    cx.run_until_parked();
    assert_eq!(focused(&view, cx), "rail");
}

#[gpui::test]
async fn focus_cycle_skips_hidden_panes_when_aggregate_open(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    for (i, p) in e.providers.iter().enumerate() {
        p.add_conversation(domain::test_support::conv(
            &format!("C{}", i + 1),
            &format!("chan{}", i + 1),
        ));
    }
    let dirs = parla::removal::AppDirs {
        data: e.data.clone(),
        cache: e.cache.clone(),
    };
    cx.update(|cx| {
        cx.set_global(dirs);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    let r = e.registry.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, st, window, cx));
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    view.update_in(cx, |v, window, cx| {
        v.open_aggregate(AggregateKind::Unreads, window, cx)
    });
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let focused = |view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext| -> String {
        view.update_in(cx, |v, window, cx| {
            v.focused_pane_name(window, cx)
                .unwrap_or("none")
                .to_string()
        })
    };

    let forward = if cfg!(target_os = "macos") {
        "cmd-f6"
    } else {
        "ctrl-f6"
    };

    view.update_in(cx, |v, window, cx| {
        v.focus_pane(parla::root_view::Pane::Rail, window, cx)
    });
    assert_eq!(focused(&view, cx), "rail");
    cx.simulate_keystrokes(forward);
    cx.run_until_parked();
    assert_eq!(focused(&view, cx), "sidebar");
    cx.simulate_keystrokes(forward);
    cx.run_until_parked();
    // Aggregate/collection center takes the timeline slot in the cycle.
    assert_eq!(focused(&view, cx), "timeline");
    cx.simulate_keystrokes(forward);
    cx.run_until_parked();
    assert_eq!(focused(&view, cx), "rail");
}
#[gpui::test]
async fn command_palette_opens_and_closes_with_escape(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    for (i, p) in e.providers.iter().enumerate() {
        p.add_conversation(domain::test_support::conv(
            &format!("C{}", i + 1),
            &format!("chan{}", i + 1),
        ));
    }
    let dirs = parla::removal::AppDirs {
        data: e.data.clone(),
        cache: e.cache.clone(),
    };
    cx.update(|cx| {
        cx.set_global(dirs);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    let r = e.registry.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, st, window, cx));
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(domain::test_support::cid("C1"), cx));
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    view.update_in(cx, |v, window, cx| {
        v.focus_pane(parla::root_view::Pane::Timeline, window, cx)
    });
    cx.run_until_parked();

    // Open palette via the action
    view.update_in(cx, |v, window, cx| v.open_command_palette(window, cx));
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |v, _| v.command_palette().is_some()),
        "palette should be open"
    );

    // Press Escape to close
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        view.read_with(cx, |v, _| v.command_palette().is_none()),
        "palette should be closed after escape"
    );
}

#[gpui::test]
async fn shortcuts_sheet_opens_with_its_action_and_closes_with_escape(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dirs = parla::removal::AppDirs {
        data: e.data.clone(),
        cache: e.cache.clone(),
    };
    cx.update(|cx| {
        cx.set_global(dirs);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    let r = e.registry.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, st, window, cx));
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    view.update_in(cx, |v, window, cx| {
        v.focus_pane(parla::root_view::Pane::Sidebar, window, cx)
    });
    cx.run_until_parked();

    cx.dispatch_action(parla::actions::OpenKeyboardShortcuts);
    cx.run_until_parked();
    let labels = view.read_with(cx, |v, cx| {
        v.shortcuts_sheet().map(|s| s.read(cx).visible_labels(cx))
    });
    let labels = labels.expect("sheet open");
    assert!(labels.iter().any(|l| l == "Command palette"), "{labels:?}");
    assert!(cx.debug_bounds("shortcuts-sheet").is_some());

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.shortcuts_sheet().is_none()));
}
