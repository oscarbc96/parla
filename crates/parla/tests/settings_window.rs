//! The Settings window: navigation, the General page and its writes.

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use i18n::LocalePref;
use parla::settings_window::{
    AppearanceChange, AppearanceModel, GeneralChange, GeneralModel, NotificationsChange,
    NotificationsModel, Page, SettingsView,
};
use settings::{
    Clock, ConfigPaths, Density, DesktopNotifications, DockIcon, InlineImages, LocaleSetting,
    QuietHours, SettingsStore, SidebarSort, ThemeMode, Weekday,
};

fn setup(cx: &mut TestAppContext) -> (tempfile::TempDir, ConfigPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    let p = paths.clone();
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(
            cx,
            &[components::catalog(), parla::catalog()],
            LocalePref::Code("en".parse().unwrap()),
        );
        cx.set_global(SettingsStore::load(p));
    });
    (dir, paths)
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    paths: &ConfigPaths,
) -> (gpui::Entity<SettingsView>, &'a mut VisualTestContext) {
    open_on(cx, paths, Page::General)
}

fn open_on<'a>(
    cx: &'a mut TestAppContext,
    paths: &ConfigPaths,
    page: Page,
) -> (gpui::Entity<SettingsView>, &'a mut VisualTestContext) {
    let path = paths.settings.clone();
    let (view, vcx) =
        cx.add_window_view(move |_, _| parla::settings_window::view_on(path, "1.2.3".into(), page));
    vcx.run_until_parked();
    (view, vcx)
}

/// Reloads the store from the file, as the file watcher does after a write.
fn reload(vcx: &mut VisualTestContext) {
    vcx.update(|_, cx| {
        cx.global_mut::<SettingsStore>().reload();
    });
    vcx.run_until_parked();
}

fn click(vcx: &mut VisualTestContext, id: &'static str) {
    let b = vcx
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("{id} is drawn"));
    vcx.simulate_click(b.center(), Modifiers::none());
}

fn stored(paths: &ConfigPaths) -> settings::Settings {
    SettingsStore::load(paths.clone()).current().clone()
}

#[test]
fn every_change_names_its_key_and_json_value() {
    let c = |c: GeneralChange| (c.key_path().join("."), c.raw_json());
    assert_eq!(
        c(GeneralChange::Locale(LocaleSetting::System)),
        ("locale".into(), "\"system\"".into())
    );
    assert_eq!(
        c(GeneralChange::Locale(LocaleSetting::Code("es".into()))),
        ("locale".into(), "\"es\"".into())
    );
    assert_eq!(
        c(GeneralChange::LaunchAtLogin(true)),
        ("launch_at_login".into(), "true".into())
    );
    assert_eq!(
        c(GeneralChange::DockIcon(DockIcon::WithWindow)),
        ("dock_icon".into(), "\"with_window\"".into())
    );
    assert_eq!(
        c(GeneralChange::DockIcon(DockIcon::Never)),
        ("dock_icon".into(), "\"never\"".into())
    );
    assert_eq!(
        c(GeneralChange::RestoreWindows(false)),
        ("windows.restore".into(), "false".into())
    );
    assert_eq!(
        c(GeneralChange::Clock(Clock::H12)),
        ("messages.clock".into(), "\"12h\"".into())
    );
    assert_eq!(
        c(GeneralChange::InlineImages(InlineImages::Never)),
        ("messages.inline_images".into(), "\"never\"".into())
    );
    assert_eq!(
        c(GeneralChange::UpdatesCheck(false)),
        ("updates.check".into(), "false".into())
    );
}

#[gpui::test]
fn changing_a_control_writes_the_key_through_the_edit_api(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    // Comments in the file survive an edit, as with every settings edit.
    std::fs::write(
        &paths.settings,
        "{\n  // my note\n  \"density\": \"cozy\"\n}\n",
    )
    .unwrap();
    let (_view, vcx) = open(cx, &paths);

    click(vcx, "settings-clock-12h");
    click(vcx, "settings-images-never");
    click(vcx, "settings-login");
    click(vcx, "settings-updates-auto");

    let text = std::fs::read_to_string(&paths.settings).unwrap();
    assert!(text.contains("// my note"), "{text}");
    let s = stored(&paths);
    assert_eq!(s.messages.clock, Clock::H12);
    assert_eq!(s.messages.inline_images, InlineImages::Never);
    assert!(s.launch_at_login);
    assert!(!s.updates.check, "the switch was on, so it turned off");
    assert_eq!(s.density, settings::Density::Cozy, "other keys stay");
}

