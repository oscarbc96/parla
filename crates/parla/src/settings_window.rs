//! The Settings window (spec 7.1, the SettingsGeneral board): a left
//! navigation of pages and the page itself. Pages not built yet say so and
//! offer `settings.json`. Every control writes its key through
//! [`settings::edit_file`], so the file watcher applies it like a hand edit;
//! the page reads the live [`SettingsStore`] at render time, so it also
//! follows edits made in the file. Nothing here polls or arms a timer.

use std::io;
use std::path::{Path, PathBuf};

use components::controls::{ButtonKind, button, compact, on_activate_app};
use components::keycap::Glyphs;
use components::settings_controls::{
    ThemePreview, add_keyword, choice_card, density_card, keyword_chip, keyword_field, nav_item,
    notice, parse_time, radio_card, segment, segment_keys, segmented, select, select_option,
    select_trigger, settings_row, settings_row_keys, settings_section, stepper, stepper_button,
    switch, theme_card, time_field,
};
use components::text_input::{TextInput, TextInputEvent};

use crate::idle_lock::IdleInput as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable as _, Global,
    IntoElement, KeyDownEvent, Render, SharedString, Subscription, Window, WindowHandle, div,
    prelude::*, size,
};
use settings::{
    AccentPreset, Clock, Density, DesktopNotifications, DockIcon, InlineImages, LocaleSetting,
    QuietHours, SendKey, Settings, SettingsStore, SidebarSort, ThemeMode, Weekday,
};
use themes::{ActiveTheme as _, Appearance, ThemeRegistry, metrics};

mod accounts_page;
mod advanced_page;
mod calls_page;
mod keymap_page;
mod mcp_page;

pub use accounts_page::{CACHE_MB_STEPS, PrivacyChange, RETENTION_STEPS, StorageChange};
pub use advanced_page::{AdvancedChange, IMAGE_CACHE_STEPS, LEAN_STEPS, MESSAGE_WINDOW_STEPS};
pub use calls_page::{CallsChange, RINGTONE_CHIME, RINGTONE_NONE};
pub use mcp_page::{McpChange, McpWorkspaceField, mcp_client_config};

/// A page of the Settings window, in the order of the navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Appearance,
    Notifications,
    Keymap,
    Workspaces,
    Privacy,
    Calls,
    Ai,
    Plugins,
    Mcp,
    Advanced,
}

impl Page {
    pub const ALL: [Page; 11] = [
        Page::General,
        Page::Appearance,
        Page::Notifications,
        Page::Keymap,
        Page::Workspaces,
        Page::Privacy,
        Page::Calls,
        Page::Ai,
        Page::Plugins,
        Page::Mcp,
        Page::Advanced,
    ];

    /// Stable id: the control ids and the catalog keys use it.
    pub fn id(self) -> &'static str {
        match self {
            Page::General => "general",
            Page::Appearance => "appearance",
            Page::Notifications => "notifications",
            Page::Keymap => "keymap",
            Page::Workspaces => "workspaces",
            Page::Privacy => "privacy",
            Page::Calls => "calls",
            Page::Ai => "ai",
            Page::Plugins => "plugins",
            Page::Mcp => "mcp",
            Page::Advanced => "advanced",
        }
    }

    fn label(self, l: &i18n::Localizer) -> String {
        let key = match self {
            Page::General => "settings-page-general",
            Page::Appearance => "settings-page-appearance",
            Page::Notifications => "settings-page-notifications",
            Page::Keymap => "settings-page-keymap",
            Page::Workspaces => "settings-page-workspaces",
            Page::Privacy => "settings-page-privacy",
            Page::Calls => "settings-page-calls",
            Page::Ai => "settings-page-ai",
            Page::Plugins => "settings-page-plugins",
            Page::Mcp => "settings-page-mcp",
            Page::Advanced => "settings-page-advanced",
        };
        l.format("parla", key, None).into_owned()
    }
}

/// One edit a General page control makes: the settings key and its value.
#[derive(Clone, Debug, PartialEq)]
pub enum GeneralChange {
    Locale(LocaleSetting),
    LaunchAtLogin(bool),
    DockIcon(DockIcon),
    RestoreWindows(bool),
    Clock(Clock),
    InlineImages(InlineImages),
    UpdatesCheck(bool),
}

impl GeneralChange {
    /// The key in `settings.json`, from the top-level object.
    pub fn key_path(&self) -> &'static [&'static str] {
        match self {
            Self::Locale(_) => &["locale"],
            Self::LaunchAtLogin(_) => &["launch_at_login"],
            Self::DockIcon(_) => &["dock_icon"],
            Self::RestoreWindows(_) => &["windows", "restore"],
            Self::Clock(_) => &["messages", "clock"],
            Self::InlineImages(_) => &["messages", "inline_images"],
            Self::UpdatesCheck(_) => &["updates", "check"],
        }
    }

    /// The new value as JSON text, spelled as the model reads it.
    pub fn raw_json(&self) -> String {
        let json = match self {
            Self::Locale(v) => serde_json::to_string(v),
            Self::LaunchAtLogin(v) | Self::UpdatesCheck(v) | Self::RestoreWindows(v) => {
                serde_json::to_string(v)
            }
            Self::DockIcon(v) => serde_json::to_string(v),
            Self::Clock(v) => serde_json::to_string(v),
            Self::InlineImages(v) => serde_json::to_string(v),
        };
        json.unwrap_or_default()
    }

    /// Writes the key into the file in place (comments stay). `Ok(false)`:
    /// the file is not a JSON object, nothing was written.
    pub fn write(&self, file: &Path) -> io::Result<bool> {
        settings::edit_file(file, self.key_path(), &self.raw_json())
    }
}

/// What the General page shows: the live values of its keys.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneralModel {
    pub locale: LocaleSetting,
    pub launch_at_login: bool,
    pub dock_icon: DockIcon,
    pub restore_windows: bool,
    pub clock: Clock,
    pub inline_images: InlineImages,
    pub updates_check: bool,
}

impl GeneralModel {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            locale: s.locale.clone(),
            launch_at_login: s.launch_at_login,
            dock_icon: s.dock_icon,
            restore_windows: s.windows.restore,
            clock: s.messages.clock,
            inline_images: s.messages.inline_images,
            updates_check: s.updates.check,
        }
    }

    /// Reads the settings store (defaults when the app has none).
    pub fn read(cx: &App) -> Self {
        match cx.try_global::<SettingsStore>() {
            Some(store) => Self::from_settings(store.current()),
            None => Self::from_settings(&Settings::default()),
        }
    }
}

/// One edit a page control makes: a settings key and its new value.
pub trait SettingChange {
    /// The key in `settings.json`, from the top-level object. A workspace's
    /// MCP policy key carries the workspace key, so the path is owned.
    fn path(&self) -> Vec<String>;
    /// The new value as JSON text, spelled as the model reads it.
    fn raw_json(&self) -> String;
    /// Writes the key into the file in place (comments stay). `Ok(false)`:
    /// the file is not a JSON object, nothing was written.
    fn write(&self, file: &Path) -> io::Result<bool> {
        let path = self.path();
        let refs: Vec<&str> = path.iter().map(String::as_str).collect();
        settings::edit_file(file, &refs, &self.raw_json())
    }
}

fn owned(path: &[&str]) -> Vec<String> {
    path.iter().map(|s| (*s).to_string()).collect()
}

impl SettingChange for GeneralChange {
    fn path(&self) -> Vec<String> {
        owned(GeneralChange::key_path(self))
    }
    fn raw_json(&self) -> String {
        GeneralChange::raw_json(self)
    }
}

fn json_text<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// One edit an Appearance page control makes.
#[derive(Clone, Debug, PartialEq)]
pub enum AppearanceChange {
    ThemeMode(ThemeMode),
    ThemeLight(String),
    ThemeDark(String),
    Density(Density),
    Accent(AccentPreset),
    SendKey(SendKey),
    SidebarUnreads(bool),
    SidebarSort(SidebarSort),
}

