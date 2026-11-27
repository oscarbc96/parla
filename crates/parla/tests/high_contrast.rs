//! The OS "Increase contrast" switch swaps the brand themes for Parla High
//! Contrast live, and back when it goes off.

use gpui::TestAppContext;
use settings::ConfigPaths;
use themes::ActiveTheme as _;

#[gpui::test]
fn increase_contrast_change_swaps_the_active_theme_live(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(
        dir.path().join("settings.json"),
        r#"{ "theme": { "mode": "dark" } }"#,
    )
    .unwrap();
    cx.update(|cx| {
        parla::init_without_watcher(cx, ConfigPaths::at(dir.path()));
        assert_eq!(cx.theme().name.as_ref(), themes::DEFAULT_DARK);
        parla::increase_contrast_changed(cx, true);
        assert_eq!(cx.theme().name.as_ref(), themes::HIGH_CONTRAST_DARK);
        // The same value again changes nothing.
        parla::increase_contrast_changed(cx, true);
        assert_eq!(cx.theme().name.as_ref(), themes::HIGH_CONTRAST_DARK);
        parla::increase_contrast_changed(cx, false);
        assert_eq!(cx.theme().name.as_ref(), themes::DEFAULT_DARK);
    });
}

/// A theme the user set in settings.json, even Parla Dark, wins over the switch;
/// removing it from the file hands the slot back to high contrast.
#[gpui::test]
fn an_explicit_theme_in_settings_wins_until_it_is_removed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.json");
    std::fs::write(
        &file,
        r#"{ "theme": { "mode": "dark", "dark": "Parla Dark" } }"#,
    )
    .unwrap();
    cx.update(|cx| {
        parla::init_without_watcher(cx, ConfigPaths::at(dir.path()));
        parla::increase_contrast_changed(cx, true);
        assert_eq!(cx.theme().name.as_ref(), themes::DEFAULT_DARK);
        assert!(themes::explicit_themes(cx).dark);
        assert!(!themes::explicit_themes(cx).light);
    });
    std::fs::write(&file, r#"{ "theme": { "mode": "dark" } }"#).unwrap();
    cx.update(|cx| {
        parla::handle_change(cx, settings::ChangedFile::Settings);
        assert_eq!(cx.theme().name.as_ref(), themes::HIGH_CONTRAST_DARK);
    });
}