#[gpui::test]
fn language_select_writes_the_locale(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open(cx, &paths);
    assert!(vcx.debug_bounds("settings-language-es").is_none(), "closed");
    click(vcx, "settings-language");
    click(vcx, "settings-language-es");
    assert_eq!(stored(&paths).locale, LocaleSetting::Code("es".into()));
    click(vcx, "settings-language");
    click(vcx, "settings-language-ca");
    assert_eq!(stored(&paths).locale, LocaleSetting::Code("ca".into()));
    click(vcx, "settings-language");
    click(vcx, "settings-language-system");
    assert_eq!(stored(&paths).locale, LocaleSetting::System);
}

fn language_open(vcx: &mut VisualTestContext) -> bool {
    vcx.debug_bounds("settings-language-list").is_some()
}

#[gpui::test]
fn language_select_floats_and_closes(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open(cx, &paths);
    let trigger = vcx.debug_bounds("settings-language").unwrap();
    click(vcx, "settings-language");
    assert!(language_open(vcx), "it opens");
    // A press on the page left of the select, over the section text.
    let outside = gpui::point(trigger.left() - gpui::px(400.), trigger.top());
    vcx.simulate_click(outside, Modifiers::none());
    vcx.run_until_parked();
    assert!(!language_open(vcx), "an outside click closes it");
    click(vcx, "settings-language");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!language_open(vcx), "Escape closes it");
    assert!(
        !paths.settings.exists() || stored(&paths).locale == LocaleSetting::System,
        "nothing picked"
    );
    click(vcx, "settings-language");
    click(vcx, "settings-language-ca");
    assert!(!language_open(vcx), "a pick closes it");
    assert_eq!(stored(&paths).locale, LocaleSetting::Code("ca".into()));
}

#[gpui::test]
fn keyboard_reaches_and_flips_a_switch(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open(cx, &paths);
    // Tab stops in order: header button, nav (11), language, login.
    vcx.simulate_keystrokes("tab");
    vcx.simulate_keystrokes(&"tab ".repeat(11));
    vcx.simulate_keystrokes("tab tab enter");
    assert!(stored(&paths).launch_at_login);
}

#[gpui::test]
fn the_page_follows_the_settings_store(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open(cx, &paths);
    let read = |vcx: &mut VisualTestContext| vcx.update(|_, cx| GeneralModel::read(cx));
    assert_eq!(read(vcx).clock, Clock::H24);
    std::fs::write(
        &paths.settings,
        r#"{"messages":{"clock":"12h"},"locale":"es"}"#,
    )
    .unwrap();
    vcx.update(|_, cx| {
        cx.global_mut::<SettingsStore>().reload();
    });
    let m = read(vcx);
    assert_eq!(m.clock, Clock::H12);
    assert_eq!(m.locale, LocaleSetting::Code("es".into()));
}

#[gpui::test]
fn nav_lists_every_page_and_unbuilt_pages_offer_the_file(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (view, vcx) = open(cx, &paths);
    for page in Page::ALL {
        let id: &'static str = Box::leak(format!("settings-nav-{}", page.id()).into_boxed_str());
        assert!(vcx.debug_bounds(id).is_some(), "{id} in the nav");
    }
    assert!(vcx.debug_bounds("settings-open-file").is_some());
    assert!(vcx.debug_bounds("settings-clock-12h").is_some());
    assert!(vcx.debug_bounds("settings-placeholder").is_none());

    click(vcx, "settings-nav-ai");
    assert_eq!(view.read_with(vcx, |v, _| v.page()), Page::Ai);
    assert!(vcx.debug_bounds("settings-placeholder").is_some());
    assert!(vcx.debug_bounds("settings-placeholder-open-file").is_some());
    assert!(vcx.debug_bounds("settings-clock-12h").is_none());

    click(vcx, "settings-nav-general");
    assert!(vcx.debug_bounds("settings-clock-12h").is_some());
}

