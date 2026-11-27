use std::{fs, sync::mpsc, time::Duration};

use settings::*;
use tempfile::TempDir;

fn setup() -> (TempDir, ConfigPaths) {
    let dir = TempDir::new().unwrap();
    let paths = ConfigPaths::at(dir.path());
    (dir, paths)
}

#[test]
fn first_run_creates_template() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    assert!(
        fs::read_to_string(&paths.settings)
            .unwrap()
            .starts_with("//")
    );
    assert_eq!(fs::read_to_string(&paths.keymap).unwrap(), "[]");
    assert!(paths.themes_dir.is_dir() && paths.workspaces_dir.is_dir());
    let store = SettingsStore::load(paths);
    // Only the editor pointer differs from the defaults.
    let mut loaded = store.current().clone();
    assert_eq!(
        loaded.schema.take().as_deref(),
        Some("./settings.schema.json")
    );
    assert_eq!(loaded, Settings::default());
    assert!(store.last_error().is_none());
}

#[test]
fn first_run_points_settings_at_the_shipped_schema() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    let text = fs::read_to_string(&paths.settings).unwrap();
    assert!(
        text.contains(r#""$schema": "./settings.schema.json""#),
        "{text}"
    );
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.schema).unwrap()).unwrap();
    assert_eq!(schema, settings_schema());
    // The template loads with no warnings about the pointer.
    assert!(unknown_keys(&text).is_empty(), "{:?}", unknown_keys(&text));
    // A stale schema is refreshed; the user's settings file is not touched.
    fs::write(&paths.schema, "{}").unwrap();
    paths.ensure_exists().unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&paths.schema).unwrap()).unwrap();
    assert_eq!(schema, settings_schema());
}

#[test]
fn ensure_exists_keeps_existing_files() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::write(&paths.settings, "{\"ui_font_size\":16}").unwrap();
    paths.ensure_exists().unwrap();
    assert_eq!(
        fs::read_to_string(&paths.settings).unwrap(),
        "{\"ui_font_size\":16}"
    );
}

#[test]
fn invalid_edit_keeps_last_good() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::write(&paths.settings, "{\"ui_font_size\":16}").unwrap();
    let mut store = SettingsStore::load(paths.clone());
    assert_eq!(store.current().ui_font_size, 16.0);
    fs::write(&paths.settings, "{\"ui_font_size\": 18,").unwrap();
    assert!(matches!(
        store.reload(),
        ReloadOutcome::Rejected(SettingsError::Parse { .. })
    ));
    assert_eq!(store.current().ui_font_size, 16.0);
    assert!(store.last_error().is_some());
}

#[test]
fn fixing_file_clears_error() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    let mut store = SettingsStore::load(paths.clone());
    fs::write(&paths.settings, "{\"ui_font_size\": 18,").unwrap();
    assert!(matches!(store.reload(), ReloadOutcome::Rejected(_)));
    fs::write(&paths.settings, "{\"ui_font_size\": 18}").unwrap();
    assert_eq!(store.reload(), ReloadOutcome::Changed);
    assert_eq!(store.current().ui_font_size, 18.0);
    assert!(store.last_error().is_none());
    assert_eq!(store.reload(), ReloadOutcome::Unchanged);
}

#[test]
fn per_workspace_file_applies() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::create_dir_all(paths.workspaces_dir.join("fake")).unwrap();
    fs::write(
        paths.workspaces_dir.join("fake").join("W1.json"),
        "{\"density\":\"cozy\"}",
    )
    .unwrap();
    let store = SettingsStore::load(paths);
    assert_eq!(
        store
            .for_workspace(&WorkspaceKey::new("fake", "W1"))
            .density,
        Density::Cozy
    );
    assert_eq!(
        store
            .for_workspace(&WorkspaceKey::new("fake", "W2"))
            .density,
        Density::Compact
    );
    assert_eq!(store.current().density, Density::Compact);
}

#[test]
fn deleted_settings_file_means_defaults() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::write(&paths.settings, "{\"ui_font_size\":16}").unwrap();
    let mut store = SettingsStore::load(paths.clone());
    fs::remove_file(&paths.settings).unwrap();
    assert_eq!(store.reload(), ReloadOutcome::Changed);
    assert_eq!(*store.current(), Settings::default());
    assert!(store.last_error().is_none());
}

fn recv_until(rx: &mpsc::Receiver<ChangedFile>, want: &ChangedFile) -> bool {
    let end = std::time::Instant::now() + Duration::from_secs(2);
    while let Some(left) = end.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(c) if &c == want => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

#[test]
fn watcher_reloads_after_atomic_rename() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    let (tx, rx) = mpsc::channel();
    let _h = watch(&paths, move |c| {
        let _ = tx.send(c);
    })
    .unwrap();
    let tmp = paths.root.join("settings.json.tmp");
    fs::write(&tmp, "{\"ui_font_size\":20}").unwrap();
    fs::rename(&tmp, &paths.settings).unwrap();
    assert!(recv_until(&rx, &ChangedFile::Settings));
}

#[test]
fn watcher_reports_theme_file() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    let (tx, rx) = mpsc::channel();
    let _h = watch(&paths, move |c| {
        let _ = tx.send(c);
    })
    .unwrap();
    let theme = paths.themes_dir.join("x.json");
    fs::write(&theme, "{}").unwrap();
    assert!(recv_until(&rx, &ChangedFile::Theme(theme)));
}

