use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Identifier of a chat provider (for example `fake`), as registered by the app.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ProviderId(pub String);

/// Provider-side workspace identifier. Unique only within its provider.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct WorkspaceId(pub String);

/// Globally unique workspace key, text form `<provider>/<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WorkspaceKey {
    pub provider: ProviderId,
    pub id: WorkspaceId,
}

impl WorkspaceKey {
    pub fn new(provider: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            provider: ProviderId(provider.into()),
            id: WorkspaceId(id.into()),
        }
    }

    /// The `(provider, id)` path components for per-workspace files: `/` and
    /// `\` become `_`, and `.` or `..` become `_`, so neither can leave the
    /// directory they are joined to.
    pub fn path_parts(&self) -> (String, String) {
        fn safe(s: &str) -> String {
            let s: String = s
                .chars()
                .map(|c| if matches!(c, '/' | '\\') { '_' } else { c })
                .collect();
            if s == ".." || s == "." { "_".into() } else { s }
        }
        (safe(&self.provider.0), safe(&self.id.0))
    }
}

impl std::fmt::Display for WorkspaceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.provider.0, self.id.0)
    }
}

impl std::str::FromStr for WorkspaceKey {
    type Err = String;

    /// Parses `<provider>/<id>`; both parts must be non-empty and the provider
    /// must not contain `/`. The id may not contain path separators.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.split_once('/') {
            Some((provider, id))
                if !provider.is_empty() && !id.is_empty() && !id.contains(['/', '\\']) =>
            {
                Ok(Self::new(provider, id))
            }
            _ => Err(format!("workspace key {text:?} is not <provider>/<id>")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ThemeSelection {
    pub mode: ThemeMode,
    pub light: String,
    pub dark: String,
}

/// Name of the built-in light theme.
pub const DEFAULT_LIGHT_THEME: &str = "Parla Light";
/// Name of the built-in dark theme.
pub const DEFAULT_DARK_THEME: &str = "Parla Dark";

impl Default for ThemeSelection {
    fn default() -> Self {
        Self {
            mode: ThemeMode::System,
            light: DEFAULT_LIGHT_THEME.into(),
            dark: DEFAULT_DARK_THEME.into(),
        }
    }
}

/// Global accent colour preset (Appearance page); `brand` keeps the theme's own accent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccentPreset {
    #[default]
    Brand,
    Ember,
    Amber,
    Coral,
    Ink,
}

impl AccentPreset {
    /// Every preset, in the order the Appearance page lists them.
    pub const ALL: [AccentPreset; 5] = [
        AccentPreset::Brand,
        AccentPreset::Ember,
        AccentPreset::Amber,
        AccentPreset::Coral,
        AccentPreset::Ink,
    ];

    /// The `snake_case` name used in the settings file and in ids.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Brand => "brand",
            Self::Ember => "ember",
            Self::Amber => "amber",
            Self::Coral => "coral",
            Self::Ink => "ink",
        }
    }
}

/// Which key sends from the composer. Shift-Enter always inserts a newline; a
/// plain Enter does when Cmd-Enter sends, and Cmd-Enter does nothing when Enter sends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendKey {
    /// Enter sends; Shift-Enter makes a newline.
    #[default]
    Enter,
    /// Cmd-Enter (Ctrl-Enter off macOS) sends; Enter makes a newline.
    CmdEnter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    Cozy,
}