impl AppearanceChange {
    pub fn key_path(&self) -> &'static [&'static str] {
        match self {
            Self::ThemeMode(_) => &["theme", "mode"],
            Self::ThemeLight(_) => &["theme", "light"],
            Self::ThemeDark(_) => &["theme", "dark"],
            Self::Density(_) => &["density"],
            Self::Accent(_) => &["accent_preset"],
            Self::SendKey(_) => &["send_key"],
            Self::SidebarUnreads(_) => &["sidebar", "show_unreads_section"],
            Self::SidebarSort(_) => &["sidebar", "sort"],
        }
    }

    pub fn raw_json(&self) -> String {
        match self {
            Self::ThemeMode(v) => json_text(v),
            Self::ThemeLight(v) | Self::ThemeDark(v) => json_text(v),
            Self::Density(v) => json_text(v),
            Self::Accent(v) => json_text(v),
            Self::SendKey(v) => json_text(v),
            Self::SidebarUnreads(v) => json_text(v),
            Self::SidebarSort(v) => json_text(v),
        }
    }

    pub fn write(&self, file: &Path) -> io::Result<bool> {
        settings::edit_file(file, self.key_path(), &self.raw_json())
    }
}

impl SettingChange for AppearanceChange {
    fn path(&self) -> Vec<String> {
        owned(AppearanceChange::key_path(self))
    }
    fn raw_json(&self) -> String {
        AppearanceChange::raw_json(self)
    }
}

/// What the Appearance page shows: the live values of its keys.
#[derive(Clone, Debug, PartialEq)]
pub struct AppearanceModel {
    pub mode: ThemeMode,
    pub light: String,
    pub dark: String,
    pub density: Density,
    pub accent: AccentPreset,
    pub send_key: SendKey,
    pub show_unreads: bool,
    pub sort: SidebarSort,
}

impl AppearanceModel {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            mode: s.theme.mode,
            light: s.theme.light.clone(),
            dark: s.theme.dark.clone(),
            density: s.density,
            accent: s.accent_preset,
            send_key: s.send_key,
            show_unreads: s.sidebar.show_unreads_section,
            sort: s.sidebar.sort,
        }
    }

    pub fn read(cx: &App) -> Self {
        Self::from_settings(&current_settings(cx))
    }
}

/// One edit a Notifications page control makes.
#[derive(Clone, Debug, PartialEq)]
pub enum NotificationsChange {
    Desktop(DesktopNotifications),
    Keywords(Vec<String>),
    Sound(String),
    /// `None` turns quiet hours off (`null` in the file).
    QuietHours(Option<QuietHours>),
    ShowPreview(bool),
    HuddleRing(bool),
}

impl NotificationsChange {
    pub fn key_path(&self) -> &'static [&'static str] {
        match self {
            Self::Desktop(_) => &["notifications", "desktop"],
            Self::Keywords(_) => &["notifications", "keywords"],
            Self::Sound(_) => &["notifications", "sound"],
            Self::QuietHours(_) => &["notifications", "quiet_hours"],
            Self::ShowPreview(_) => &["notifications", "show_preview"],
            Self::HuddleRing(_) => &["calls", "ring"],
        }
    }

    pub fn raw_json(&self) -> String {
        match self {
            Self::Desktop(v) => json_text(v),
            Self::Keywords(v) => json_text(v),
            Self::Sound(v) => json_text(v),
            Self::QuietHours(v) => json_text(v),
            Self::ShowPreview(v) => json_text(v),
            Self::HuddleRing(v) => json_text(v),
        }
    }

    pub fn write(&self, file: &Path) -> io::Result<bool> {
        settings::edit_file(file, self.key_path(), &self.raw_json())
    }
}

impl SettingChange for NotificationsChange {
    fn path(&self) -> Vec<String> {
        owned(NotificationsChange::key_path(self))
    }
    fn raw_json(&self) -> String {
        NotificationsChange::raw_json(self)
    }
}

/// The sound value for "on" when it was off (`none`): the default sound.
const SOUND_ON: &str = "subtle";
const SOUND_OFF: &str = "none";
/// The weekday tiles of quiet hours: day, element id, letter key and full-name key.
const QUIET_DAYS: [(Weekday, &str, &str, &str); 7] = [
    (
        Weekday::Mon,
        "settings-quiet-day-mon",
        "settings-quiet-mon",
        "settings-quiet-mon-name",
    ),
    (
        Weekday::Tue,
        "settings-quiet-day-tue",
        "settings-quiet-tue",
        "settings-quiet-tue-name",
    ),
    (
        Weekday::Wed,
        "settings-quiet-day-wed",
        "settings-quiet-wed",
        "settings-quiet-wed-name",
    ),
    (
        Weekday::Thu,
        "settings-quiet-day-thu",
        "settings-quiet-thu",
        "settings-quiet-thu-name",
    ),
    (
        Weekday::Fri,
        "settings-quiet-day-fri",
        "settings-quiet-fri",
        "settings-quiet-fri-name",
    ),
    (
        Weekday::Sat,
        "settings-quiet-day-sat",
        "settings-quiet-sat",
        "settings-quiet-sat-name",
    ),
    (
        Weekday::Sun,
        "settings-quiet-day-sun",
        "settings-quiet-sun",
        "settings-quiet-sun-name",
    ),
];
/// Quiet hours a switch turns on.
const QUIET_FROM: &str = "20:00";
const QUIET_TO: &str = "08:00";

/// What the Notifications page shows: the live values of its keys.
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationsModel {
    pub desktop: DesktopNotifications,
    pub keywords: Vec<String>,
    pub sound_on: bool,
    pub quiet_hours: Option<QuietHours>,
    pub show_preview: bool,
    pub huddle_ring: bool,
}

impl NotificationsModel {
    pub fn from_settings(s: &Settings) -> Self {
        Self {
            desktop: s.notifications.desktop,
            keywords: s.notifications.keywords.clone(),
            sound_on: s.notifications.sound != SOUND_OFF,
            quiet_hours: s.notifications.quiet_hours.clone(),
            show_preview: s.notifications.show_preview,
            huddle_ring: s.calls.ring,
        }
    }

    pub fn read(cx: &App) -> Self {
        Self::from_settings(&current_settings(cx))
    }
}

/// Quiet-hours days after a click on `day`. An empty list means every day, so
/// the first click leaves the six others; picking all seven goes back to the
/// empty list; the last day cannot be taken off (that would silence nothing).
pub fn toggle_day(days: &[Weekday], day: Weekday) -> Vec<Weekday> {
    let mut set: Vec<Weekday> = if days.is_empty() {
        Weekday::ALL.to_vec()
    } else {
        days.to_vec()
    };
    if let Some(i) = set.iter().position(|d| *d == day) {
        if set.len() > 1 {
            set.remove(i);
        }
    } else {
        set.push(day);
        set.sort();
    }
    if set.len() == Weekday::ALL.len() {
        set.clear();
    }
    set
}

/// The live settings (defaults when the app has no store).
fn current_settings(cx: &App) -> Settings {
    match cx.try_global::<SettingsStore>() {
        Some(store) => store.current().clone(),
        None => Settings::default(),
    }
}

/// `base` with each `(key path, JSON value)` edit applied on top, for a value
/// written to the file but not yet reloaded. A value that does not read back
/// is ignored.
pub fn overlay(base: &Settings, edits: &[(&[&str], &str)]) -> Settings {
    let Ok(mut doc) = serde_json::to_value(base) else {
        return base.clone();
    };
    for (path, raw) in edits {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            continue;
        };
        set_path(&mut doc, path, value);
    }
    serde_json::from_value(doc).unwrap_or_else(|_| base.clone())
}

fn set_path(doc: &mut serde_json::Value, path: &[&str], value: serde_json::Value) {
    match path {
        [] => {}
        [last] => {
            if let Some(obj) = doc.as_object_mut() {
                obj.insert((*last).to_string(), value);
            }
        }
        [first, rest @ ..] => {
            // A missing object on the way is made, as the file edit does (a
            // workspace's first MCP policy key).
            if let Some(obj) = doc.as_object_mut() {
                let next = obj
                    .entry((*first).to_string())
                    .or_insert_with(|| serde_json::Value::Object(Default::default()));
                set_path(next, rest, value);
            }
        }
    }
}

/// A value written to the file that the settings store has not reloaded yet.
/// It stays valid for the store generation it was written in, so a second
/// click before the reload reads what the first one wrote.
struct Pending {
    path: Vec<String>,
    raw: String,
    generation: u64,
}

