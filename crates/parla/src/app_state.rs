//! Globals and reload wiring: settings, themes and keymap hot reload.

use futures::StreamExt as _;
use gpui::{App, Global, KeyBinding};
use settings::{
    ChangedFile, ConfigPaths, KeymapBlock, ReloadOutcome, SettingsStore, WatchHandle, parse_keymap,
    resolve_bindings,
};
use themes::{ActiveTheme as _, Appearance, Theme, ThemeRegistry};

use crate::actions::{
    self, OpenKeymap, OpenSettings, OpenSettingsFile, Quit, ReloadConfig, ToggleMode,
};
use crate::toast::{ConfigBanner, WarningSource};

use crate::shortcuts::DEFAULT_KEYMAP;

/// Config paths plus the live watcher; one per app.
struct AppState {
    paths: ConfigPaths,
    _watch: Option<WatchHandle>,
}

impl Global for AppState {}

/// The `settings.json` path the app was initialised with (falls back to the
/// discovered path before `init`).
pub fn settings_path(cx: &App) -> std::path::PathBuf {
    match cx.try_global::<AppState>() {
        Some(state) => state.paths.settings.clone(),
        None => ConfigPaths::discover().settings,
    }
}

/// The config paths the app was initialised with; `None` before `init` (and
/// in tests without it), so nothing writes to the discovered folder then.
pub fn config_paths(cx: &App) -> Option<ConfigPaths> {
    cx.try_global::<AppState>().map(|s| s.paths.clone())
}

/// The currently active theme.
pub fn active_theme(cx: &App) -> &Theme {
    cx.theme()
}

/// Sets up globals, binds the keymap, registers actions and starts the config watcher.
pub fn init(cx: &mut App, paths: ConfigPaths) {
    init_with(cx, paths, true);
}

/// Like [`init`] but never starts the OS file watcher. Tests drive reloads
/// through [`handle_change`]; a real watcher thread would touch the GPUI test
/// executor and trip its scheduler checks.
pub fn init_without_watcher(cx: &mut App, paths: ConfigPaths) {
    init_with(cx, paths, false);
}

fn init_with(cx: &mut App, paths: ConfigPaths, watch_files: bool) {
    let _ = paths.ensure_exists();
    let store = SettingsStore::load(paths.clone());
    // Core catalogs join this list as their crates gain one; providers add theirs
    // through `ProviderRegistry::register_catalogs`.
    i18n::gpui::init(
        cx,
        &[components::catalog(), crate::catalog()],
        store.current().locale.pref(),
    );
    // The real app follows the OS "Increase contrast" switch; tests (no
    // watcher) stay independent of the machine they run on.
    if watch_files {
        themes::set_increase_contrast(cx, crate::platform::increase_contrast());
    }
    sync_explicit_themes(cx, &paths);
    themes::init(cx, store.current());
    cx.set_global(store);
    components::live_settings::init_ui_font(cx);
    // Real clock and live motion unless a harness installed its own first.
    if !cx.has_global::<components::NowGlobal>() {
        components::set_now(cx, std::sync::Arc::new(domain::SystemNow));
    }
    if !cx.has_global::<components::Motion>() {
        // Spec section 9: nothing animates with the OS's reduce-motion on.
        let motion = if crate::platform::reduce_motion() {
            components::Motion::Frozen
        } else {
            components::Motion::Live
        };
        components::set_motion(cx, motion);
    }
    cx.set_global(ConfigBanner::default());

    let registry = ThemeRegistry::global(cx);
    let theme_errors = themes::load_user_themes(&registry, &paths.themes_dir);
    let mut warnings: Vec<String> = theme_errors
        .iter()
        .map(|e| format!("{}: {}", e.path.display(), e.message))
        .collect();
    let current = cx.global::<SettingsStore>().current().clone();
    if let Some(w) = themes::apply(cx, &current) {
        warnings.push(w.to_string());
    }
    let error = cx.global::<SettingsStore>().last_error().cloned();
    {
        let banner = cx.global_mut::<ConfigBanner>();
        banner.set_error(error);
        banner.set_warnings(WarningSource::Theme, warnings);
    }
    bind_keymap(cx, &paths);
    register_actions(cx);

    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let watch = if watch_files {
        settings::watch(&paths, move |file| {
            let _ = tx.unbounded_send(file);
        })
        .ok()
    } else {
        None
    };
    cx.set_global(AppState {
        paths,
        _watch: watch,
    });
    cx.spawn(async move |cx| {
        while let Some(file) = rx.next().await {
            cx.update(|cx| handle_change(cx, file));
        }
    })
    .detach();
    if watch_files {
        watch_increase_contrast(cx);
    }
}