#[gpui::test]
fn the_window_is_single_instance(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let path = paths.settings.clone();
    cx.update(|cx| {
        parla::settings_window::open_at(cx, path.clone(), "1.2.3");
        parla::settings_window::open_at(cx, path, "1.2.3");
    });
    let n = cx.update(|cx| cx.windows().len());
    assert_eq!(n, 1, "the second open focuses the first window");
}

// Appearance and Notifications pages.

#[test]
fn appearance_changes_name_their_key_and_json_value() {
    let c = |c: AppearanceChange| (c.key_path().join("."), c.raw_json());
    assert_eq!(
        c(AppearanceChange::ThemeMode(ThemeMode::Dark)),
        ("theme.mode".into(), "\"dark\"".into())
    );
    assert_eq!(
        c(AppearanceChange::ThemeLight("Parla Light".into())),
        ("theme.light".into(), "\"Parla Light\"".into())
    );
    assert_eq!(
        c(AppearanceChange::ThemeDark("My \"Dark\"".into())),
        ("theme.dark".into(), "\"My \\\"Dark\\\"\"".into())
    );
    assert_eq!(
        c(AppearanceChange::Density(Density::Cozy)),
        ("density".into(), "\"cozy\"".into())
    );
    assert_eq!(
        c(AppearanceChange::Accent(settings::AccentPreset::Coral)),
        ("accent_preset".into(), "\"coral\"".into())
    );
    assert_eq!(
        c(AppearanceChange::SendKey(settings::SendKey::CmdEnter)),
        ("send_key".into(), "\"cmd_enter\"".into())
    );
    assert_eq!(
        c(AppearanceChange::SidebarUnreads(false)),
        ("sidebar.show_unreads_section".into(), "false".into())
    );
    assert_eq!(
        c(AppearanceChange::SidebarSort(SidebarSort::Recent)),
        ("sidebar.sort".into(), "\"recent\"".into())
    );
}

#[test]
fn notification_changes_name_their_key_and_json_value() {
    let c = |c: NotificationsChange| (c.key_path().join("."), c.raw_json());
    assert_eq!(
        c(NotificationsChange::Desktop(DesktopNotifications::None)),
        ("notifications.desktop".into(), "\"none\"".into())
    );
    assert_eq!(
        c(NotificationsChange::Keywords(vec![
            "a".into(),
            "b c".into()
        ])),
        ("notifications.keywords".into(), "[\"a\",\"b c\"]".into())
    );
    assert_eq!(
        c(NotificationsChange::Sound("none".into())),
        ("notifications.sound".into(), "\"none\"".into())
    );
    assert_eq!(
        c(NotificationsChange::QuietHours(Some(QuietHours {
            from: "20:00".into(),
            to: "08:00".into(),
            days: vec![Weekday::Mon, Weekday::Sun],
        }))),
        (
            "notifications.quiet_hours".into(),
            "{\"from\":\"20:00\",\"to\":\"08:00\",\"days\":[\"mon\",\"sun\"]}".into()
        )
    );
    assert_eq!(
        c(NotificationsChange::ShowPreview(false)),
        ("notifications.show_preview".into(), "false".into())
    );
    assert_eq!(
        c(NotificationsChange::QuietHours(None)),
        ("notifications.quiet_hours".into(), "null".into())
    );
    assert_eq!(
        c(NotificationsChange::HuddleRing(false)),
        ("calls.ring".into(), "false".into())
    );
}

#[gpui::test]
fn appearance_controls_write_theme_density_and_sidebar(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Appearance);
    click(vcx, "settings-theme-mode-dark");
    click(vcx, "settings-density-cozy");
    click(vcx, "settings-sidebar-unreads");
    click(vcx, "settings-sidebar-sort-recent");
    let s = stored(&paths);
    assert_eq!(s.theme.mode, ThemeMode::Dark);
    assert_eq!(s.density, Density::Cozy);
    assert!(!s.sidebar.show_unreads_section, "the switch was on");
    assert_eq!(s.sidebar.sort, SidebarSort::Recent);
}