#[test]
fn workspace_key_text_form_and_file_path() {
    let key: WorkspaceKey = "fake/W1".parse().unwrap();
    assert_eq!(key, WorkspaceKey::new("fake", "W1"));
    assert_eq!(key.to_string(), "fake/W1");
    for bad in ["", "fake", "/W1", "fake/", "fake/a/b", "fake/a\\b"] {
        assert!(bad.parse::<WorkspaceKey>().is_err(), "{bad}");
    }
    let (_d, paths) = setup();
    assert_eq!(
        paths.workspace_file(&key),
        paths.workspaces_dir.join("fake").join("W1.json")
    );
    // Parts that could leave the directory stay inside it.
    let sneaky = WorkspaceKey::new("..", "../x");
    assert!(
        paths
            .workspace_file(&sneaky)
            .starts_with(&paths.workspaces_dir)
    );
    let rel = paths.workspace_file(&sneaky);
    let rel = rel.strip_prefix(&paths.workspaces_dir).unwrap();
    assert!(
        rel.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
    );
}

#[test]
fn locale_defaults_to_system() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    let store = SettingsStore::load(paths);
    assert_eq!(store.current().locale, LocaleSetting::System);
    assert_eq!(store.current().locale.pref(), i18n::LocalePref::System);
}

#[test]
fn listed_locale_is_accepted() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::write(&paths.settings, "{\"locale\":\"ca\"}").unwrap();
    let store = SettingsStore::load(paths);
    assert_eq!(
        store.current().locale.pref(),
        i18n::LocalePref::Code("ca".parse().unwrap())
    );
}

#[test]
fn unknown_locale_keeps_last_good() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::write(&paths.settings, "{\"locale\":\"es\"}").unwrap();
    let mut store = SettingsStore::load(paths.clone());
    fs::write(&paths.settings, "{\"locale\":\"xx\"}").unwrap();
    match store.reload() {
        ReloadOutcome::Rejected(SettingsError::Type { pointer, .. }) => {
            assert_eq!(pointer, "/locale")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        store.current().locale.pref(),
        i18n::LocalePref::Code("es".parse().unwrap())
    );
}

#[test]
fn pseudo_locale_accepted() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::write(&paths.settings, "{\"locale\":\"en-XA\"}").unwrap();
    let store = SettingsStore::load(paths);
    assert!(store.last_error().is_none());
    assert_eq!(
        store.current().locale.pref(),
        i18n::LocalePref::Code("en-XA".parse().unwrap())
    );
}

#[test]
fn for_workspace_is_cached_until_the_generation_moves() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::create_dir_all(paths.workspaces_dir.join("fake")).unwrap();
    let file = paths.workspaces_dir.join("fake").join("W1.json");
    fs::write(&file, "{\"density\":\"cozy\"}").unwrap();
    let mut store = SettingsStore::load(paths);
    let key = WorkspaceKey::new("fake", "W1");
    assert_eq!(store.for_workspace(&key).density, Density::Cozy);
    // No disk read on the second call: the edit is not seen yet.
    fs::write(&file, "{\"density\":\"compact\"}").unwrap();
    assert_eq!(store.for_workspace(&key).density, Density::Cozy);
    store.note_workspace_changed();
    assert_eq!(store.for_workspace(&key).density, Density::Compact);
}

#[test]
fn a_removed_transport_key_loads_with_a_warning() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::write(
        &paths.settings,
        r#"{ "mcp": { "enabled": true, "transport": "http" } }"#,
    )
    .unwrap();
    let store = SettingsStore::load(paths);
    assert!(store.last_error().is_none(), "{:?}", store.last_error());
    assert!(store.current().mcp.enabled);
    assert_eq!(store.warnings(), ["/mcp/transport"]);
}

#[test]
fn stale_keys_are_removed_from_the_file_once() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    fs::write(
        &paths.settings,
        "{ // keep\n  \"mcp\": { \"enabled\": true, \"transport\": \"http\" } }",
    )
    .unwrap();
    let mut store = SettingsStore::load(paths.clone());
    assert_eq!(store.remove_stale_keys(), ["/mcp/transport"]);
    let text = fs::read_to_string(&paths.settings).unwrap();
    assert!(
        text.contains("// keep") && !text.contains("transport"),
        "{text}"
    );
    assert!(store.warnings().is_empty());
    assert!(store.current().mcp.enabled);
    assert!(store.remove_stale_keys().is_empty());
}

#[cfg(unix)]
#[test]
fn a_read_only_folder_keeps_the_key_and_loads() {
    use std::os::unix::fs::PermissionsExt as _;
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    let text = r#"{ "mcp": { "enabled": true, "transport": "http" } }"#;
    fs::write(&paths.settings, text).unwrap();
    let dir = paths.settings.parent().unwrap().to_path_buf();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    // Root ignores directory permissions: nothing to prove then.
    let probe = dir.join("probe");
    let is_root = fs::write(&probe, "x").is_ok();
    let _ = fs::remove_file(&probe);
    let mut store = SettingsStore::load(paths.clone());
    let removed = store.remove_stale_keys();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    if is_root {
        return;
    }
    assert!(removed.is_empty());
    assert_eq!(fs::read_to_string(&paths.settings).unwrap(), text);
    assert!(store.current().mcp.enabled);
    assert!(store.last_error().is_none());
}

#[test]
fn a_clean_file_has_no_warnings() {
    let (_d, paths) = setup();
    paths.ensure_exists().unwrap();
    assert!(SettingsStore::load(paths).warnings().is_empty());
}
