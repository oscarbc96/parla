use std::path::Path;

use settings::{KeymapOS, KeymapWarning, parse_keymap, resolve_bindings, resolve_bindings_with_os};

fn known(a: &str) -> bool {
    matches!(
        a,
        "app::Quit"
            | "app::OpenSettings"
            | "app::OpenKeymap"
            | "app::ReloadConfig"
            | "app::CommandPalette"
            | "app::CloseWindow"
            | "app::OpenKeyboardShortcuts"
            | "app::Hide"
            | "app::HideOthers"
            | "app::Minimize"
            | "theme::ToggleMode"
    )
}

fn parse(text: &str) -> Vec<settings::KeymapBlock> {
    parse_keymap(Path::new("keymap.json"), text).unwrap()
}

#[test]
fn parses_zed_format() {
    let blocks = parse(r#"[{"context":"Workspace","bindings":{"cmd-,":"app::OpenSettings"}}]"#);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].context.as_deref(), Some("Workspace"));
    assert_eq!(blocks[0].bindings["cmd-,"], "app::OpenSettings");
}

#[test]
fn user_overrides_default_same_key() {
    let default = parse(r#"[{"bindings":{"cmd-q":"app::Quit","cmd-,":"app::OpenSettings"}}]"#);
    let user = parse(r#"[{"bindings":{"cmd-q":"theme::ToggleMode"}}]"#);
    let (bindings, warnings) = resolve_bindings(&default, &user, &known);
    assert!(warnings.is_empty());
    assert_eq!(bindings.len(), 2);
    let q: Vec<_> = bindings
        .iter()
        .filter(|b| b.keystrokes == "cmd-q")
        .collect();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].action, "theme::ToggleMode");
}

#[test]
fn same_key_different_context_is_kept() {
    let default = parse(r#"[{"bindings":{"cmd-q":"app::Quit"}}]"#);
    let user = parse(r#"[{"context":"Workspace","bindings":{"cmd-q":"theme::ToggleMode"}}]"#);
    let (bindings, _) = resolve_bindings(&default, &user, &known);
    assert_eq!(bindings.len(), 2);
}

#[test]
fn bad_binding_is_skipped() {
    let user = parse(
        r#"[{"bindings":{"cmd-+-+":"app::Quit","cmd-k":"foo::Bar","cmd-,":"app::OpenSettings"}}]"#,
    );
    let (bindings, warnings) = resolve_bindings(&[], &user, &known);
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].keystrokes, "cmd-,");
    assert_eq!(warnings.len(), 2);
    assert!(warnings.contains(&KeymapWarning::BadKeystroke {
        keystrokes: "cmd-+-+".into(),
        context: None
    }));
    assert!(warnings.contains(&KeymapWarning::UnknownAction {
        action: "foo::Bar".into()
    }));
}

#[test]
fn comments_allowed() {
    let blocks = parse("// my keys\n[\n  // quit\n  {\"bindings\": {\"cmd-q\": \"app::Quit\"}}\n]");
    assert_eq!(blocks.len(), 1);
}

#[test]
fn parse_error_names_file() {
    let err = parse_keymap(Path::new("keymap.json"), "[{").unwrap_err();
    assert!(err.to_string().starts_with("keymap.json:"));
}

#[test]
fn shipped_defaults_are_valid() {
    for os in ["macos", "windows", "linux"] {
        let path = format!(
            "{}/../../assets/keymaps/default-{os}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap();
        let blocks = parse_keymap(Path::new(&path), &text).unwrap();
        let known_or_component = |a: &str| {
            known(a)
                || [
                    "sidebar::",
                    "timeline::",
                    "composer::",
                    "text_input::",
                    "picker::",
                    "workspace::",
                    "mcp::",
                    "call::",
                ]
                .iter()
                .any(|p| a.starts_with(p))
        };
        let (bindings, warnings) = resolve_bindings(&blocks, &[], &known_or_component);
        assert!(warnings.is_empty(), "{os}: {warnings:?}");
        // 31 app (call mute included), sidebar, timeline, picker and workspace bindings (quick switch, back/forward and
        // jump to latest all unreads and pause notifications included), the 11 rail switch keys, composer paste and accept-completion (tab), the confirmation prompt's escape, the 9
        // formatting keys, the send-later key, the 2 focus-cycle keys, the command palette key, the 2 pop-out keys, then the text input keys (macOS has no Home/End/Ctrl-Y).
        // Close window everywhere; macOS also binds hide, hide others and minimize.
        let window_keys = if os == "macos" { 4 } else { 1 };
        let text_input = if os == "macos" { 27 } else { 28 };
        assert_eq!(
            bindings.len(),
            5 + 1
                + 3
                + 4
                + 4
                + 6
                + 1
                + 2
                + 9
                + 1 // send later
                + 1 // GIF picker
                + 1
                + 1
                + 11
                + 3
                + 1
                + 1
                + 1
                + 1
                + 1
                + 2
                + 1
                + 2
                + 1 // the keyboard shortcuts sheet
                + 2 // the Cmd-Enter send key's enter and cmd-enter
                + window_keys
                + text_input
        );
    }
}

#[test]
fn shipped_default_keymaps_validate_with_their_os() {
    for (file, os) in [
        ("default-macos.json", KeymapOS::Mac),
        ("default-windows.json", KeymapOS::Windows),
        ("default-linux.json", KeymapOS::Linux),
    ] {
        let path = format!("{}/../../assets/keymaps/{file}", env!("CARGO_MANIFEST_DIR"));
        assert_eq!(KeymapOS::for_path(Path::new(&path)), os);
        let text = std::fs::read_to_string(&path).unwrap();
        let blocks = parse_keymap(Path::new(&path), &text).unwrap();
        let (bindings, warnings) = resolve_bindings_with_os(&blocks, &[], os, os, &|_| true);
        assert!(warnings.is_empty(), "{file}: {warnings:?}");
        assert!(!bindings.is_empty(), "{file}: no bindings");
    }
    assert_eq!(
        KeymapOS::for_path(Path::new("keymap.json")),
        KeymapOS::running()
    );
}

#[test]
fn type_error_pointer_escapes_slash_in_keys() {
    let err =
        parse_keymap(Path::new("keymap.json"), r#"[{"bindings": {"cmd-/": 5}}]"#).unwrap_err();
    match err {
        settings::SettingsError::Type { pointer, .. } => {
            assert_eq!(pointer, "/0/bindings/cmd-~1");
        }
        other => panic!("expected a type error, got {other:?}"),
    }
}

#[test]
fn null_unbinds_a_default_binding() {
    let default = parse(r#"[{"bindings":{"cmd-q":"app::Quit","cmd-,":"app::OpenSettings"}}]"#);
    let user = parse(r#"[{"bindings":{"cmd-q":null}}]"#);
    let (bindings, warnings) = resolve_bindings(&default, &user, &known);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].action, "app::OpenSettings");
}

#[test]
fn set_user_binding_creates_the_file_text() {
    let out = settings::set_user_binding("", Some("Composer"), "cmd-e", Some("composer::EditLast"))
        .unwrap();
    let blocks = parse(&out);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].context.as_deref(), Some("Composer"));
    assert_eq!(blocks[0].bindings["cmd-e"], "composer::EditLast");
}

#[test]
fn set_user_binding_edits_the_matching_block_and_keeps_comments() {
    let text = "// mine\n[\n  {\n    \"context\": \"Timeline\",\n    \"bindings\": {\n      // keep\n      \"e\": \"timeline::EditMessage\"\n    }\n  }\n]\n";
    let out = settings::set_user_binding(text, Some("Timeline"), "cmd-e", None).unwrap();
    assert!(out.contains("// mine") && out.contains("// keep"), "{out}");
    let blocks = parse(&out);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].bindings["cmd-e"], "", "null reads as unbind");
    assert_eq!(blocks[0].bindings["e"], "timeline::EditMessage");

    let out = settings::set_user_binding(&out, None, "cmd-q", Some("app::Quit")).unwrap();
    let blocks = parse(&out);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[1].context, None);
    assert_eq!(blocks[1].bindings["cmd-q"], "app::Quit");

    let out = settings::set_user_binding("[]", None, "cmd-q", Some("app::Hide")).unwrap();
    assert_eq!(parse(&out)[0].bindings["cmd-q"], "app::Hide");
    assert!(settings::set_user_binding("{}", None, "cmd-q", None).is_none());
}

