use gpui::TestAppContext;
use parla::actions::ToggleMode;
use parla::{ConfigBanner, about, active_theme, handle_change, init_without_watcher};
use settings::{ChangedFile, ConfigPaths, SettingsError};

fn setup(cx: &mut TestAppContext, settings: &str) -> (tempfile::TempDir, ConfigPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    std::fs::write(dir.path().join("settings.json"), settings).unwrap();
    let p = paths.clone();
    cx.update(|cx| init_without_watcher(cx, p));
    (dir, paths)
}

fn theme_name(cx: &mut TestAppContext) -> String {
    cx.update(|cx| active_theme(cx).name.to_string())
}

#[gpui::test]
async fn window_uses_theme_from_settings(cx: &mut TestAppContext) {
    let (_dir, _paths) = setup(cx, r#"{ "theme": { "mode": "light" } }"#);
    assert_eq!(theme_name(cx), "Parla Light");
}

#[gpui::test]
async fn editing_settings_switches_theme(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx, r#"{ "theme": { "mode": "light" } }"#);
    assert_eq!(theme_name(cx), "Parla Light");
    std::fs::write(&paths.settings, r#"{ "theme": { "mode": "dark" } }"#).unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Settings));
    cx.run_until_parked();
    assert_eq!(theme_name(cx), "Parla Dark");
}

#[gpui::test]
async fn bad_settings_shows_banner_and_keeps_theme(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx, r#"{ "theme": { "mode": "light" } }"#);
    std::fs::write(&paths.settings, "{").unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Settings));
    cx.run_until_parked();
    assert_eq!(theme_name(cx), "Parla Light");
    cx.update(|cx| {
        let banner = cx.global::<ConfigBanner>();
        assert!(matches!(banner.error, Some(SettingsError::Parse { .. })));
    });
    // Fixing the file clears the banner.
    std::fs::write(&paths.settings, "{}").unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Settings));
    cx.update(|cx| assert!(cx.global::<ConfigBanner>().error.is_none()));
}

#[gpui::test]
async fn toggle_mode_action_writes_file(cx: &mut TestAppContext) {
    let text = "{\n  // my comment\n  \"theme\": { \"mode\": \"light\" }\n}\n";
    let (_dir, paths) = setup(cx, text);
    cx.update(|cx| cx.dispatch_action(&ToggleMode));
    cx.run_until_parked();
    let after = std::fs::read_to_string(&paths.settings).unwrap();
    assert!(after.contains("// my comment"), "{after}");
    assert!(after.contains(r#""mode": "dark""#), "{after}");
    assert_eq!(theme_name(cx), "Parla Dark");
}

#[test]
fn about_contains_notice() {
    let chain = i18n::chain(
        &i18n::LocalePref::Code("en".parse().unwrap()),
        &[],
        i18n::LocaleSet::embedded(),
    );
    let l = i18n::Localizer::new(
        chain,
        &[parla::catalog(), domain::test_support::fake_catalog()],
    )
    .unwrap();
    let text = about::about_text(&l, &[domain::test_support::fake_catalog()], "1.2.3");
    assert!(text.contains("This is the fake provider used in tests."));
    assert!(text.contains("MPL-2.0"));
}

#[gpui::test]
fn component_actions_in_default_keymap_build(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        components::init(cx);
        for a in [
            "sidebar::SelectNext",
            "sidebar::SelectPrev",
            "sidebar::Open",
            "timeline::EditMessage",
            "timeline::DeleteMessage",
            "composer::Send",
            "composer::Newline",
            "composer::PlainEnter",
            "composer::EditLast",
            "composer::Cancel",
            "composer::Paste",
            "text_input::Left",
            "text_input::SelectAll",
            "text_input::Undo",
        ] {
            assert!(parla::actions::is_known(a), "{a}");
            assert!(cx.build_action(a, None).is_ok(), "{a}");
        }
    });
}

fn just_now(cx: &mut TestAppContext) -> String {
    cx.update(|cx| {
        use i18n::Translate as _;
        i18n::gpui::localizer(cx)
            .localizer()
            .format("i18n", "time-just-now", None)
            .into_owned()
    })
}