/// When the Dock icon shows (macOS).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DockIcon {
    /// Always, even while the window is closed and only the menu-bar icon is left.
    #[default]
    Always,
    /// Only while a window is open (needs the `tray` icon to get back in).
    WithWindow,
    /// Never: menu bar only (needs the `tray` icon; without it the Dock icon stays).
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSort {
    Alphabetical,
    Recent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SidebarSettings {
    pub show_unreads_section: bool,
    pub sort: SidebarSort,
    /// The user's own sidebar sections, in order. Local to Parla (never sent to
    /// the provider); the app keeps them in the workspace's file
    /// (`workspaces/<provider>/<id>.json`), as conversation ids are per workspace.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<CustomSection>,
}

impl Default for SidebarSettings {
    fn default() -> Self {
        Self {
            show_unreads_section: true,
            sort: SidebarSort::Alphabetical,
            sections: Vec::new(),
        }
    }
}

/// One of the user's own sidebar sections: a name and the conversations it
/// holds (provider conversation ids). Listed above Channels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CustomSection {
    /// Stable within the workspace; the folded state is kept under it.
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub conversations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Clock {
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "12h")]
    H12,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InlineImages {
    Always,
    OnClick,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MessageSettings {
    pub show_avatars: bool,
    pub clock: Clock,
    pub inline_images: InlineImages,
}

impl Default for MessageSettings {
    fn default() -> Self {
        Self {
            show_avatars: true,
            clock: Clock::H24,
            inline_images: InlineImages::OnClick,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesktopNotifications {
    All,
    Mentions,
    None,
}

/// A day of the week, spelled `mon` .. `sun` in the file.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    pub const ALL: [Weekday; 7] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];
}

/// Local-time window, `HH:MM` each. It may wrap midnight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QuietHours {
    pub from: String,
    pub to: String,
    /// Days the window starts on (a window that wraps midnight belongs to the
    /// day it starts). Empty: every day.
    #[serde(default)]
    pub days: Vec<Weekday>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct NotificationSettings {
    pub desktop: DesktopNotifications,
    pub keywords: Vec<String>,
    pub sound: String,
    pub quiet_hours: Option<QuietHours>,
    /// Show the message text in a notification. Off: only the sender and the
    /// channel.
    pub show_preview: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            desktop: DesktopNotifications::Mentions,
            keywords: Vec::new(),
            sound: "subtle".into(),
            quiet_hours: None,
            show_preview: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PerformanceSettings {
    pub image_cache_mb: u32,
    pub message_window: u32,
    pub lean_after_minutes: u32,
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            image_cache_mb: 32,
            message_window: 200,
            lean_after_minutes: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SendLaterSettings {
    pub presets: Vec<String>,
    /// IANA zone name overriding the profile zone for scheduling.
    pub timezone: Option<String>,
}

impl Default for SendLaterSettings {
    fn default() -> Self {
        Self {
            presets: vec!["tomorrow 09:00".into(), "monday 09:00".into()],
            timezone: None,
        }
    }
}

/// The Lock store when idle choices the Settings page offers, in minutes
/// (`0` is off). The file accepts any number.
pub const LOCK_AFTER_STEPS: &[u32] = &[0, 5, 15, 30, 60];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct StorageSettings {
    pub cache_mb: u32,
    pub retention_days: u32,
    /// Minutes without input after which Parla closes the encrypted store,
    /// forgets the store keys it holds in memory and shows a lock screen;
    /// `0` is off. Applies at once.
    pub lock_after_minutes: u32,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            cache_mb: 8,
            retention_days: 90,
            lock_after_minutes: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GifSendAs {
    Link,
    Upload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GifAutoplay {
    Visible,
    Hover,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct GifSettings {
    pub providers: Vec<String>,
    pub rating: String,
    pub send_as: GifSendAs,
    pub folder: Option<String>,
    pub autoplay_in_messages: GifAutoplay,
}

impl Default for GifSettings {
    fn default() -> Self {
        Self {
            providers: vec!["giphy".into(), "klipy".into()],
            rating: "pg-13".into(),
            send_as: GifSendAs::Link,
            folder: None,
            autoplay_in_messages: GifAutoplay::Visible,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JoinWith {
    Native,
    Window,
    Browser,
    ProviderApp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CallSettings {
    pub ring: bool,
    pub ringtone: String,
    pub join_with: JoinWith,
    pub mute_on_join: bool,
    pub camera_on_join: bool,
    /// Microphone for native calls, by name; `None` follows the system
    /// default. A named device that is not connected falls back to the
    /// default until it is.
    pub input_device: Option<String>,
    /// Speaker for native calls, by name; `None` follows the system default.
    pub output_device: Option<String>,
}

impl Default for CallSettings {
    fn default() -> Self {
        Self {
            ring: true,
            ringtone: "parla-chime".into(),
            join_with: JoinWith::Native,
            mute_on_join: true,
            camera_on_join: false,
            input_device: None,
            output_device: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UpdateSettings {
    pub check: bool,
    /// A release tag the user chose to skip; a newer version is still offered.
    pub skip: Option<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check: true,
            skip: None,
        }
    }
}

/// Per-workspace MCP policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct McpWorkspaceSettings {
    /// Whether MCP clients may see this workspace at all (off by default).
    pub enabled: bool,
    pub allow_writes: bool,
    pub confirm_writes: bool,
    pub include_dms: bool,
    pub allow_channels: Vec<String>,
    pub deny_channels: Vec<String>,
}

impl Default for McpWorkspaceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_writes: false,
            confirm_writes: true,
            include_dms: false,
            allow_channels: Vec::new(),
            deny_channels: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct McpSettings {
    pub enabled: bool,
    /// Days the MCP audit log keeps rows before they are pruned.
    pub audit_retention_days: u32,
    /// Keyed by workspace key, `<provider>/<id>`.
    pub workspaces: BTreeMap<String, McpWorkspaceSettings>,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            audit_retention_days: 90,
            workspaces: BTreeMap::new(),
        }
    }
}

impl McpSettings {
    /// The policy for `key`: its entry, or the default (disabled) when the
    /// workspace has none. Entries whose key is not a valid workspace key never
    /// match (they are reported when the file loads).
    pub fn workspace(&self, key: &WorkspaceKey) -> McpWorkspaceSettings {
        self.workspaces
            .get(&key.to_string())
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LogSettings {
    pub include_content: bool,
    /// The most detailed lines Parla writes to its log: `error`, `info` or
    /// `debug`. Applies at once.
    pub level: LogLevel,
}

/// How much Parla writes to `parla.log`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Only failures.
    Error,
    /// Failures, warnings and the main events (the default).
    #[default]
    Info,
    /// Everything, for a bug report.
    Debug,
}

/// What happens when Parla quits unexpectedly. Reports stay on this device
/// in every mode; nothing is sent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CrashReports {
    /// No report is written.
    Off,
    /// A redacted report is written, and the next launch asks whether to
    /// show or delete it (the default).
    #[default]
    Ask,
    /// A redacted report is written and kept without asking.
    Local,
}

/// The `locale` setting: `"system"` (follow the OS) or a code listed in
/// `i18n/locales.toml`, or the pseudo-locale `en-XA`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LocaleSetting {
    #[default]
    System,
    Code(String),
}

impl LocaleSetting {
    pub fn pref(&self) -> i18n::LocalePref {
        match self {
            Self::System => i18n::LocalePref::System,
            Self::Code(c) => c
                .parse()
                .map(i18n::LocalePref::Code)
                .unwrap_or(i18n::LocalePref::System),
        }
    }

    fn allowed() -> Vec<String> {
        let mut v = vec!["system".to_string()];
        v.extend(
            i18n::LocaleSet::embedded()
                .locales
                .iter()
                .map(|l| l.code.to_string()),
        );
        v.push(i18n::PSEUDO.to_string());
        v
    }
}

impl Serialize for LocaleSetting {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::System => s.serialize_str("system"),
            Self::Code(c) => s.serialize_str(c),
        }
    }
}

impl<'de> Deserialize<'de> for LocaleSetting {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        if raw == "system" {
            return Ok(Self::System);
        }
        let allowed = Self::allowed();
        if allowed.contains(&raw) {
            Ok(Self::Code(raw))
        } else {
            Err(serde::de::Error::custom(format!(
                "unknown locale {raw:?}, expected one of {}",
                allowed.join(", ")
            )))
        }
    }
}

impl JsonSchema for LocaleSetting {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LocaleSetting".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "enum": Self::allowed() })
    }
}

/// Smallest and largest `ui_font_size`; values outside are clamped.
pub const UI_FONT_SIZE_RANGE: (f32, f32) = (10.0, 24.0);

fn clamp_ui_font_size<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    let v = <f32 as Deserialize>::deserialize(d)?;
    Ok(v.clamp(UI_FONT_SIZE_RANGE.0, UI_FONT_SIZE_RANGE.1))
}

/// Window behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WindowSettings {
    /// Reopen the main window at its last size and position (kept on a
    /// visible display).
    pub restore: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self { restore: true }
    }
}

/// Plugin preferences. `app.db` holds the
/// install state; this is only what the user prefers. Plugin ids are
/// `[a-z0-9-]`, so they never collide with `auto_update` or `dev_paths`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PluginsSettings {
    /// Apply updates that ask for no new permissions when Parla starts.
    pub auto_update: bool,
    /// Folders loaded as dev plugins (dev mode).
    pub dev_paths: Vec<String>,
    /// One entry per plugin id. Entries for ids that are not installed are
    /// kept and ignored.
    #[serde(flatten)]
    pub entries: BTreeMap<String, PluginEntry>,
}

impl PluginsSettings {
    /// The entry for plugin `id`, if the file has one.
    pub fn entry(&self, id: &str) -> Option<&PluginEntry> {
        self.entries.get(id)
    }
}

/// One plugin's preferences under `plugins.<id>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PluginEntry {
    /// The switch on the plugin's settings card. `true` cannot revive a
    /// quarantined or blocked plugin.
    pub enabled: bool,
    /// Workspace keys (`<provider>/<id>`) the plugin runs in; absent means all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<Vec<String>>,
    /// Values of the plugin's own `[[settings]]`. Settings of type `secret`
    /// live in the keychain, never here.
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub config: serde_json::Map<String, serde_json::Value>,
}