#[test]
fn remove_user_bindings_drops_only_matching_members() {
    let text = "[\n  {\n    \"bindings\": {\n      \"cmd-q\": \"app::Quit\", // quit\n      \"cmd-h\": null,\n      \"cmd-m\": \"app::Minimize\"\n    }\n  },\n  {\n    \"context\": \"Timeline\",\n    \"bindings\": { \"cmd-q\": \"app::Quit\" }\n  }\n]";
    let out = settings::remove_user_bindings(text, &|ctx, keys, action| {
        ctx.is_none() && (action == Some("app::Quit") || keys == "cmd-m")
    })
    .unwrap();
    let blocks = parse(&out);
    assert_eq!(blocks[0].bindings.len(), 1, "{out}");
    assert_eq!(blocks[0].bindings["cmd-h"], "");
    assert_eq!(blocks[1].bindings["cmd-q"], "app::Quit");
    let out = settings::remove_user_bindings(&out, &|_, _, _| true).unwrap();
    assert!(parse(&out).iter().all(|b| b.bindings.is_empty()), "{out}");
}

#[test]
fn modifier_order_does_not_matter() {
    assert_eq!(
        settings::canonical_keys("cmd-shift-p"),
        settings::canonical_keys("shift-cmd-p")
    );
    assert_eq!(settings::canonical_keys("cmd-shift--"), "shift-cmd--");
    let default = parse(r#"[{"bindings":{"cmd-shift-q":"app::Quit"}}]"#);
    let user = parse(r#"[{"bindings":{"shift-cmd-q":"app::Hide"}}]"#);
    let (bindings, _) = resolve_bindings(&default, &user, &known);
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].action, "app::Hide");
}