#[gpui::test]
async fn reload_switches_language(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx, r#"{ "locale": "en" }"#);
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let h = hits.clone();
    cx.update(|cx| i18n::gpui::on_relocalize(cx, move |_| h.set(h.get() + 1)));
    assert_eq!(just_now(cx), "just now");
    std::fs::write(&paths.settings, r#"{ "locale": "ca" }"#).unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Settings));
    cx.run_until_parked();
    assert_eq!(just_now(cx), "ara mateix");
    assert_eq!(hits.get(), 1);
}

#[gpui::test]
async fn same_chain_does_not_refresh(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx, r#"{ "locale": "ca" }"#);
    let hits = std::rc::Rc::new(std::cell::Cell::new(0));
    let h = hits.clone();
    cx.update(|cx| i18n::gpui::on_relocalize(cx, move |_| h.set(h.get() + 1)));
    std::fs::write(&paths.settings, r#"{ "locale": "ca", "ui_font_size": 15 }"#).unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Settings));
    cx.run_until_parked();
    assert_eq!(hits.get(), 0);
    assert_eq!(just_now(cx), "ara mateix");
}

#[gpui::test]
async fn keymap_reload_keeps_theme_warnings(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    std::fs::create_dir_all(&paths.themes_dir).unwrap();
    std::fs::write(paths.themes_dir.join("bad.json"), "{").unwrap();
    let p = paths.clone();
    cx.update(|cx| init_without_watcher(cx, p));
    let before = cx.update(|cx| cx.global::<ConfigBanner>().warnings.clone());
    assert_eq!(before.len(), 1, "{before:?}");
    std::fs::write(&paths.keymap, "{").unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Keymap));
    let after = cx.update(|cx| cx.global::<ConfigBanner>().warnings.clone());
    assert_eq!(after.len(), 2, "{after:?}");
    assert!(after.contains(&before[0]));
    // Fixing the keymap drops only its own warning.
    std::fs::write(&paths.keymap, "[]").unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Keymap));
    let fixed = cx.update(|cx| cx.global::<ConfigBanner>().warnings.clone());
    assert_eq!(fixed, before);
}

#[gpui::test]
async fn reload_keeps_the_disk_notice(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx, "{}");
    cx.update(|cx| parla::store_banner::warn_if_unencrypted_later(cx, || Some(false)));
    cx.executor().allow_parking();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(cx.global::<ConfigBanner>().warnings.len(), 1));
    std::fs::write(&paths.settings, r#"{ "theme": { "mode": "dark" } }"#).unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Settings));
    cx.update(|cx| assert_eq!(cx.global::<ConfigBanner>().warnings.len(), 1));
}

/// Action name bound to `key` in a Composer context after `init_without_watcher`.
fn composer_binding(cx: &mut TestAppContext, key: &str, cmd_enter: bool) -> Option<String> {
    cx.update(|cx| {
        let ctx = if cmd_enter {
            "Composer send_cmd_enter"
        } else {
            "Composer"
        };
        let stack = [gpui::KeyContext::parse(ctx).unwrap()];
        let keystroke = gpui::Keystroke::parse_with_os(key, cx.platform_info().os).unwrap();
        let keymap = cx.key_bindings();
        let (found, _) = keymap.borrow().bindings_for_input(&[keystroke], &stack);
        found.first().map(|b| b.action().name().to_string())
    })
}

#[gpui::test]
async fn cmd_enter_send_key_survives_keymap_binding(cx: &mut TestAppContext) {
    let _ = setup(cx, "{}");
    // A keymap reload clears every binding and rebinds only the keymap files.
    cx.update(|cx| handle_change(cx, ChangedFile::Keymap));
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let cmd_enter = format!("{modifier}-enter");
    assert_eq!(
        composer_binding(cx, "enter", false).as_deref(),
        Some("composer::Send")
    );
    assert_eq!(
        composer_binding(cx, "enter", true).as_deref(),
        Some("composer::PlainEnter")
    );
    assert_eq!(
        composer_binding(cx, &cmd_enter, true).as_deref(),
        Some("composer::Send")
    );
    assert_eq!(composer_binding(cx, &cmd_enter, false), None);
}