#[gpui::test]
fn accent_swatches_and_send_key_write_their_settings_and_apply_live(cx: &mut TestAppContext) {
    use themes::ActiveTheme as _;
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Appearance);
    let before = vcx.update(|_, cx| cx.theme().colors().element_background);
    click(vcx, "settings-accent-coral");
    click(vcx, "settings-send-key-cmd-enter");
    let s = stored(&paths);
    assert_eq!(s.accent_preset, settings::AccentPreset::Coral);
    assert_eq!(s.send_key, settings::SendKey::CmdEnter);
    reload(vcx);
    // The app re-applies the theme when settings change (`app_state`).
    let after = vcx.update(|_, cx| {
        let s = cx.global::<SettingsStore>().current().clone();
        themes::apply(cx, &s);
        cx.theme().colors().element_background
    });
    assert_ne!(before, after, "the accent applied without a restart");
}

#[gpui::test]
fn theme_cards_come_from_the_registry_and_pick_the_theme(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    // A user theme in the registry gets its own card.
    cx.update(|cx| {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("one.json"),
            include_str!("../../themes/tests/fixtures/zed-format-sample.json"),
        )
        .unwrap();
        themes::load_user_themes(&themes::ThemeRegistry::global(cx), dir.path());
    });
    let (_view, vcx) = open_on(cx, &paths, Page::Appearance);
    for id in [
        "settings-theme-parla-light",
        "settings-theme-parla-dark",
        "settings-theme-sample-light",
        "settings-theme-sample-dark",
    ] {
        assert!(vcx.debug_bounds(id).is_some(), "{id} is a card");
    }
    // The mode is System: the pick sets the theme of its appearance only.
    click(vcx, "settings-theme-sample-dark");
    let s = stored(&paths);
    assert_eq!(s.theme.dark, "Sample Dark");
    assert_eq!(s.theme.light, "Parla Light");
    assert_eq!(s.theme.mode, ThemeMode::System);
    // With a pinned mode the other appearance's card pins its mode too.
    click(vcx, "settings-theme-mode-light");
    reload(vcx);
    click(vcx, "settings-theme-parla-dark");
    let s = stored(&paths);
    assert_eq!(s.theme.dark, "Parla Dark");
    assert_eq!(s.theme.mode, ThemeMode::Dark);
}

#[gpui::test]
fn notification_controls_write_their_keys(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Notifications);
    click(vcx, "settings-desktop-all");
    click(vcx, "settings-sound");
    click(vcx, "settings-huddle-ring");
    click(vcx, "settings-quiet");
    let s = stored(&paths);
    assert_eq!(s.notifications.desktop, DesktopNotifications::All);
    assert_eq!(s.notifications.sound, "none", "the sound was on");
    assert!(!s.calls.ring, "the ring was on");
    let q = s.notifications.quiet_hours.expect("quiet hours on");
    assert_eq!((q.from.as_str(), q.to.as_str()), ("20:00", "08:00"));
    // The fields appear with quiet hours on; the switch turns them off again.
    assert!(vcx.debug_bounds("settings-quiet-from").is_some());
    click(vcx, "settings-quiet");
    assert_eq!(stored(&paths).notifications.quiet_hours, None);
    assert!(vcx.debug_bounds("settings-quiet-from").is_none());
}

#[gpui::test]
fn keywords_are_added_with_enter_and_removed_with_their_chip(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    std::fs::write(
        &paths.settings,
        r#"{"notifications":{"keywords":["release","tokens"]}}"#,
    )
    .unwrap();
    vcx_reload(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Notifications);
    assert!(vcx.debug_bounds("settings-keyword-1").is_some());
    click(vcx, "settings-keyword-input");
    vcx.simulate_input("design review");
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        stored(&paths).notifications.keywords,
        ["release", "tokens", "design review"]
    );
    // A duplicate is not added twice (case aside).
    reload(vcx);
    vcx.simulate_input("RELEASE");
    vcx.simulate_keystrokes("enter");
    assert_eq!(stored(&paths).notifications.keywords.len(), 3);
    click(vcx, "settings-keyword-0");
    assert_eq!(
        stored(&paths).notifications.keywords,
        ["tokens", "design review"]
    );
}