/// The text inputs of the Notifications page, made on first use.
struct Inputs {
    keyword: Entity<TextInput>,
    from: Entity<TextInput>,
    to: Entity<TextInput>,
    _subs: Vec<Subscription>,
}

pub struct SettingsView {
    path: PathBuf,
    version: SharedString,
    page: Page,
    language_open: bool,
    /// The Calls page's microphone or speaker select is open.
    pub(crate) device_open: Option<calls_page::DeviceSide>,
    /// The devices listed when a device select last opened.
    pub(crate) device_names: (Vec<String>, Vec<String>),
    focus: Option<FocusHandle>,
    focused: bool,
    pending: Vec<Pending>,
    /// Why the last edit did not reach the file, shown above the page.
    error: Option<String>,
    inputs: Option<Inputs>,
    /// The Keymap page's search, recording and filter, made on first use.
    keymap: Option<keymap_page::KeymapUi>,
    /// Copy client config was pressed on this visit of the MCP page.
    copied: bool,
    /// The Workspaces page's status pills: the manager's events (accounts
    /// and sessions added or removed, a failed start) and each session's
    /// sign-in state. Conversation-list and message changes are not watched.
    workspaces_sub: Option<Subscription>,
    session_subs: Vec<Subscription>,
    /// The idle lock: the page is covered while it is on.
    lock_sub: Option<Subscription>,
    lock_focus: Option<FocusHandle>,
}

/// Whether the card of theme `name` is marked: in System mode both
/// configured themes are, otherwise the one of the pinned appearance.
pub fn theme_card_selected(mode: ThemeMode, light: &str, dark: &str, name: &str) -> bool {
    match mode {
        ThemeMode::System => name == light || name == dark,
        ThemeMode::Light => name == light,
        ThemeMode::Dark => name == dark,
    }
}

/// The view, for the window and for tests: `path` is the `settings.json`
/// the controls edit.
pub fn view(path: PathBuf, version: String) -> SettingsView {
    SettingsView::new(path, version)
}

/// [`view`] opened on `page`.
pub fn view_on(path: PathBuf, version: String, page: Page) -> SettingsView {
    let mut v = SettingsView::new(path, version);
    v.page = page;
    v
}

impl SettingsView {
    fn new(path: PathBuf, version: String) -> Self {
        Self {
            path,
            version: version.into(),
            page: Page::General,
            language_open: false,
            device_open: None,
            device_names: (Vec::new(), Vec::new()),
            focus: None,
            focused: false,
            pending: Vec::new(),
            error: None,
            inputs: None,
            keymap: None,
            copied: false,
            workspaces_sub: None,
            session_subs: Vec::new(),
            lock_sub: None,
            lock_focus: None,
        }
    }

    pub fn page(&self) -> Page {
        self.page
    }

    /// The error notice shown above the page, if the last edit failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The error of the last rejected reload of the settings file, if any.
    fn store_error(cx: &App) -> Option<String> {
        cx.try_global::<SettingsStore>()
            .and_then(SettingsStore::last_error)
            .map(ToString::to_string)
    }

    /// The text of the notice above the page: the failed edit, else the
    /// reason the settings file was rejected.
    pub fn notice_text(&self, cx: &App) -> Option<String> {
        self.error.clone().or_else(|| Self::store_error(cx))
    }

    /// The settings the controls show: the store plus pending edits.
    pub fn live_settings(&self, cx: &App) -> Settings {
        self.settings(cx)
    }

    /// The live settings with the edits written but not yet reloaded on top.
    /// While the file is rejected the store never moves on, so pending edits
    /// are not shown.
    fn settings(&self, cx: &App) -> Settings {
        let base = current_settings(cx);
        if Self::store_error(cx).is_some() {
            return base;
        }
        let generation = cx
            .try_global::<SettingsStore>()
            .map_or(0, SettingsStore::generation);
        let paths: Vec<(Vec<&str>, &str)> = self
            .pending
            .iter()
            .filter(|p| p.generation == generation)
            .map(|p| (p.path.iter().map(String::as_str).collect(), p.raw.as_str()))
            .collect();
        let edits: Vec<(&[&str], &str)> = paths.iter().map(|(p, r)| (p.as_slice(), *r)).collect();
        if edits.is_empty() {
            base
        } else {
            overlay(&base, &edits)
        }
    }

    /// Writes `change` and remembers it until the store reloads. A failed
    /// write shows an error notice instead of doing nothing.
    fn apply(&mut self, change: impl SettingChange, cx: &mut Context<Self>) {
        let generation = cx
            .try_global::<SettingsStore>()
            .map_or(0, SettingsStore::generation);
        let path = change.path();
        match change.write(&self.path) {
            Ok(true) => {
                self.error = None;
                self.pending
                    .retain(|p| p.generation == generation && p.path != path);
                if Self::store_error(cx).is_none() {
                    self.pending.push(Pending {
                        path,
                        raw: change.raw_json(),
                        generation,
                    });
                }
            }
            Ok(false) => {
                crate::log::error(&format!("settings: {path:?}: the file is not an object"));
                let l = i18n::gpui::localizer(cx);
                self.error = Some(t!(l, "settings-error-not-object").into_owned());
            }
            Err(e) => {
                crate::log::error(&format!("settings: cannot write {path:?}: {e}"));
                let l = i18n::gpui::localizer(cx);
                self.error =
                    Some(t!(l, "settings-error-write", reason = e.to_string()).into_owned());
            }
        }
        self.language_open = false;
        self.device_open = None;
        cx.notify();
    }

