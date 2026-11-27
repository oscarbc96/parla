//! The shortcuts model behind the keyboard shortcuts sheet and the Keymap
//! settings page: rows from the default plus user keymap, areas, filters,
//! conflicts and the edits a recorded binding writes to `keymap.json`.

use parla::shortcuts::{self, Area, Shortcut, Source};

const DEFAULT: &str = r#"[
  { "bindings": { "cmd-q": "app::Quit", "cmd-k": "workspace::QuickSwitch" } },
  { "context": "Composer", "bindings": { "enter": "composer::Send", "cmd-b": "composer::Bold" } },
  { "context": "Timeline", "bindings": { "e": "timeline::EditMessage" } }
]"#;

fn row<'a>(rows: &'a [Shortcut], action: &str) -> &'a Shortcut {
    rows.iter()
        .find(|r| r.action == action)
        .unwrap_or_else(|| panic!("{action} listed"))
}

#[test]
fn rows_list_every_action_with_its_live_binding_and_source() {
    let user = r#"[{ "bindings": { "cmd-shift-q": "app::Quit", "cmd-q": null } },
                  { "context": "Composer", "bindings": { "cmd-b": "composer::Italic" } }]"#;
    let rows = shortcuts::shortcuts(DEFAULT, user);
    let quit: Vec<_> = rows.iter().filter(|r| r.action == "app::Quit").collect();
    assert_eq!(quit.len(), 1, "{quit:?}");
    assert_eq!(quit[0].keystrokes.as_deref(), Some("cmd-shift-q"));
    assert_eq!(quit[0].source, Source::User);
    let send = row(&rows, "composer::Send");
    assert_eq!(send.keystrokes.as_deref(), Some("enter"));
    assert_eq!(send.context.as_deref(), Some("Composer"));
    assert_eq!(send.source, Source::Default);
    // The user moved cmd-b to Italic, so Bold is now unbound in Composer.
    assert_eq!(row(&rows, "composer::Bold").keystrokes, None);
    assert_eq!(
        row(&rows, "composer::Bold").context.as_deref(),
        Some("Composer")
    );
    assert_eq!(
        row(&rows, "composer::Italic").keystrokes.as_deref(),
        Some("cmd-b")
    );
    // Every known action is listed, bound or not.
    for name in parla::actions::KNOWN {
        if !parla::actions::unsupported_here(name, cfg!(target_os = "macos")) {
            assert!(rows.iter().any(|r| r.action == *name), "{name} missing");
        }
    }
}

#[test]
fn actions_fall_into_areas() {
    assert_eq!(shortcuts::area("workspace::QuickSwitch"), Area::Navigation);
    assert_eq!(shortcuts::area("sidebar::NextUnread"), Area::Navigation);
    assert_eq!(shortcuts::area("timeline::EditMessage"), Area::Messages);
    assert_eq!(shortcuts::area("composer::Bold"), Area::Composer);
    assert_eq!(shortcuts::area("text_input::Copy"), Area::Text);
    assert_eq!(shortcuts::area("call::ToggleMute"), Area::Calls);
    assert_eq!(shortcuts::area("app::OpenSettings"), Area::App);
    assert_eq!(shortcuts::area("theme::ToggleMode"), Area::App);
}

#[test]
fn filter_matches_title_action_and_keys() {
    let rows = shortcuts::shortcuts(DEFAULT, "");
    let quit = row(&rows, "app::Quit");
    assert!(shortcuts::matches(quit, "Quit Parla", "quit", true));
    assert!(shortcuts::matches(quit, "Quit Parla", "app::q", true));
    assert!(shortcuts::matches(quit, "Quit Parla", "cmd-q", true));
    assert!(shortcuts::matches(quit, "Quit Parla", "\u{2318}Q", true));
    assert!(shortcuts::matches(quit, "Quit Parla", "", true));
    assert!(!shortcuts::matches(quit, "Quit Parla", "bold", true));
}

#[test]
fn a_recorded_key_already_bound_in_the_context_is_a_conflict() {
    let rows = shortcuts::shortcuts(DEFAULT, "");
    let bold = row(&rows, "composer::Bold");
    let hit = shortcuts::conflict(&rows, "enter", bold).expect("enter sends");
    assert_eq!(hit.action, "composer::Send");
    assert!(
        shortcuts::conflict(&rows, "cmd-b", bold).is_none(),
        "its own key"
    );
    assert!(shortcuts::conflict(&rows, "cmd-i", bold).is_none());
    assert!(
        shortcuts::conflict(&rows, "e", bold).is_none(),
        "another context"
    );
}