fn vcx_reload(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.global_mut::<SettingsStore>().reload();
    });
}

#[gpui::test]
fn quiet_hours_fields_write_a_valid_time_only(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    std::fs::write(
        &paths.settings,
        r#"{"notifications":{"quiet_hours":{"from":"20:00","to":"08:00"}}}"#,
    )
    .unwrap();
    vcx_reload(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Notifications);
    click(vcx, "settings-quiet-from");
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("9");
    assert_eq!(
        stored(&paths).notifications.quiet_hours.unwrap().from,
        "20:00",
        "9 is not a time yet"
    );
    vcx.simulate_input(":30");
    let q = stored(&paths).notifications.quiet_hours.unwrap();
    assert_eq!((q.from.as_str(), q.to.as_str()), ("09:30", "08:00"));
}

#[gpui::test]
fn a_quick_double_click_toggles_back_before_the_reload(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open(cx, &paths);
    // No reload between the clicks: the second one reads what the first wrote.
    click(vcx, "settings-login");
    click(vcx, "settings-login");
    reload(vcx);
    assert!(!stored(&paths).launch_at_login, "on, then off again");
    click(vcx, "settings-updates-auto");
    click(vcx, "settings-updates-auto");
    click(vcx, "settings-updates-auto");
    reload(vcx);
    assert!(!stored(&paths).updates.check, "three flips from on end off");
}

#[gpui::test]
fn the_quick_double_click_holds_on_every_page(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (view, vcx) = open_on(cx, &paths, Page::Notifications);
    click(vcx, "settings-sound");
    click(vcx, "settings-sound");
    assert_eq!(stored(&paths).notifications.sound, "subtle");
    click(vcx, "settings-nav-appearance");
    click(vcx, "settings-sidebar-unreads");
    click(vcx, "settings-sidebar-unreads");
    assert!(stored(&paths).sidebar.show_unreads_section);
    assert_eq!(view.read_with(vcx, |v, _| v.page()), Page::Appearance);
}

#[gpui::test]
fn a_settled_reload_drops_the_pending_value_so_file_edits_show(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open(cx, &paths);
    click(vcx, "settings-login");
    reload(vcx);
    // The file is edited by hand after the reload: the page shows the file.
    std::fs::write(&paths.settings, r#"{"launch_at_login":false}"#).unwrap();
    reload(vcx);
    let m = vcx.update(|_, cx| GeneralModel::read(cx));
    assert!(!m.launch_at_login);
    click(vcx, "settings-login");
    assert!(stored(&paths).launch_at_login, "the switch showed off");
}

#[gpui::test]
fn pages_follow_the_settings_store_live(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (view, vcx) = open_on(cx, &paths, Page::Notifications);
    std::fs::write(
        &paths.settings,
        r#"{"density":"cozy","sidebar":{"sort":"recent"},
            "notifications":{"desktop":"none","keywords":["a"],"sound":"none"},
            "calls":{"ring":false}}"#,
    )
    .unwrap();
    reload(vcx);
    // The app refreshes its windows after a reload (`refresh_windows`).
    view.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    let a = vcx.update(|_, cx| AppearanceModel::read(cx));
    assert_eq!(a.density, Density::Cozy);
    assert_eq!(a.sort, SidebarSort::Recent);
    let n = vcx.update(|_, cx| NotificationsModel::read(cx));
    assert_eq!(n.desktop, DesktopNotifications::None);
    assert_eq!(n.keywords, ["a"]);
    assert!(!n.sound_on);
    assert!(!n.huddle_ring);
    assert!(vcx.debug_bounds("settings-keyword-0").is_some());
}

#[gpui::test]
fn a_failed_write_shows_an_error_notice(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    // Not an object: edit_file answers Ok(false).
    std::fs::write(&paths.settings, "[1, 2]").unwrap();
    let (view, vcx) = open(cx, &paths);
    assert!(vcx.debug_bounds("settings-error").is_none());
    click(vcx, "settings-clock-12h");
    assert!(vcx.debug_bounds("settings-error").is_some());
    let text = view.read_with(vcx, |v, _| v.error().map(str::to_string));
    assert!(text.unwrap().contains("not a JSON object"));
    assert_eq!(
        std::fs::read_to_string(&paths.settings).unwrap(),
        "[1, 2]",
        "nothing was written"
    );
    // Leaving the page clears the notice.
    click(vcx, "settings-nav-appearance");
    assert!(vcx.debug_bounds("settings-error").is_none());
}