    fn select_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.language_open = false;
        self.device_open = None;
        self.error = None;
        self.copied = false;
        cx.notify();
    }

    /// Runs `f` on this view when `el` is activated by click, Enter or Space.
    fn wire(
        &self,
        el: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let this = cx.entity();
        on_activate_app(el, move |window, cx| {
            this.update(cx, |v, cx| f(v, window, cx));
        })
    }

    fn open_file_button(
        &self,
        id: &'static str,
        label: String,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = *cx.theme().colors();
        let el = compact(button(id, label, ButtonKind::Secondary, true, &colors))
            .debug_selector(|| id.to_string());
        on_activate_app(el, |window, cx| {
            window.dispatch_action(Box::new(crate::actions::OpenSettingsFile), cx);
        })
    }

    fn header(&mut self, l: &i18n::Localizer, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.theme().colors();
        let left = if cfg!(target_os = "macos") {
            metrics::LAYOUT_TITLEBAR_LIGHTS
        } else {
            metrics::SPACE_6
        };
        let open = self.open_file_button(
            "settings-open-file",
            t!(l, "settings-open-file").into_owned(),
            cx,
        );
        crate::titlebar::draggable(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(metrics::SPACE_5)
                .h(metrics::LAYOUT_SETTINGS_HEADER)
                .pl(left)
                .pr(metrics::SPACE_6)
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(colors.border),
        )
        .child(
            div()
                .flex_1()
                .text_size(metrics::FONT_SIZE_SM)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .child(t!(l, "settings-title").into_owned()),
        )
        .child(open)
    }

    fn nav(&mut self, l: &i18n::Localizer, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let mut items = div().flex().flex_col().gap(metrics::SPACE_1);
        for page in Page::ALL {
            let el = nav_item(
                format!("settings-nav-{}", page.id()),
                page.label(l),
                self.page == page,
                &colors,
            );
            items = items.child(self.wire(el, cx, move |v, _, cx| v.select_page(page, cx)));
        }
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(metrics::LAYOUT_SETTINGS_NAV)
            .px(metrics::SPACE_4)
            .py(metrics::SPACE_5)
            .gap(metrics::SPACE_1)
            .border_r_1()
            .border_color(colors.border)
            .bg(colors.panel_background)
            .child(items)
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px(metrics::SPACE_5)
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(tokens.text_subtle)
                    .child(
                        t!(
                            l,
                            "settings-footer-version",
                            version = self.version.to_string()
                        )
                        .into_owned(),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(
                                t!(
                                    l,
                                    "settings-footer-file",
                                    path = config_dir_text(&self.path)
                                )
                                .into_owned(),
                            ),
                    ),
            )
    }

    fn page_header(
        title: String,
        subtitle: Option<String>,
        colors: &themes::ThemeColors,
    ) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_2)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_3XL)
                    .font_weight(metrics::FONT_WEIGHT_BOLD)
                    .child(title),
            )
            .children(subtitle.map(|s| {
                div()
                    .text_size(metrics::FONT_SIZE_SM)
                    .text_color(colors.text_muted)
                    .child(s)
            }))
    }

    fn placeholder(
        &mut self,
        page: Page,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let open = self.open_file_button(
            "settings-placeholder-open-file",
            t!(l, "settings-open-file").into_owned(),
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_6)
            .child(Self::page_header(page.label(l), None, &colors))
            .child(
                div()
                    .debug_selector(|| "settings-placeholder".into())
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(metrics::SPACE_4)
                    .max_w(metrics::LAYOUT_SETTINGS_CONTENT_MAX)
                    .p(metrics::SPACE_6)
                    .rounded(metrics::RADIUS_LG)
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.surface_background)
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_MD)
                            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                            .child(t!(l, "settings-placeholder-title").into_owned()),
                    )
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_SM)
                            .text_color(colors.text_muted)
                            .child(t!(l, "settings-placeholder-body").into_owned()),
                    )
                    .child(open),
            )
    }

    fn general(
        &mut self,
        window: &mut Window,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let (subtle, hairline) = (tokens.text_subtle, tokens.border_subtle);
        let m = GeneralModel::from_settings(&self.settings(cx));
        let locales = i18n::LocaleSet::embedded();

        // Language: the select with its open list.
        let current = match &m.locale {
            LocaleSetting::System => t!(l, "settings-language-system").into_owned(),
            LocaleSetting::Code(c) => locales
                .locales
                .iter()
                .find(|i| i.code.to_string() == *c)
                .map(|i| i.name.clone())
                .unwrap_or_else(|| c.clone()),
        };
        let trigger = self.wire(
            select_trigger("settings-language", current, self.language_open, &colors),
            cx,
            |v, _, cx| {
                v.language_open = !v.language_open;
                cx.notify();
            },
        );
        let options = self.language_open.then(|| {
            let mut options = vec![self.wire(
                select_option(
                    "settings-language-system",
                    t!(l, "settings-language-system").into_owned(),
                    m.locale == LocaleSetting::System,
                    &colors,
                ),
                cx,
                |v, _, cx| v.apply(GeneralChange::Locale(LocaleSetting::System), cx),
            )];
            for info in &locales.locales {
                let code = info.code.to_string();
                let choice = LocaleSetting::Code(code.clone());
                let el = select_option(
                    format!("settings-language-{code}"),
                    info.name.clone(),
                    m.locale == choice,
                    &colors,
                );
                options.push(self.wire(el, cx, move |v, _, cx| {
                    v.apply(GeneralChange::Locale(choice.clone()), cx)
                }));
            }
            options
        });
        let this = cx.entity();
        let language = select(
            "settings-language",
            trigger,
            options,
            &colors,
            move |_, cx| {
                this.update(cx, |v, cx| {
                    v.language_open = false;
                    cx.notify();
                })
            },
            window,
            cx,
        );
        let language_body = settings_row(
            t!(l, "settings-language-label").into_owned(),
            None,
            language,
            &colors,
            subtle,
        );

        let login = self.wire(
            switch("settings-login", m.launch_at_login, &colors),
            cx,
            |v, _, cx| {
                let on = !v.settings(cx).launch_at_login;
                v.apply(GeneralChange::LaunchAtLogin(on), cx)
            },
        );
        let restore = self.wire(
            switch("settings-restore-windows", m.restore_windows, &colors),
            cx,
            |v, _, cx| {
                let on = !v.settings(cx).windows.restore;
                v.apply(GeneralChange::RestoreWindows(on), cx)
            },
        );
        let startup_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-login-label").into_owned(),
                Some(t!(l, "settings-login-help").into_owned().into()),
                login,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-restore-label").into_owned(),
                Some(t!(l, "settings-restore-help").into_owned().into()),
                restore,
                &colors,
                subtle,
            ));

        let dock_card = |this: &Self,
                         cx: &mut Context<Self>,
                         id: &'static str,
                         title: String,
                         desc: String,
                         value: DockIcon| {
            let el = choice_card(id, title, desc, m.dock_icon == value, &colors, subtle).min_w_0();
            this.wire(el, cx, move |v, _, cx| {
                v.apply(GeneralChange::DockIcon(value), cx)
            })
        };
        let dock_body = div()
            .flex()
            .gap(metrics::SPACE_5)
            .child(dock_card(
                self,
                cx,
                "settings-dock-always",
                t!(l, "settings-dock-always").into_owned(),
                t!(l, "settings-dock-always-desc").into_owned(),
                DockIcon::Always,
            ))
            .child(dock_card(
                self,
                cx,
                "settings-dock-window",
                t!(l, "settings-dock-window").into_owned(),
                t!(l, "settings-dock-window-desc").into_owned(),
                DockIcon::WithWindow,
            ))
            .child(dock_card(
                self,
                cx,
                "settings-dock-never",
                t!(l, "settings-dock-never").into_owned(),
                t!(l, "settings-dock-never-desc").into_owned(),
                DockIcon::Never,
            ));

        let clock_seg =
            |this: &Self, cx: &mut Context<Self>, id: &'static str, label: String, value: Clock| {
                let el = segment(id, label, m.clock == value, &colors);
                this.wire(el, cx, move |v, _, cx| {
                    v.apply(GeneralChange::Clock(value), cx)
                })
            };
        let clock = segmented(
            [
                clock_seg(
                    self,
                    cx,
                    "settings-clock-24h",
                    t!(l, "settings-clock-24").into_owned(),
                    Clock::H24,
                ),
                clock_seg(
                    self,
                    cx,
                    "settings-clock-12h",
                    t!(l, "settings-clock-12").into_owned(),
                    Clock::H12,
                ),
            ],
            &colors,
        );
        let image_seg = |this: &Self,
                         cx: &mut Context<Self>,
                         id: &'static str,
                         label: String,
                         value: InlineImages| {
            let el = segment(id, label, m.inline_images == value, &colors);
            this.wire(el, cx, move |v, _, cx| {
                v.apply(GeneralChange::InlineImages(value), cx)
            })
        };
        let images = segmented(
            [
                image_seg(
                    self,
                    cx,
                    "settings-images-always",
                    t!(l, "settings-images-always").into_owned(),
                    InlineImages::Always,
                ),
                image_seg(
                    self,
                    cx,
                    "settings-images-click",
                    t!(l, "settings-images-click").into_owned(),
                    InlineImages::OnClick,
                ),
                image_seg(
                    self,
                    cx,
                    "settings-images-never",
                    t!(l, "settings-images-never").into_owned(),
                    InlineImages::Never,
                ),
            ],
            &colors,
        );
        let messages_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-clock-label").into_owned(),
                None,
                clock,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-images-label").into_owned(),
                None,
                images,
                &colors,
                subtle,
            ));

        let auto = self.wire(
            switch("settings-updates-auto", m.updates_check, &colors),
            cx,
            |v, _, cx| {
                let on = !v.settings(cx).updates.check;
                v.apply(GeneralChange::UpdatesCheck(on), cx)
            },
        );
        let check_now = {
            let el = compact(button(
                "settings-check-now",
                t!(l, "settings-updates-check-now").into_owned(),
                ButtonKind::Secondary,
                true,
                &colors,
            ))
            .debug_selector(|| "settings-check-now".into());
            on_activate_app(el, |window, cx| {
                window.dispatch_action(Box::new(crate::actions::CheckForUpdates), cx);
            })
        };
        let mut status = t!(
            l,
            "settings-updates-status",
            version = self.version.to_string()
        )
        .into_owned();
        // The last check, from the update manager's in-memory timestamp (no
        // disk read); nothing is shown before a check has happened.
        if let Some(ts) = cx
            .try_global::<crate::updates::UpdateManagerHandle>()
            .and_then(|h| h.0.read(cx).checker().last_attempt())
            .and_then(|t| jiff::Timestamp::from_second(t).ok())
        {
            let time = l.time(
                ts,
                &jiff::tz::TimeZone::system(),
                components::message_row::hour_cycle(m.clock),
            );
            status = format!(
                "{status} {}",
                t!(l, "settings-updates-checked", time = time)
            );
        }
        let updates_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-updates-auto").into_owned(),
                None,
                auto,
                &colors,
                subtle,
            ))
            .child(settings_row(status, None, check_now, &colors, subtle));

        let section = |title: &str, desc: &str, body: gpui::Div, first: bool| {
            settings_section(
                l.format("parla", title, None).into_owned(),
                l.format("parla", desc, None).into_owned(),
                body,
                first,
                &colors,
                subtle,
                hairline,
            )
        };
        let mut page = div()
            .flex()
            .flex_col()
            .child(Self::page_header(
                t!(l, "settings-general-title").into_owned(),
                Some(t!(l, "settings-general-subtitle").into_owned()),
                &colors,
            ))
            .child(div().h(metrics::SPACE_4))
            .child(section(
                "settings-language-title",
                "settings-language-desc",
                language_body,
                true,
            ))
            .child(section(
                "settings-startup-title",
                "settings-startup-desc",
                div().child(startup_body),
                false,
            ));
        if cfg!(target_os = "macos") {
            page = page.child(section(
                "settings-dock-title",
                "settings-dock-desc",
                dock_body,
                false,
            ));
        }
        page.child(section(
            "settings-messages-title",
            "settings-messages-desc",
            messages_body,
            false,
        ))
        .child(section(
            "settings-updates-title",
            "settings-updates-desc",
            updates_body,
            false,
        ))
        .pb(metrics::SPACE_8)
    }

    /// One section of a page, with the strings of `title` and `desc`.
    fn section(
        l: &i18n::Localizer,
        title: &str,
        desc: &str,
        body: impl IntoElement,
        first: bool,
        cx: &App,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        settings_section(
            l.format("parla", title, None).into_owned(),
            l.format("parla", desc, None).into_owned(),
            body,
            first,
            &colors,
            tokens.text_subtle,
            tokens.border_subtle,
        )
    }

    fn appearance(&mut self, l: &i18n::Localizer, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let subtle = tokens.text_subtle;
        let m = AppearanceModel::from_settings(&self.settings(cx));

        let mode_seg =
            |this: &Self, cx: &mut Context<Self>, id: &'static str, label: String, v: ThemeMode| {
                let el = segment(id, label, m.mode == v, &colors);
                this.wire(el, cx, move |v_, _, cx| {
                    v_.apply(AppearanceChange::ThemeMode(v), cx)
                })
            };
        let mode = segmented(
            [
                mode_seg(
                    self,
                    cx,
                    "settings-theme-mode-system",
                    t!(l, "settings-theme-system").into_owned(),
                    ThemeMode::System,
                ),
                mode_seg(
                    self,
                    cx,
                    "settings-theme-mode-light",
                    t!(l, "settings-theme-light").into_owned(),
                    ThemeMode::Light,
                ),
                mode_seg(
                    self,
                    cx,
                    "settings-theme-mode-dark",
                    t!(l, "settings-theme-dark").into_owned(),
                    ThemeMode::Dark,
                ),
            ],
            &colors,
        )
        .self_start();

        // One card per registered theme: the built-ins and the user's.
        let mut cards = div().flex().flex_wrap().gap(metrics::SPACE_5);
        for theme in ThemeRegistry::global(cx).list() {
            let light = theme.appearance == Appearance::Light;
            let name = theme.name.to_string();
            let chosen = self.settings(cx).theme;
            // Mark what renders: under "Increase contrast" that is the high
            // contrast pair unless the user chose a theme.
            let (light_name, dark_name) = themes::rendered_names(cx, &chosen);
            let selected = theme_card_selected(chosen.mode, &light_name, &dark_name, &name);
            let tk = themes::parla_tokens(&theme);
            let preview = ThemePreview {
                background: theme.colors().background,
                sidebar: tk.sidebar_background,
                sidebar_text: tk.sidebar_text,
                text: theme.colors().text,
                accent: theme.colors().element_background,
            };
            let kind = if light {
                t!(l, "settings-theme-light").into_owned()
            } else {
                t!(l, "settings-theme-dark").into_owned()
            };
            let el = theme_card(
                format!("settings-theme-{}", slug(&name)),
                name.clone(),
                kind,
                selected,
                preview,
                &colors,
                subtle,
            );
            cards = cards.child(self.wire(el, cx, move |v, _, cx| {
                v.pick_theme(&name, light, cx);
            }));
        }
        let theme_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_6)
            .child(mode)
            .child(cards);

        let dens = |this: &Self,
                    cx: &mut Context<Self>,
                    id: &'static str,
                    title: String,
                    rows: usize,
                    value: Density| {
            let el = density_card(
                id,
                title,
                rows,
                value == Density::Cozy,
                m.density == value,
                &colors,
            );
            this.wire(el, cx, move |v, _, cx| {
                v.apply(AppearanceChange::Density(value), cx)
            })
        };
        let density_body = div()
            .flex()
            .gap(metrics::SPACE_5)
            .child(dens(
                self,
                cx,
                "settings-density-compact",
                t!(l, "settings-density-compact").into_owned(),
                3,
                Density::Compact,
            ))
            .child(dens(
                self,
                cx,
                "settings-density-cozy",
                t!(l, "settings-density-cozy").into_owned(),
                2,
                Density::Cozy,
            ));

        let current = self.settings(cx);
        // The built-in high contrast themes keep their own accent (7:1).
        let presets_on = themes::accent_presets_apply(cx);
        let mut swatches = div().flex().flex_wrap().gap(metrics::SPACE_6);
        for p in AccentPreset::ALL {
            let name = match p {
                AccentPreset::Brand => t!(l, "settings-accent-brand"),
                AccentPreset::Ember => t!(l, "settings-accent-ember"),
                AccentPreset::Amber => t!(l, "settings-accent-amber"),
                AccentPreset::Coral => t!(l, "settings-accent-coral"),
                AccentPreset::Ink => t!(l, "settings-accent-ink"),
            }
            .into_owned();
            let el = components::settings_controls::accent_swatch(
                format!("settings-accent-{}", p.as_str()),
                name,
                themes::accent_swatch(cx, &current, p),
                m.accent == p,
                presets_on,
                &colors,
            );
            swatches = if presets_on {
                swatches.child(self.wire(el, cx, move |v, _, cx| {
                    v.apply(AppearanceChange::Accent(p), cx)
                }))
            } else {
                swatches.child(components::settings_controls::disabled(el, l))
            };
        }
        let swatches = if presets_on {
            swatches
        } else {
            div()
                .flex()
                .flex_col()
                .gap(metrics::SPACE_4)
                .child(swatches)
                .child(
                    div()
                        .debug_selector(|| "settings-accent-high-contrast".into())
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(colors.text_muted)
                        .child(t!(l, "settings-accent-high-contrast").into_owned()),
                )
        };

        let unreads = self.wire(
            switch("settings-sidebar-unreads", m.show_unreads, &colors),
            cx,
            |v, _, cx| {
                let on = !v.settings(cx).sidebar.show_unreads_section;
                v.apply(AppearanceChange::SidebarUnreads(on), cx)
            },
        );
        let sort_seg = |this: &Self,
                        cx: &mut Context<Self>,
                        id: &'static str,
                        label: String,
                        v: SidebarSort| {
            let el = segment(id, label, m.sort == v, &colors);
            this.wire(el, cx, move |v_, _, cx| {
                v_.apply(AppearanceChange::SidebarSort(v), cx)
            })
        };
        let sort = segmented(
            [
                sort_seg(
                    self,
                    cx,
                    "settings-sidebar-sort-alpha",
                    t!(l, "settings-sidebar-sort-alpha").into_owned(),
                    SidebarSort::Alphabetical,
                ),
                sort_seg(
                    self,
                    cx,
                    "settings-sidebar-sort-recent",
                    t!(l, "settings-sidebar-sort-recent").into_owned(),
                    SidebarSort::Recent,
                ),
            ],
            &colors,
        );
        let key_seg =
            |this: &Self, cx: &mut Context<Self>, id: &'static str, label: &Glyphs, v: SendKey| {
                let el = segment_keys(id, label, m.send_key == v, &colors);
                this.wire(el, cx, move |v_, _, cx| {
                    v_.apply(AppearanceChange::SendKey(v), cx)
                })
            };
        let cmd_label = Glyphs::new(
            if cfg!(target_os = "macos") {
                t!(l, "settings-send-key-cmd-enter-mac").into_owned()
            } else {
                t!(l, "settings-send-key-cmd-enter").into_owned()
            },
            cx,
        );
        let enter_label = Glyphs::new(t!(l, "settings-send-key-enter").into_owned(), cx);
        let send_key_seg = segmented(
            [
                key_seg(
                    self,
                    cx,
                    "settings-send-key-enter",
                    &enter_label,
                    SendKey::Enter,
                ),
                key_seg(
                    self,
                    cx,
                    "settings-send-key-cmd-enter",
                    &cmd_label,
                    SendKey::CmdEnter,
                ),
            ],
            &colors,
        )
        .self_start();
        let composer_body = settings_row_keys(
            t!(l, "settings-send-key-row").into_owned(),
            &Glyphs::new(t!(l, "settings-send-key-row-desc").into_owned(), cx),
            send_key_seg,
            &colors,
            subtle,
        );
        let sidebar_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-sidebar-unreads").into_owned(),
                None,
                unreads,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-sidebar-sort").into_owned(),
                None,
                sort,
                &colors,
                subtle,
            ));

        div()
            .flex()
            .flex_col()
            .child(Self::page_header(
                t!(l, "settings-appearance-title").into_owned(),
                Some(t!(l, "settings-appearance-subtitle").into_owned()),
                &colors,
            ))
            .child(div().h(metrics::SPACE_4))
            .child(Self::section(
                l,
                "settings-theme-title",
                "settings-theme-desc",
                theme_body,
                true,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-accent-title",
                "settings-accent-desc",
                swatches,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-density-title",
                "settings-density-desc",
                density_body,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-sidebar-title",
                "settings-sidebar-desc",
                sidebar_body,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-composer-title",
                "settings-composer-desc",
                composer_body,
                false,
                cx,
            ))
            .child(
                div()
                    .pt(metrics::SPACE_5)
                    .border_t_1()
                    .border_color(tokens.border_subtle)
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(colors.text_muted)
                    .child(t!(l, "settings-appearance-footer").into_owned()),
            )
            .pb(metrics::SPACE_8)
    }

    /// Picks `name` as the theme of its appearance. With a fixed mode of the
    /// other appearance the mode follows, so the pick shows at once.
    fn pick_theme(&mut self, name: &str, light: bool, cx: &mut Context<Self>) {
        let mode = self.settings(cx).theme.mode;
        self.apply(
            if light {
                AppearanceChange::ThemeLight(name.to_string())
            } else {
                AppearanceChange::ThemeDark(name.to_string())
            },
            cx,
        );
        let want = if light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        if mode != ThemeMode::System && mode != want {
            self.apply(AppearanceChange::ThemeMode(want), cx);
        }
    }

    /// The text inputs of the Notifications page, made on first use.
    fn inputs(
        &mut self,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> (Entity<TextInput>, Entity<TextInput>, Entity<TextInput>) {
        if self.inputs.is_none() {
            let keyword = cx.new(TextInput::new);
            let from = cx.new(TextInput::new);
            let to = cx.new(TextInput::new);
            let add = t!(l, "settings-keywords-add").into_owned();
            keyword.update(cx, |t, _| {
                t.set_single_line(true);
                t.set_placeholder(add);
            });
            let hhmm = t!(l, "settings-time-placeholder").into_owned();
            for input in [&from, &to] {
                let ph = hhmm.clone();
                input.update(cx, |t, _| {
                    t.set_single_line(true);
                    t.set_placeholder(ph);
                });
            }
            let subs = vec![
                cx.subscribe(&from, |v, _, _: &TextInputEvent, cx| v.time_typed(true, cx)),
                cx.subscribe(&to, |v, _, _: &TextInputEvent, cx| v.time_typed(false, cx)),
            ];
            self.inputs = Some(Inputs {
                keyword,
                from,
                to,
                _subs: subs,
            });
        }
        let i = self.inputs.as_ref().expect("made above");
        (i.keyword.clone(), i.from.clone(), i.to.clone())
    }

    /// A quiet-hours field changed: a valid time that differs from the stored
    /// one is written; anything else waits for more typing.
    fn time_typed(&mut self, from: bool, cx: &mut Context<Self>) {
        let Some(inputs) = &self.inputs else { return };
        let input = if from { &inputs.from } else { &inputs.to };
        let Some(time) = parse_time(input.read(cx).text()) else {
            return;
        };
        let Some(mut q) = self.settings(cx).notifications.quiet_hours else {
            return;
        };
        let slot = if from { &mut q.from } else { &mut q.to };
        if *slot != time {
            *slot = time;
            self.apply(NotificationsChange::QuietHours(Some(q)), cx);
        }
    }

    /// Adds the typed keyword (Enter in the keyword box).
    fn add_typed_keyword(&mut self, cx: &mut Context<Self>) {
        let Some(inputs) = &self.inputs else { return };
        let input = inputs.keyword.clone();
        let words = self.settings(cx).notifications.keywords;
        if let Some(next) = add_keyword(&words, input.read(cx).text()) {
            self.apply(NotificationsChange::Keywords(next), cx);
            input.update(cx, |t, cx| t.set_text("", cx));
        }
    }

    fn notifications(
        &mut self,
        window: &mut Window,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let subtle = tokens.text_subtle;
        let error = cx.theme().status().error;
        let m = NotificationsModel::from_settings(&self.settings(cx));
        let (keyword_input, from_input, to_input) = self.inputs(l, cx);

        let desktop_card = |this: &Self,
                            cx: &mut Context<Self>,
                            id: &'static str,
                            title: String,
                            desc: String,
                            value: DesktopNotifications| {
            let el = radio_card(id, title, desc, m.desktop == value, &colors, subtle);
            this.wire(el, cx, move |v, _, cx| {
                v.apply(NotificationsChange::Desktop(value), cx)
            })
        };
        let desktop_body = div()
            .flex()
            .gap(metrics::SPACE_4)
            .child(desktop_card(
                self,
                cx,
                "settings-desktop-all",
                t!(l, "settings-desktop-all").into_owned(),
                t!(l, "settings-desktop-all-desc").into_owned(),
                DesktopNotifications::All,
            ))
            .child(desktop_card(
                self,
                cx,
                "settings-desktop-mentions",
                t!(l, "settings-desktop-mentions").into_owned(),
                t!(l, "settings-desktop-mentions-desc").into_owned(),
                DesktopNotifications::Mentions,
            ))
            .child(desktop_card(
                self,
                cx,
                "settings-desktop-none",
                t!(l, "settings-desktop-none").into_owned(),
                t!(l, "settings-desktop-none-desc").into_owned(),
                DesktopNotifications::None,
            ));

        let preview = self.wire(
            switch("settings-preview", m.show_preview, &colors),
            cx,
            |v, _, cx| {
                let on = !v.settings(cx).notifications.show_preview;
                v.apply(NotificationsChange::ShowPreview(on), cx)
            },
        );
        let desktop_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_4)
            .child(desktop_body)
            .child(settings_row(
                t!(l, "settings-preview-label").into_owned(),
                Some(t!(l, "settings-preview-help").into_owned().into()),
                preview,
                &colors,
                subtle,
            ));

        let mut chips = Vec::new();
        for (i, word) in m.keywords.iter().enumerate() {
            let el = keyword_chip(
                format!("settings-keyword-{i}"),
                word,
                &colors,
                tokens.accent_soft_text,
            );
            chips.push(self.wire(el, cx, move |v, _, cx| {
                let mut words = v.settings(cx).notifications.keywords;
                if i < words.len() {
                    words.remove(i);
                    v.apply(NotificationsChange::Keywords(words), cx);
                }
            }));
        }
        let keyword_focused = keyword_input.read(cx).focus_handle(cx).is_focused(window);
        let keywords_body = keyword_field(
            chips,
            keyword_input,
            keyword_focused,
            &colors,
            tokens.border_control,
        )
        .on_key_down(cx.listener(|v, ev: &KeyDownEvent, _, cx| {
            if ev.keystroke.key == "enter" {
                v.add_typed_keyword(cx);
                cx.stop_propagation();
            }
        }));

        let sound = self.wire(
            switch("settings-sound", m.sound_on, &colors),
            cx,
            |v, _, cx| {
                let on = v.settings(cx).notifications.sound == SOUND_OFF;
                let value = if on { SOUND_ON } else { SOUND_OFF };
                v.apply(NotificationsChange::Sound(value.into()), cx)
            },
        );
        let sound_body = settings_row(
            t!(l, "settings-sound-label").into_owned(),
            None,
            sound,
            &colors,
            subtle,
        );

        let quiet = self.wire(
            switch("settings-quiet", m.quiet_hours.is_some(), &colors),
            cx,
            |v, _, cx| {
                let next = match v.settings(cx).notifications.quiet_hours {
                    Some(_) => None,
                    None => Some(QuietHours {
                        from: QUIET_FROM.into(),
                        to: QUIET_TO.into(),
                        days: Vec::new(),
                    }),
                };
                v.apply(NotificationsChange::QuietHours(next), cx)
            },
        );
        let mut quiet_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(metrics::SPACE_6)
            .child(quiet);
        if let Some(q) = &m.quiet_hours {
            // The fields follow the stored times unless the user is typing.
            for (input, stored) in [(&from_input, &q.from), (&to_input, &q.to)] {
                let typing = input.focus_handle(cx).is_focused(window);
                if !typing && input.read(cx).text() != stored {
                    let stored = stored.clone();
                    input.update(cx, |t, cx| t.set_text(&stored, cx));
                }
            }
            let bad = |input: &Entity<TextInput>, cx: &App| {
                let text = input.read(cx).text();
                !(text.is_empty() || parse_time(text).is_some())
            };
            let border_for = |bad: bool| if bad { error } else { tokens.border_control };
            let (from_bad, to_bad) = (bad(&from_input, cx), bad(&to_input, cx));
            let (from_focused, to_focused) = (
                from_input.focus_handle(cx).is_focused(window),
                to_input.focus_handle(cx).is_focused(window),
            );
            let label = |text: String| {
                div()
                    .text_size(metrics::FONT_SIZE_SM)
                    .text_color(colors.text)
                    .child(text)
            };
            // An invalid time also says so in words and an icon, not colour alone.
            let hint = |id: &'static str, bad: bool| {
                bad.then(|| {
                    div()
                        .debug_selector(|| id.to_string())
                        .flex()
                        .items_center()
                        .gap(metrics::SPACE_2)
                        .text_size(metrics::FONT_SIZE_SM)
                        .text_color(colors.text)
                        .child(
                            components::icon::Icon::new(components::icon::IconName::Alert)
                                .size(metrics::ICON_SIZE_SM)
                                .color(error),
                        )
                        .child(t!(l, "settings-time-invalid").into_owned())
                })
            };
            quiet_row = quiet_row
                .child(label(t!(l, "settings-quiet-from").into_owned()))
                .child(time_field(
                    "settings-quiet-from",
                    from_input,
                    from_focused,
                    &colors,
                    border_for(from_bad),
                ))
                .children(hint("settings-quiet-from-hint", from_bad))
                .child(label(t!(l, "settings-quiet-to").into_owned()))
                .child(time_field(
                    "settings-quiet-to",
                    to_input,
                    to_focused,
                    &colors,
                    border_for(to_bad),
                ))
                .children(hint("settings-quiet-to-hint", to_bad));
        }
        // The days a window starts on, inline after the times: all lit when
        // none are picked.
        if let Some(q) = &m.quiet_hours {
            for (day, id, letter, name) in QUIET_DAYS {
                let on = q.days.is_empty() || q.days.contains(&day);
                let el = components::settings_controls::day_tile(
                    id,
                    l.format("parla", letter, None).into_owned(),
                    l.format("parla", name, None).into_owned(),
                    on,
                    &colors,
                );
                quiet_row = quiet_row.child(self.wire(el, cx, move |v, _, cx| {
                    let Some(mut q) = v.settings(cx).notifications.quiet_hours else {
                        return;
                    };
                    let next = toggle_day(&q.days, day);
                    if next != q.days {
                        q.days = next;
                        v.apply(NotificationsChange::QuietHours(Some(q)), cx);
                    }
                }));
            }
        }

        let ring = self.wire(
            switch("settings-huddle-ring", m.huddle_ring, &colors),
            cx,
            |v, _, cx| {
                let on = !v.settings(cx).calls.ring;
                v.apply(NotificationsChange::HuddleRing(on), cx)
            },
        );
        let huddles_body = settings_row(
            t!(l, "settings-huddles-ring").into_owned(),
            None,
            ring,
            &colors,
            subtle,
        );

        div()
            .flex()
            .flex_col()
            .child(Self::page_header(
                t!(l, "settings-notifications-title").into_owned(),
                Some(t!(l, "settings-notifications-subtitle").into_owned()),
                &colors,
            ))
            .child(div().h(metrics::SPACE_4))
            .child(Self::section(
                l,
                "settings-desktop-title",
                "settings-desktop-desc",
                desktop_body,
                true,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-keywords-title",
                "settings-keywords-desc",
                keywords_body,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-sound-title",
                "settings-sound-desc",
                sound_body,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-quiet-title",
                "settings-quiet-desc",
                quiet_row,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-huddles-title",
                "settings-huddles-desc",
                huddles_body,
                false,
                cx,
            ))
            .pb(metrics::SPACE_8)
    }
}