/// Re-reads "Increase contrast" each time the OS posts an accessibility
/// display change (no polling) and swaps the theme when it moved.
fn watch_increase_contrast(cx: &mut App) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
    crate::platform::on_accessibility_display_change(move || {
        let _ = tx.unbounded_send(());
    });
    cx.spawn(async move |cx| {
        while rx.next().await.is_some() {
            let on = crate::platform::increase_contrast();
            cx.update(|cx| increase_contrast_changed(cx, on));
        }
    })
    .detach();
}

/// Which theme slots the user's `settings.json` sets (`theme.light`,
/// `theme.dark`); the commented template sets none. An unreadable file sets
/// none, like the defaults the store falls back to.
pub fn explicit_themes_in(text: &str) -> themes::ExplicitThemes {
    let value: serde_json::Value = serde_json_lenient::from_str(text).unwrap_or_default();
    let theme = value.get("theme");
    let set = |key: &str| {
        theme
            .and_then(|t| t.get(key))
            .is_some_and(|v| v.is_string())
    };
    themes::ExplicitThemes {
        light: set("light"),
        dark: set("dark"),
    }
}

/// Records which theme slots `settings.json` sets, for the high contrast swap.
fn sync_explicit_themes(cx: &mut App, paths: &settings::ConfigPaths) {
    let text = std::fs::read_to_string(&paths.settings).unwrap_or_default();
    themes::set_explicit_themes(cx, explicit_themes_in(&text));
}

/// The OS "Increase contrast" switch is now `on`: re-applies the theme (the
/// brand themes become Parla High Contrast) and redraws. A repeat is a no-op.
pub fn increase_contrast_changed(cx: &mut App, on: bool) {
    if themes::increase_contrast(cx) == on {
        return;
    }
    themes::set_increase_contrast(cx, on);
    let Some(store) = cx.try_global::<SettingsStore>() else {
        return;
    };
    let settings = store.current().clone();
    let _ = themes::apply(cx, &settings);
    cx.refresh_windows();
}

/// Reloads the part of the config that `file` belongs to, then redraws.
pub fn handle_change(cx: &mut App, file: ChangedFile) {
    let paths = cx.global::<AppState>().paths.clone();
    // The source reloaded and its warnings; other sources keep theirs.
    let mut reloaded = None;
    match file {
        ChangedFile::Settings => {
            let outcome = cx.global_mut::<SettingsStore>().reload();
            // Setting or removing `theme.light` / `theme.dark` with the
            // default value leaves the settings equal but moves the swap.
            let explicit = themes::explicit_themes(cx);
            sync_explicit_themes(cx, &paths);
            let explicit_moved = explicit != themes::explicit_themes(cx);
            if matches!(outcome, ReloadOutcome::Changed) {
                let settings = cx.global::<SettingsStore>().current().clone();
                i18n::gpui::set_pref(cx, settings.locale.pref());
                crate::log::set_level(settings.log.level);
                crate::crash::set_mode(settings.crash_reports);
                crate::tray::Tray::settings_changed(cx);
                crate::mcp_host::McpHost::sync(cx);
                crate::login_item::LoginItems::apply(cx);
                crate::updates::settings_changed(cx);
                let warnings = themes::apply(cx, &settings)
                    .map(|w| w.to_string())
                    .into_iter()
                    .collect();
                reloaded = Some((WarningSource::Settings, warnings));
            } else if explicit_moved {
                let settings = cx.global::<SettingsStore>().current().clone();
                let _ = themes::apply(cx, &settings);
            }
        }
        ChangedFile::Theme(_) => {
            let registry = ThemeRegistry::global(cx);
            let mut warnings: Vec<String> = themes::load_user_themes(&registry, &paths.themes_dir)
                .iter()
                .map(|e| format!("{}: {}", e.path.display(), e.message))
                .collect();
            let settings = cx.global::<SettingsStore>().current().clone();
            if let Some(w) = themes::apply(cx, &settings) {
                warnings.push(w.to_string());
            }
            reloaded = Some((WarningSource::Theme, warnings));
        }
        ChangedFile::Keymap => reloaded = Some((WarningSource::Keymap, bind_keymap(cx, &paths))),
        ChangedFile::Workspace(_) => cx.global_mut::<SettingsStore>().note_workspace_changed(),
    }
    let error = cx.global::<SettingsStore>().last_error().cloned();
    let banner = cx.global_mut::<ConfigBanner>();
    banner.set_error(error);
    if let Some((source, warnings)) = reloaded {
        banner.set_warnings(source, warnings);
    }
    cx.refresh_windows();
}

/// The Keymap settings page wrote `text` to `keymap.json`: rebinds at once
/// (the file watcher reloads it again, harmlessly). Without the app state
/// (tests) only the live keymap text moves.
pub fn keymap_written(cx: &mut App, text: String) {
    if cx.try_global::<AppState>().is_some() {
        handle_change(cx, ChangedFile::Keymap);
    } else {
        cx.set_global(crate::shortcuts::LiveKeymap { user: text });
    }
}