#[gpui::test]
fn an_unwritable_file_shows_the_io_error(cx: &mut TestAppContext) {
    let (dir, _paths) = setup(cx);
    // The settings path is a folder: reading and writing it fail.
    let folder = dir.path().join("as-folder");
    std::fs::create_dir(&folder).unwrap();
    let f = folder.clone();
    let (view, vcx) = cx.add_window_view(move |_, _| {
        parla::settings_window::view_on(f, "1.2.3".into(), Page::Appearance)
    });
    vcx.run_until_parked();
    click(vcx, "settings-density-cozy");
    assert!(vcx.debug_bounds("settings-error").is_some());
    let text = view.read_with(vcx, |v, _| v.error().map(str::to_string));
    assert!(text.unwrap().starts_with("Could not save settings.json"));
}

#[gpui::test]
fn a_type_invalid_file_shows_its_error_and_drops_the_pending_edit(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    std::fs::write(&paths.settings, r#"{"density":5}"#).unwrap();
    vcx_reload(cx);
    let (view, vcx) = open(cx, &paths);
    click(vcx, "settings-login");
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("settings-error").is_some(),
        "the store error is the page notice"
    );
    let shown = view.read_with(vcx, |v, cx| v.live_settings(cx).launch_at_login);
    assert!(!shown, "the rejected file never reloaded: no overlay");
    let text = view.read_with(vcx, |v, cx| v.notice_text(cx));
    assert!(text.unwrap().contains("density"));
}

#[test]
fn both_configured_themes_are_selected_in_system_mode() {
    use parla::settings_window::theme_card_selected;
    let (l, d) = ("Parla Light", "Parla Dark");
    assert!(theme_card_selected(ThemeMode::System, l, d, "Parla Light"));
    assert!(theme_card_selected(ThemeMode::System, l, d, "Parla Dark"));
    assert!(!theme_card_selected(ThemeMode::System, l, d, "Other"));
    assert!(theme_card_selected(ThemeMode::Dark, l, d, "Parla Dark"));
    assert!(!theme_card_selected(ThemeMode::Dark, l, d, "Parla Light"));
    assert!(theme_card_selected(ThemeMode::Light, l, d, "Parla Light"));
}

#[gpui::test]
fn an_invalid_time_shows_a_text_cue(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    std::fs::write(
        &paths.settings,
        r#"{"notifications":{"quiet_hours":{"from":"20:00","to":"08:00"}}}"#,
    )
    .unwrap();
    vcx_reload(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Notifications);
    assert!(vcx.debug_bounds("settings-quiet-from-hint").is_none());
    click(vcx, "settings-quiet-from");
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("99");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("settings-quiet-from-hint").is_some());
}

#[gpui::test]
fn keywords_dedupe_with_case_and_padding(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    std::fs::write(
        &paths.settings,
        r#"{"notifications":{"keywords":["Release","release "," Tokens"]}}"#,
    )
    .unwrap();
    vcx_reload(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Notifications);
    click(vcx, "settings-keyword-input");
    vcx.simulate_input("TOKENS");
    vcx.simulate_keystrokes("enter");
    assert_eq!(stored(&paths).notifications.keywords.len(), 3, "not added");
}

fn quit_row(vcx: &mut VisualTestContext) -> parla::shortcuts::Shortcut {
    vcx.update(|_, cx| {
        parla::shortcuts::current(cx)
            .into_iter()
            .find(|r| r.action == "app::Quit")
            .unwrap()
    })
}