#[test]
fn rebinding_a_default_unbinds_it_and_reset_restores_it() {
    let rows = shortcuts::shortcuts(DEFAULT, "");
    let quit = row(&rows, "app::Quit").clone();
    let text = shortcuts::rebind("[]", &quit, "cmd-shift-q").unwrap();
    let rows = shortcuts::shortcuts(DEFAULT, &text);
    let quit: Vec<_> = rows.iter().filter(|r| r.action == "app::Quit").collect();
    assert_eq!(quit.len(), 1, "{text}");
    assert_eq!(quit[0].keystrokes.as_deref(), Some("cmd-shift-q"));
    assert_eq!(quit[0].source, Source::User);

    // Rebinding a user binding replaces it rather than adding a second one.
    let again = shortcuts::rebind(&text, quit[0], "cmd-alt-q").unwrap();
    let rows = shortcuts::shortcuts(DEFAULT, &again);
    let quit: Vec<_> = rows.iter().filter(|r| r.action == "app::Quit").collect();
    assert_eq!(quit.len(), 1, "{again}");
    assert_eq!(quit[0].keystrokes.as_deref(), Some("cmd-alt-q"));

    let reset = shortcuts::reset(&again, DEFAULT, quit[0]).unwrap();
    let rows = shortcuts::shortcuts(DEFAULT, &reset);
    let quit = row(&rows, "app::Quit");
    assert_eq!(quit.keystrokes.as_deref(), Some("cmd-q"), "{reset}");
    assert_eq!(quit.source, Source::Default);
}

#[test]
fn an_unbound_action_gets_its_area_context() {
    let rows = shortcuts::shortcuts(DEFAULT, "");
    let italic = row(&rows, "composer::Italic").clone();
    assert_eq!(italic.keystrokes, None);
    assert_eq!(italic.context.as_deref(), Some("Composer"));
    let text = shortcuts::rebind("", &italic, "cmd-i").unwrap();
    let rows = shortcuts::shortcuts(DEFAULT, &text);
    let italic = row(&rows, "composer::Italic");
    assert_eq!(italic.keystrokes.as_deref(), Some("cmd-i"));
    assert_eq!(italic.context.as_deref(), Some("Composer"));
}

#[test]
fn keys_show_as_glyphs_on_mac_and_words_elsewhere() {
    assert_eq!(
        shortcuts::key_caps("cmd-shift-k", true),
        ["\u{2318}", "\u{21E7}", "K"]
    );
    assert_eq!(
        shortcuts::key_caps("ctrl-shift-k", false),
        ["Ctrl", "Shift", "K"]
    );
    assert_eq!(
        shortcuts::key_caps("cmd-shift--", true),
        ["\u{2318}", "\u{21E7}", "-"]
    );
    assert_eq!(
        shortcuts::key_caps("cmd-k cmd-s", true),
        ["\u{2318}", "K", "\u{2318}", "S"]
    );
    assert_eq!(shortcuts::key_caps("enter", true), ["\u{23CE}"]);
}