impl Default for PluginEntry {
    fn default() -> Self {
        Self {
            enabled: true,
            workspaces: None,
            config: serde_json::Map::new(),
        }
    }
}

/// A plugin setting declared `secret` was about to be written to the
/// settings file; secrets go to the keychain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretSettingRejected {
    pub key: String,
}

impl std::fmt::Display for SecretSettingRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "plugin setting {:?} is secret and is never written to settings",
            self.key
        )
    }
}

impl std::error::Error for SecretSettingRejected {}

impl PluginEntry {
    /// Whether the plugin runs in workspace `key`: no list means every one.
    pub fn applies_to(&self, key: &WorkspaceKey) -> bool {
        match &self.workspaces {
            None => true,
            Some(list) => {
                let key = key.to_string();
                list.contains(&key)
            }
        }
    }

    /// Sets config `key` to `value`. `secret` is whether the manifest
    /// declares the key `secret`; such a key is rejected and nothing changes.
    pub fn set_config(
        &mut self,
        key: &str,
        value: serde_json::Value,
        secret: bool,
    ) -> Result<(), SecretSettingRejected> {
        if secret {
            return Err(SecretSettingRejected { key: key.into() });
        }
        self.config.insert(key.into(), value);
        Ok(())
    }
}

/// All user-visible settings. Unknown keys are tolerated (see `unknown_keys`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Settings {
    /// Editor pointer to the JSON schema shipped beside the file; ignored.
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    pub theme: ThemeSelection,
    pub ui_font_family: String,
    #[serde(deserialize_with = "clamp_ui_font_size")]
    #[schemars(range(min = 10.0, max = 24.0))]
    pub ui_font_size: f32,
    pub buffer_font_family: String,
    pub locale: LocaleSetting,
    pub density: Density,
    /// Global accent preset, applied live to both themes. Not the workspace `accent`.
    pub accent_preset: AccentPreset,
    /// Composer send key.
    pub send_key: SendKey,
    pub sidebar: SidebarSettings,
    pub messages: MessageSettings,
    pub notifications: NotificationSettings,
    pub performance: PerformanceSettings,
    pub send_later: SendLaterSettings,
    pub storage: StorageSettings,
    pub gifs: GifSettings,
    pub calls: CallSettings,
    pub updates: UpdateSettings,
    pub mcp: McpSettings,
    pub log: LogSettings,
    /// Local crash reports: `off`, `ask` or `local`. Never sent anywhere.
    pub crash_reports: CrashReports,
    pub windows: WindowSettings,
    /// Menu-bar (tray) icon with workspace badges; closing the window keeps
    /// the app running in it. Off: closing the last window quits.
    pub tray: bool,
    /// macOS Dock icon: `always`, `with_window` to hide it while the window
    /// is closed (the `tray` icon keeps running), or `never` for the menu bar
    /// only. Ignored without `tray`.
    pub dock_icon: DockIcon,
    /// Register Parla as a login item. A login launch starts hidden, behind the
    /// menu-bar icon, with no window open.
    pub launch_at_login: bool,
    pub plugins: PluginsSettings,
    /// Accent colour as `#rrggbb`, meaningful in a workspace settings file:
    /// the workspace rail and header use it, nothing else.
    pub accent: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: None,
            theme: ThemeSelection::default(),
            ui_font_family: "Inter".into(),
            ui_font_size: 14.0,
            buffer_font_family: "JetBrains Mono".into(),
            locale: LocaleSetting::System,
            density: Density::Compact,
            accent_preset: AccentPreset::Brand,
            send_key: SendKey::Enter,
            sidebar: SidebarSettings::default(),
            messages: MessageSettings::default(),
            notifications: NotificationSettings::default(),
            performance: PerformanceSettings::default(),
            send_later: SendLaterSettings::default(),
            storage: StorageSettings::default(),
            gifs: GifSettings::default(),
            calls: CallSettings::default(),
            updates: UpdateSettings::default(),
            mcp: McpSettings::default(),
            log: LogSettings::default(),
            crash_reports: CrashReports::Ask,
            windows: WindowSettings::default(),
            tray: true,
            dock_icon: DockIcon::Always,
            launch_at_login: false,
            plugins: PluginsSettings::default(),
            accent: None,
        }
    }
}