#[gpui::test]
fn keymap_page_lists_searches_records_and_resets(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (view, vcx) = open_on(cx, &paths, Page::Keymap);
    assert!(vcx.debug_bounds("settings-placeholder").is_none());
    assert!(vcx.debug_bounds("settings-keymap-table").is_some());
    assert!(vcx.debug_bounds("settings-keymap-open-file").is_some());
    assert!(
        vcx.debug_bounds("settings-keymap-edit-app::OpenSettings")
            .is_some()
    );

    view.update(vcx, |v, cx| v.set_keymap_query("quit", cx));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("settings-keymap-edit-app::Quit").is_some());
    assert!(
        vcx.debug_bounds("settings-keymap-edit-app::OpenSettings")
            .is_none()
    );
    let default_keys = quit_row(vcx).keystrokes.unwrap();

    click(vcx, "settings-keymap-edit-app::Quit");
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.keymap_recording()),
        Some(("app::Quit".to_string(), None))
    );
    vcx.simulate_keystrokes("alt-shift-q");
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.keymap_recording()),
        Some(("app::Quit".to_string(), Some("alt-shift-q".to_string())))
    );
    click(vcx, "settings-keymap-save");
    vcx.run_until_parked();
    let text = std::fs::read_to_string(&paths.keymap).unwrap();
    assert!(text.contains("\"alt-shift-q\": \"app::Quit\""), "{text}");
    assert!(
        text.contains(&format!("\"{default_keys}\": null")),
        "{text}"
    );
    let row = quit_row(vcx);
    assert_eq!(row.keystrokes.as_deref(), Some("alt-shift-q"));
    assert_eq!(row.source, parla::shortcuts::Source::User);
    assert!(view.read_with(vcx, |v, _| v.keymap_recording()).is_none());

    click(vcx, "settings-keymap-reset-app::Quit");
    vcx.run_until_parked();
    let row = quit_row(vcx);
    assert_eq!(row.keystrokes.as_deref(), Some(default_keys.as_str()));
    assert_eq!(row.source, parla::shortcuts::Source::Default);
}

#[gpui::test]
fn recording_keys_of_another_action_warns_and_escape_cancels(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (view, vcx) = open_on(cx, &paths, Page::Keymap);
    let palette = vcx.update(|_, cx| {
        parla::shortcuts::current(cx)
            .into_iter()
            .find(|r| r.action == "app::CommandPalette")
            .and_then(|r| r.keystrokes)
            .unwrap()
    });
    view.update(vcx, |v, cx| v.set_keymap_query("quit", cx));
    vcx.run_until_parked();
    click(vcx, "settings-keymap-edit-app::Quit");
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.keymap_recording()).is_some());
    assert!(vcx.debug_bounds("settings-keymap-conflict").is_none());
    vcx.simulate_keystrokes(&palette);
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("settings-keymap-conflict").is_some(),
        "{palette}: {:?}",
        view.read_with(vcx, |v, _| v.keymap_recording())
    );
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.keymap_recording()).is_none());
    assert!(
        !paths.keymap.exists() || std::fs::read_to_string(&paths.keymap).unwrap().trim() == "[]"
    );
}

fn record_quit(view: &gpui::Entity<SettingsView>, vcx: &mut VisualTestContext) {
    view.update(vcx, |v, cx| v.set_keymap_query("quit", cx));
    vcx.run_until_parked();
    click(vcx, "settings-keymap-edit-app::Quit");
    vcx.run_until_parked();
}

#[gpui::test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "records cmd; GPUI names that key super off macOS"
)]
fn enter_saves_a_recording_and_two_chords_make_a_sequence(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (view, vcx) = open_on(cx, &paths, Page::Keymap);
    record_quit(&view, vcx);
    vcx.simulate_keystrokes("cmd-k cmd-q");
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.keymap_recording()),
        Some(("app::Quit".to_string(), Some("cmd-k cmd-q".to_string())))
    );
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.keymap_recording()).is_none());
    assert_eq!(quit_row(vcx).keystrokes.as_deref(), Some("cmd-k cmd-q"));
}

#[gpui::test]
fn a_plain_typing_key_for_a_global_action_is_refused(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (view, vcx) = open_on(cx, &paths, Page::Keymap);
    record_quit(&view, vcx);
    vcx.simulate_keystrokes("q");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("settings-keymap-needs-modifier").is_some());
    assert!(vcx.debug_bounds("settings-keymap-save").is_none());
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(
        view.read_with(vcx, |v, _| v.keymap_recording()).is_some(),
        "not saved"
    );
    assert!(
        !paths.keymap.exists()
            || !std::fs::read_to_string(&paths.keymap)
                .unwrap()
                .contains("app::Quit")
    );
}