/// The value after `current` on `ladder` (`up`) or before it, staying at the
/// ends. A value between steps moves to the next step its way.
pub fn step(ladder: &[u32], current: u32, up: bool) -> u32 {
    let next = if up {
        ladder.iter().copied().find(|&v| v > current)
    } else {
        ladder.iter().rev().copied().find(|&v| v < current)
    };
    next.unwrap_or(current)
}

/// The signed-in workspaces, in the manager's order (none without one).
fn accounts(cx: &App) -> Vec<domain::AccountInfo> {
    cx.try_global::<crate::workspaces::WorkspacesHandle>()
        .map(|h| h.0.read(cx).accounts().to_vec())
        .unwrap_or_default()
}

/// Runs `f` on the main window's view, later in this frame. `false`: there is
/// no main window.
fn with_root(
    cx: &mut App,
    f: impl FnOnce(
        &mut crate::root_view::RootView,
        &mut Window,
        &mut Context<crate::root_view::RootView>,
    ) + 'static,
) -> bool {
    let Some(root) = cx
        .windows()
        .into_iter()
        .find_map(|w| w.downcast::<crate::root_view::RootView>())
    else {
        return false;
    };
    cx.defer(move |cx| {
        let _ = root.update(cx, |v, window, cx| f(v, window, cx));
    });
    true
}