/// Opens the Settings window (or focuses the open one), as `app::OpenSettings`
/// does. On-screen buttons call this directly: `App::dispatch_action` from a
/// click handler cannot reach the global handler, because the clicked window
/// is already being updated (GPUI logs "window not found" and drops it).
pub fn open_settings(cx: &mut App) {
    let Some(path) = cx
        .try_global::<AppState>()
        .map(|s| s.paths.settings.clone())
    else {
        return;
    };
    crate::settings_window::open_at(cx, path, env!("CARGO_PKG_VERSION"));
}

/// Opens the Settings window on `page`.
pub fn open_settings_page(cx: &mut App, page: crate::settings_window::Page) {
    let Some(path) = cx
        .try_global::<AppState>()
        .map(|s| s.paths.settings.clone())
    else {
        return;
    };
    crate::settings_window::open_on(cx, path, env!("CARGO_PKG_VERSION"), page);
}

/// Rebinds default + user keymap. Returns formatted warnings for dropped bindings.
fn bind_keymap(cx: &mut App, paths: &ConfigPaths) -> Vec<String> {
    let mut warnings = Vec::new();
    let default_path = std::path::Path::new("default keymap");
    let default = parse_keymap(default_path, DEFAULT_KEYMAP).unwrap_or_default();
    let user_text = std::fs::read_to_string(&paths.keymap).unwrap_or_default();
    let user: Vec<KeymapBlock> = match parse_keymap(&paths.keymap, &user_text) {
        Ok(blocks) => blocks,
        Err(e) => {
            warnings.push(e.to_string());
            Vec::new()
        }
    };
    cx.set_global(crate::shortcuts::LiveKeymap {
        user: user_text.clone(),
    });
    let (bindings, dropped) = resolve_bindings(&default, &user, &actions::is_known);
    let l = i18n::gpui::localizer(cx);
    warnings.extend(
        dropped
            .iter()
            .map(|w| t!(l, "warning-keymap", detail = format!("{w:?}")).into_owned()),
    );
    cx.clear_key_bindings();
    let mut keys = Vec::new();
    for b in bindings {
        let Ok(action) = cx.build_action(&b.action, None) else {
            continue;
        };
        let predicate = b
            .context
            .as_deref()
            .and_then(|c| gpui::KeyBindingContextPredicate::parse(c).ok())
            .map(std::rc::Rc::new);
        match KeyBinding::load(&b.keystrokes, action, predicate, false, None, cx) {
            Ok(k) => keys.push(k),
            Err(_) => {
                warnings.push(
                    t!(
                        l,
                        "warning-keymap-bad-keystrokes",
                        keystrokes = &b.keystrokes
                    )
                    .into_owned(),
                );
            }
        }
    }
    cx.bind_keys(keys);
    warnings
}

fn register_actions(cx: &mut App) {
    // Links the component actions so the keymap can name them; bind_keymap rebinds.
    components::init(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &OpenSettings, cx| open_settings(cx));
    cx.on_action(|_: &OpenSettingsFile, cx| {
        let path = cx.global::<AppState>().paths.settings.clone();
        cx.open_with_system(&path);
    });
    cx.on_action(|_: &OpenKeymap, cx| {
        let path = cx.global::<AppState>().paths.keymap.clone();
        cx.open_with_system(&path);
    });
    cx.on_action(|_: &ReloadConfig, cx| {
        let paths = cx.global::<AppState>().paths.clone();
        handle_change(cx, ChangedFile::Settings);
        handle_change(cx, ChangedFile::Theme(paths.themes_dir));
        handle_change(cx, ChangedFile::Keymap);
    });
    cx.on_action(|_: &ToggleMode, cx| toggle_mode(cx));
    crate::menus::register_actions(cx);
    cx.on_action(|_: &crate::actions::ToggleMute, cx| {
        crate::call_manager::CallManager::toggle_mute(cx)
    });
    cx.on_action(|_: &crate::actions::Leave, cx| crate::call_manager::CallManager::leave(cx));
    cx.on_action(|_: &crate::actions::ShowWindow, cx| {
        crate::call_manager::CallManager::show_window(cx)
    });
}

/// Flips light/dark by editing `settings.json` in place so comments survive.
fn toggle_mode(cx: &mut App) {
    let path = cx.global::<AppState>().paths.settings.clone();
    let next = match cx.theme().appearance {
        Appearance::Dark => "light",
        Appearance::Light => "dark",
    };
    if settings::edit_file(&path, &["theme", "mode"], &format!("\"{next}\"")).unwrap_or(false) {
        handle_change(cx, ChangedFile::Settings);
    }
}