#[gpui::test]
fn changed_lists_unbound_defaults_and_conflicts_has_a_count(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let quit_keys = cx.update(|cx| {
        parla::shortcuts::current(cx)
            .into_iter()
            .find(|r| r.action == "app::Quit")
            .and_then(|r| r.keystrokes)
            .unwrap()
    });
    std::fs::write(
        &paths.keymap,
        format!(
            r#"[{{ "bindings": {{ "{quit_keys}": null }} }},
               {{ "context": "Timeline", "bindings": {{ "{quit_keys}": "app::Hide" }} }}]"#
        ),
    )
    .unwrap();
    let (_view, vcx) = open_on(cx, &paths, Page::Keymap);
    assert!(
        vcx.debug_bounds("settings-keymap-filter-conflicts")
            .is_some()
    );
    click(vcx, "settings-keymap-filter-changed");
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("settings-keymap-reset-app::Quit")
            .is_some()
    );
    assert!(
        vcx.debug_bounds("settings-keymap-edit-app::OpenSettings")
            .is_none()
    );
    click(vcx, "settings-keymap-reset-app::Quit");
    vcx.run_until_parked();
    assert_eq!(
        quit_row(vcx).keystrokes.as_deref(),
        Some(quit_keys.as_str())
    );
}

#[gpui::test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "the Dock and menu bar settings exist on macOS only"
)]
fn restore_windows_and_menu_bar_only_write_their_keys(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open(cx, &paths);
    click(vcx, "settings-restore-windows");
    click(vcx, "settings-dock-never");
    let s = stored(&paths);
    assert!(!s.windows.restore, "the switch was on, so it turned off");
    assert_eq!(s.dock_icon, DockIcon::Never);
}

#[gpui::test]
fn preview_switch_and_quiet_weekdays_write_their_keys(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Notifications);
    click(vcx, "settings-preview");
    assert!(!stored(&paths).notifications.show_preview);
    // The day chips come with quiet hours; with none picked every day is lit.
    assert!(vcx.debug_bounds("settings-quiet-day-fri").is_none());
    click(vcx, "settings-quiet");
    assert!(vcx.debug_bounds("settings-quiet-day-fri").is_some());
    click(vcx, "settings-quiet-day-sat");
    click(vcx, "settings-quiet-day-sun");
    let q = stored(&paths).notifications.quiet_hours.expect("on");
    assert_eq!(
        q.days,
        vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri
        ]
    );
}

#[test]
fn toggling_a_quiet_day_keeps_a_sorted_list_and_never_empties_it() {
    use parla::settings_window::toggle_day;
    use settings::Weekday::*;
    // None picked means every day: the first click leaves the other six.
    assert_eq!(toggle_day(&[], Wed), vec![Mon, Tue, Thu, Fri, Sat, Sun]);
    // Adding keeps week order; all seven is the empty list again.
    assert_eq!(toggle_day(&[Fri, Mon], Wed), vec![Mon, Wed, Fri]);
    assert_eq!(toggle_day(&[Mon, Tue, Wed, Thu, Fri, Sat], Sun), Vec::new());
    // The last day stays.
    assert_eq!(toggle_day(&[Fri], Fri), vec![Fri]);
}

/// Under the built-in high contrast theme the accent presets are shown
/// disabled with a hint, and a click changes nothing.
#[gpui::test]
fn accent_presets_are_disabled_under_high_contrast(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    cx.update(|cx| {
        themes::set_increase_contrast(cx, true);
        let s = cx.global::<SettingsStore>().current().clone();
        themes::apply(cx, &s);
        assert!(!themes::accent_presets_apply(cx));
    });
    let (_view, vcx) = open_on(cx, &paths, Page::Appearance);
    assert!(
        vcx.debug_bounds("settings-accent-high-contrast").is_some(),
        "the hint is drawn"
    );
    click(vcx, "settings-accent-coral");
    assert_eq!(stored(&paths).accent_preset, settings::AccentPreset::Brand);
}