/// Clear cache: every running workspace drops its local database and loads
/// again from the provider (`store::RebuildCache` for each).
fn clear_cache(cx: &mut App) {
    let Some(w) = cx
        .try_global::<crate::workspaces::WorkspacesHandle>()
        .map(|h| h.0.clone())
    else {
        return;
    };
    let sessions: Vec<_> = w.read(cx).sessions().cloned().collect();
    for s in sessions {
        s.update(cx, |s, cx| s.rebuild_cache(cx));
    }
}

impl SettingsView {
    /// Re-renders when a session signs in, expires or reconnects; one
    /// subscription per running session, renewed when the set changes.
    fn watch_sessions(
        &mut self,
        w: &Entity<crate::workspaces::Workspaces>,
        cx: &mut Context<Self>,
    ) {
        let sessions: Vec<_> = w.read(cx).sessions().cloned().collect();
        self.session_subs = sessions
            .iter()
            .map(|s| {
                cx.subscribe(s, |_, _, ev: &crate::session::SessionEvent, cx| {
                    use crate::session::SessionEvent as E;
                    if matches!(ev, E::Session | E::Connection) {
                        cx.notify();
                    }
                })
            })
            .collect();
    }

    /// Asks `title` in this window with Cancel (the default) and `confirm`;
    /// runs `then` only on `confirm`.
    fn confirm(
        &mut self,
        title: String,
        detail: String,
        confirm: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let cancel = t!(cx, "shell-cancel").into_owned();
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &title,
            Some(&detail),
            &[cancel.as_str(), confirm.as_str()],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(1) {
                this.update_in(cx, |v, window, cx| then(v, window, cx)).ok();
            }
        })
        .detach();
    }

    /// A number between `-` and `+` that steps `ladder`; `read` is the key's
    /// live value and `make` the edit for a new one.
    #[allow(clippy::too_many_arguments)]
    fn stepper_control<C: SettingChange + 'static>(
        &self,
        cx: &mut Context<Self>,
        id: &'static str,
        value: u32,
        unit: String,
        names: (String, String),
        ladder: &'static [u32],
        read: fn(&Settings) -> u32,
        make: fn(u32) -> C,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let subtle = themes::parla_tokens(cx.theme()).text_subtle;
        let (dec_name, inc_name) = names;
        let end = |this: &Self, cx: &mut Context<Self>, up: bool, name: String| {
            let (suffix, label) = if up {
                ("inc", "+")
            } else {
                ("dec", "\u{2212}")
            };
            // At the end of the ladder the button is muted and does nothing.
            let enabled = step(ladder, value, up) != value;
            let el = stepper_button(format!("{id}-{suffix}"), label, name, enabled, &colors);
            if !enabled {
                return el;
            }
            this.wire(el, cx, move |v, _, cx| {
                let now = read(&v.settings(cx));
                let next = step(ladder, now, up);
                if next != now {
                    v.apply(make(next), cx);
                }
            })
        };
        let dec = end(self, cx, false, dec_name);
        let inc = end(self, cx, true, inc_name);
        stepper(dec, value.to_string(), unit, inc, &colors, subtle)
    }

    /// A button for an action that cannot be undone: the danger text colour.
    fn danger_button(
        &self,
        id: &'static str,
        label: String,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = *cx.theme().colors();
        // The error colour as text on the surface (`danger_text` is the text
        // on a danger fill).
        let danger = cx.theme().status().error;
        let el = compact(button(id, label, ButtonKind::Secondary, true, &colors))
            .text_color(danger)
            .debug_selector(|| id.to_string());
        self.wire(el, cx, f)
    }

    /// Delete all data, after the same confirmation as `app::DeleteAllData`;
    /// the main window then shows sign-in.
    fn delete_all_button(
        &self,
        id: &'static str,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        self.danger_button(
            id,
            t!(l, "settings-danger-delete-button").into_owned(),
            cx,
            |v, window, cx| {
                let l = i18n::gpui::localizer(cx);
                v.confirm(
                    t!(l, "shell-delete-all-title").into_owned(),
                    t!(l, "shell-delete-all-detail").into_owned(),
                    t!(l, "shell-delete-all-confirm").into_owned(),
                    window,
                    cx,
                    |_, _, cx| {
                        with_root(cx, |root, _, cx| root.delete_all_now(cx));
                    },
                );
            },
        )
    }

    /// [`Self::section`] with a title already formatted (one with a count).
    fn section_titled(
        l: &i18n::Localizer,
        title: String,
        desc: &str,
        body: impl IntoElement,
        cx: &App,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        settings_section(
            title,
            l.format("parla", desc, None).into_owned(),
            body,
            false,
            &colors,
            tokens.text_subtle,
            tokens.border_subtle,
        )
    }
}