/// Cmd+/ (Ctrl+/ elsewhere) opens the sheet, and no default binding used it.
#[test]
fn every_default_keymap_binds_the_sheet_without_a_conflict() {
    for (file, key) in [
        ("default-macos.json", "cmd-/"),
        ("default-linux.json", "ctrl-/"),
        ("default-windows.json", "ctrl-/"),
    ] {
        let path = format!("{}/../../assets/keymaps/{file}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap();
        let blocks = settings::parse_keymap(std::path::Path::new(file), &text).unwrap();
        let uses: Vec<_> = blocks
            .iter()
            .filter_map(|b| b.bindings.get(key).map(|a| (b.context.clone(), a.clone())))
            .collect();
        assert_eq!(
            uses,
            [(None, "app::OpenKeyboardShortcuts".to_string())],
            "{file}"
        );
    }
}

#[test]
fn recording_the_keys_of_a_nulled_default_spelled_another_way_keeps_both() {
    let default = r#"[{ "bindings": { "cmd-alt-h": "app::HideOthers", "cmd-q": "app::Quit" } }]"#;
    let rows = shortcuts::shortcuts(default, "");
    let hide = row(&rows, "app::HideOthers").clone();
    let text = shortcuts::rebind("", &hide, "cmd-shift-h").unwrap();
    let rows = shortcuts::shortcuts(default, &text);
    let quit = row(&rows, "app::Quit").clone();
    // GPUI spells the recorded chord alt-cmd-h.
    let text = shortcuts::rebind(&text, &quit, "alt-cmd-h").unwrap();
    let rows = shortcuts::shortcuts(default, &text);
    assert_eq!(
        row(&rows, "app::Quit").keystrokes.as_deref(),
        Some("alt-cmd-h"),
        "{text}"
    );
    assert_eq!(
        row(&rows, "app::HideOthers").keystrokes.as_deref(),
        Some("cmd-shift-h"),
        "{text}"
    );
}

#[test]
fn a_hand_written_binding_is_replaced_by_the_same_keys_recorded() {
    let user = r#"[{ "bindings": { "cmd-alt-h": "app::Hide" } }]"#;
    let rows = shortcuts::shortcuts(DEFAULT, user);
    let quit = row(&rows, "app::Quit").clone();
    let text = shortcuts::rebind(user, &quit, "alt-cmd-h").unwrap();
    let rows = shortcuts::shortcuts(DEFAULT, &text);
    assert_eq!(
        row(&rows, "app::Quit").keystrokes.as_deref(),
        Some("alt-cmd-h"),
        "{text}"
    );
    assert_eq!(row(&rows, "app::Hide").keystrokes, None, "{text}");
}

#[test]
fn a_global_binding_and_a_context_one_with_the_same_keys_conflict() {
    let rows = shortcuts::shortcuts(DEFAULT, "");
    let bold = row(&rows, "composer::Bold");
    let hit = shortcuts::conflict(&rows, "cmd-q", bold).expect("cmd-q is global");
    assert_eq!(hit.action, "app::Quit");
    let quit = row(&rows, "app::Quit");
    let hit = shortcuts::conflict(&rows, "cmd-b", quit).expect("shadowed in Composer");
    assert_eq!(hit.action, "composer::Bold");
    assert!(
        shortcuts::conflict(&rows, "e", bold).is_none(),
        "Timeline vs Composer"
    );
    // The live keymap: rows that share keys with another action's row.
    let user = r#"[{ "bindings": { "cmd-b": "app::Hide" } }]"#;
    let rows = shortcuts::shortcuts(DEFAULT, user);
    let hits: Vec<&str> = rows
        .iter()
        .filter(|r| shortcuts::in_conflict(&rows, r))
        .map(|r| r.action.as_str())
        .collect();
    assert_eq!(hits, ["composer::Bold", "app::Hide"]);
}

#[test]
fn typing_keys_need_a_modifier_outside_non_text_contexts() {
    use shortcuts::needs_modifier;
    assert!(needs_modifier("k", None));
    assert!(needs_modifier("shift-k", None));
    assert!(needs_modifier("k", Some("Composer")));
    assert!(needs_modifier("space", Some("TextInput")));
    assert!(!needs_modifier("k", Some("Timeline")));
    assert!(!needs_modifier("enter", Some("Composer")));
    assert!(!needs_modifier("cmd-k", None));
    assert!(!needs_modifier("f6", None));
}

#[test]
fn an_unbound_default_counts_as_changed_and_resets() {
    let user = r#"[{ "bindings": { "cmd-q": null } }]"#;
    let rows = shortcuts::shortcuts(DEFAULT, user);
    let quit = row(&rows, "app::Quit").clone();
    assert_eq!(quit.keystrokes, None);
    assert_eq!(quit.source, Source::User);
    let text = shortcuts::reset(user, DEFAULT, &quit).unwrap();
    let rows = shortcuts::shortcuts(DEFAULT, &text);
    assert_eq!(
        row(&rows, "app::Quit").keystrokes.as_deref(),
        Some("cmd-q"),
        "{text}"
    );
    // Never bound: stays a default row.
    assert_eq!(row(&rows, "composer::Italic").source, Source::Default);
}

#[test]
fn the_sheet_shows_workspace_switching_as_one_row() {
    let set = i18n::LocaleSet::embedded();
    let chain = i18n::chain(&i18n::LocalePref::Code("en".parse().unwrap()), &[], set);
    let l = i18n::Localizer::new(chain, &[parla::catalog()]).unwrap();
    let default = r#"[{ "bindings": {
        "cmd-1": "workspace::ActivateWorkspace1", "cmd-2": "workspace::ActivateWorkspace2",
        "cmd-3": "workspace::ActivateWorkspace3", "cmd-4": "workspace::ActivateWorkspace4",
        "cmd-5": "workspace::ActivateWorkspace5", "cmd-6": "workspace::ActivateWorkspace6",
        "cmd-7": "workspace::ActivateWorkspace7", "cmd-8": "workspace::ActivateWorkspace8",
        "cmd-9": "workspace::ActivateWorkspace9", "cmd-k": "workspace::QuickSwitch" } }]"#;
    let rows = shortcuts::shortcuts(default, "");
    let groups = shortcuts::sheet_groups(&l, &rows, true);
    let nav = &groups[0];
    let labels: Vec<&str> = nav.items.iter().map(|i| i.label.as_ref()).collect();
    assert_eq!(
        labels.iter().filter(|l| l.contains("workspace 1")).count(),
        1,
        "{labels:?}"
    );
    let range = nav
        .items
        .iter()
        .find(|i| i.label.as_ref() == "Switch to workspace 1\u{2013}9")
        .expect("one row");
    assert_eq!(range.keys, ["\u{2318}", "1\u{2026}9"]);
}