#[test]
fn nulls_apply_before_bindings_whatever_the_key_order() {
    let default = parse(r#"[{"bindings":{"cmd-alt-h":"app::HideOthers"}}]"#);
    // "alt-cmd-h" sorts before "cmd-alt-h": the null must not remove Quit.
    let user = parse(r#"[{"bindings":{"alt-cmd-h":"app::Quit","cmd-alt-h":null}}]"#);
    let (bindings, _) = resolve_bindings(&default, &user, &known);
    assert_eq!(bindings.len(), 1, "{bindings:?}");
    assert_eq!(bindings[0].action, "app::Quit");
}

#[test]
fn set_user_binding_replaces_the_same_keys_spelled_differently() {
    let text = "[\n  {\n    \"bindings\": {\n      \"cmd-alt-h\": \"app::Quit\"\n    }\n  }\n]\n";
    let out = settings::set_user_binding(text, None, "alt-cmd-h", Some("app::Hide")).unwrap();
    let blocks = parse(&out);
    assert_eq!(blocks[0].bindings.len(), 1, "{out}");
    assert_eq!(blocks[0].bindings["alt-cmd-h"], "app::Hide");
    let out = settings::set_user_binding(&out, None, "CMD-ALT-H", None).unwrap();
    assert_eq!(parse(&out)[0].bindings.len(), 1, "{out}");
}

#[test]
fn canonical_keys_ignore_case_and_platform_aliases() {
    let c = settings::canonical_keys;
    assert_eq!(c("Cmd-Shift-P"), c("shift-cmd-p"));
    assert_eq!(c("super-k"), c("cmd-k"));
    assert_eq!(c("win-k"), c("cmd-k"));
    assert_eq!(c("cmd-k cmd-s"), c("CMD-K Cmd-S"));
}

#[test]
fn removal_keeps_trailing_commas_valid() {
    for text in [
        r#"[{"bindings": {"cmd-q": "app::Quit",},},]"#,
        r#"[{"bindings": {"cmd-h": null, "cmd-q": "app::Quit",}}]"#,
        r#"[{"bindings": {"cmd-q": "app::Quit", "cmd-h": null,}}]"#,
    ] {
        let out = settings::remove_user_bindings(text, &|_, k, _| k == "cmd-q").unwrap();
        let blocks =
            parse_keymap(Path::new("keymap.json"), &out).unwrap_or_else(|e| panic!("{out}: {e}"));
        assert!(!blocks[0].bindings.contains_key("cmd-q"), "{out}");
    }
}

#[test]
fn removal_keeps_comments_above_the_next_binding_and_trailing_ones() {
    let text = "[\n  {\n    \"bindings\": {\n      \"cmd-q\": \"app::Quit\",\n      // why hide\n      \"cmd-h\": \"app::Hide\" // trailing\n    }\n  }\n]\n";
    let out = settings::remove_user_bindings(text, &|_, k, _| k == "cmd-q").unwrap();
    assert!(
        out.contains("// why hide") && out.contains("// trailing"),
        "{out}"
    );
    assert_eq!(parse(&out)[0].bindings.len(), 1, "{out}");
    let out = settings::remove_user_bindings(text, &|_, k, _| k == "cmd-h").unwrap();
    assert!(
        out.contains("// why hide") && out.contains("// trailing"),
        "{out}"
    );
    assert_eq!(parse(&out)[0].bindings["cmd-q"], "app::Quit", "{out}");
}

#[test]
fn first_block_in_a_commented_empty_array_keeps_the_comment() {
    let out = settings::set_user_binding("[\n  // my keys\n]\n", None, "cmd-q", Some("app::Quit"))
        .unwrap();
    assert!(out.contains("// my keys"), "{out}");
    assert_eq!(parse(&out)[0].bindings["cmd-q"], "app::Quit", "{out}");
    let out =
        settings::set_user_binding("[] // none yet\n", None, "cmd-q", Some("app::Quit")).unwrap();
    assert!(out.contains("// none yet"), "{out}");
    assert_eq!(parse(&out)[0].bindings["cmd-q"], "app::Quit", "{out}");
}

#[cfg(unix)]
#[test]
fn keymap_writes_follow_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("dotfiles-keymap.json");
    std::fs::write(&real, "[]").unwrap();
    let link = dir.path().join("keymap.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(
        settings::edit_keymap_file(&link, |t| settings::set_user_binding(
            t,
            None,
            "cmd-q",
            Some("app::Quit")
        ))
        .unwrap()
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::read_to_string(&real)
            .unwrap()
            .contains("app::Quit")
    );
}