/// A theme name as an id fragment: lowercase letters and digits, the rest `-`.
fn slug(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// The settings folder for the footer, with the home folder as `~`.
fn config_dir_text(file: &Path) -> String {
    let dir = file.parent().unwrap_or(file);
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = dir.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    dir.display().to_string()
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.lock_sub.is_none()
            && let Some(h) = cx
                .try_global::<crate::idle_lock::IdleLockHandle>()
                .map(|h| h.0.clone())
        {
            self.lock_sub = Some(cx.observe(&h, |_, _, cx| cx.notify()));
        }
        if crate::idle_lock::is_locked(cx) {
            let focus = self
                .lock_focus
                .get_or_insert_with(|| cx.focus_handle())
                .clone();
            // The page's own focus comes back after unlock.
            self.focused = false;
            return crate::idle_lock::cover(&focus, window, cx).into_any_element();
        }
        if !self.focused {
            // One focus target for the window so Tab starts at the first control.
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            self.focus = Some(focus);
            self.focused = true;
        }
        if self.workspaces_sub.is_none()
            && let Some(w) = cx
                .try_global::<crate::workspaces::WorkspacesHandle>()
                .map(|h| h.0.clone())
        {
            self.watch_sessions(&w, cx);
            self.workspaces_sub = Some(cx.subscribe(
                &w,
                |v, w, ev: &crate::workspaces::WorkspacesEvent, cx| {
                    use crate::workspaces::WorkspacesEvent as E;
                    if matches!(ev, E::Changed | E::StartFailed(..)) {
                        v.watch_sessions(&w, cx);
                        cx.notify();
                    }
                },
            ));
        }
        let colors = *cx.theme().colors();
        let l = i18n::gpui::localizer(cx);
        let page = match self.page {
            Page::General => self.general(window, &l, cx),
            Page::Appearance => self.appearance(&l, cx),
            Page::Notifications => self.notifications(window, &l, cx),
            Page::Keymap => self.keymap_page(window, &l, cx),
            Page::Workspaces | Page::Privacy => self.accounts_page(&l, cx),
            Page::Calls => self.calls_page(window, &l, cx),
            Page::Mcp => self.mcp_page(&l, cx),
            Page::Advanced => self.advanced_page(&l, cx),
            other => self.placeholder(other, &l, cx),
        };
        let header = self.header(&l, cx);
        let nav = self.nav(&l, cx);
        if Self::store_error(cx).is_some() {
            self.pending.clear();
        }
        let notice = self.notice_text(cx).map(|message| {
            notice(
                "settings-error",
                message,
                themes::parla_tokens(cx.theme()).danger_soft,
                &colors,
            )
        });
        let page = div().flex().flex_col().children(notice).child(page);
        let ui_font = components::live_settings::ui_font(cx);
        div()
            .id("settings-window")
            .when_some(self.focus.as_ref(), |d, f| d.track_focus(f))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .font_family(ui_font.0)
            .text_size(ui_font.1)
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "tab" {
                    if ev.keystroke.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(
                div().flex().flex_1().min_h_0().child(nav).child(
                    div()
                        .id("settings-page")
                        .flex_1()
                        .min_w_0()
                        .overflow_y_scroll()
                        .px(metrics::SPACE_9)
                        .pt(metrics::SPACE_8)
                        .child(page),
                ),
            )
            .idle_input()
            .into_any_element()
    }
}

/// The open Settings window, so a second open focuses it.
struct OpenWindow(WindowHandle<SettingsView>);

impl Global for OpenWindow {}

/// Opens the Settings window on `settings_file`, or focuses the one already open.
pub fn open_at(cx: &mut App, settings_file: PathBuf, version: &str) {
    open_window(cx, settings_file, version, None);
}

/// [`open_at`] showing `page` (the open window switches to it).
pub fn open_on(cx: &mut App, settings_file: PathBuf, version: &str, page: Page) {
    open_window(cx, settings_file, version, Some(page));
}

fn open_window(cx: &mut App, settings_file: PathBuf, version: &str, page: Option<Page>) {
    if let Some(OpenWindow(handle)) = cx.try_global::<OpenWindow>() {
        let handle = *handle;
        let shown = handle.update(cx, |v, window, cx| {
            if let Some(page) = page {
                v.select_page(page, cx);
            }
            window.activate_window();
        });
        if shown.is_ok() {
            cx.activate(true);
            return;
        }
    }
    let bounds = Bounds::centered(
        None,
        size(
            metrics::LAYOUT_WINDOW_SETTINGS_WIDTH,
            metrics::LAYOUT_WINDOW_SETTINGS_HEIGHT,
        ),
        cx,
    );
    let version = version.to_string();
    if let Ok(handle) = cx.open_window(crate::window_options::settings(bounds), move |_, cx| {
        cx.new(|_| match page {
            Some(page) => view_on(settings_file, version, page),
            None => view(settings_file, version),
        })
    }) {
        cx.set_global(OpenWindow(handle));
        cx.activate(true);
    }
}
