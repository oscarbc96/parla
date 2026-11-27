//! The validated manifest. Every value here has passed
//! the rules in `validate.rs`; the shapes of `exec` and renderer patterns
//! are only parsed here and matched by the host at run time.

use std::fmt;

use semver::Version;
use serde::Deserialize;

use super::areas::{ReadArea, WriteArea};

/// Default guest memory cap in MiB. The one
/// definition; the engine reads it from here.
pub const DEFAULT_MEMORY_MB: u32 = 16;
/// Largest guest memory cap a manifest may ask for, in MiB.
pub const MAX_MEMORY_MB: u32 = 64;
/// Most link patterns one renderer may declare.
pub const MAX_RENDERER_PATTERNS: usize = 16;

/// Size, count and length caps on a manifest. Lengths
/// are in characters.
pub mod limits {
    pub use super::{DEFAULT_MEMORY_MB, MAX_MEMORY_MB, MAX_RENDERER_PATTERNS};

    /// Largest `plugin.toml`, checked before parsing.
    pub const MAX_SOURCE_BYTES: usize = 64 * 1024;
    /// `name`.
    pub const MAX_NAME_CHARS: usize = 64;
    /// Titles of contributions and settings, `usage`, icon pack names.
    pub const MAX_TITLE_CHARS: usize = 64;
    /// `description` of the plugin, slash commands, MCP tools and settings.
    pub const MAX_DESCRIPTION_CHARS: usize = 280;
    /// One entry of `authors`.
    pub const MAX_AUTHOR_CHARS: usize = 64;
    pub const MAX_AUTHORS: usize = 16;
    /// A Fluent key used for a manifest string.
    pub const MAX_FLUENT_KEY_CHARS: usize = 64;
    /// `capabilities.network` entries.
    pub const MAX_NETWORK_HOSTS: usize = 16;
    /// `[[capabilities.exec]]` entries.
    pub const MAX_EXEC_RULES: usize = 4;
    /// Shapes in one exec rule's `allow`.
    pub const MAX_EXEC_SHAPES: usize = 32;
    /// Arguments in one shape.
    pub const MAX_SHAPE_ARGS: usize = 16;
    /// `[[settings]]` entries.
    pub const MAX_SETTINGS: usize = 32;
    /// Entries of each contribution kind, theme files and icon packs.
    pub const MAX_CONTRIBUTIONS: usize = 32;
}

/// Where a manifest comes from; `[dev]` is read only in dev mode (§15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestSource {
    /// An installed or staged `.parlaplugin`.
    Package,
    /// A folder loaded with "Load from folder" in dev mode.
    Dev,
}

/// A plugin id: lowercase `[a-z0-9-]`, 3 to 40 characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginId(pub(super) String);

impl PluginId {
    /// `id` when it is a valid plugin id (an id read back from `app.db`).
    pub fn parse(id: &str) -> Option<Self> {
        super::validate::is_plugin_id(id).then(|| Self(id.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A manifest string: a literal, or a key into the plugin's own Fluent
/// files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalisedText {
    Literal(String),
    /// `{ key = "..." }`
    Key(String),
}

impl<'de> Deserialize<'de> for LocalisedText {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Keyed {
            key: String,
        }
        #[derive(Deserialize)]
        #[serde(untagged, expecting = "a string or { key = \"...\" }")]
        enum Raw {
            Literal(String),
            Keyed(Keyed),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Literal(s) => Self::Literal(s),
            Raw::Keyed(k) => Self::Key(k.key),
        })
    }
}

/// Registry category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Panels,
    SlashCommands,
    Media,
    Themes,
    IconPacks,
    Renderers,
    MessageActions,
    Other,
}

/// An event that loads the plugin. There is no
/// `startup` event.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Activation {
    /// `command:<namespace>::<id>`
    Command(String),
    /// `slash:/<name>`
    Slash(String),
    MessageAction(String),
    Transform(String),
    Render(String),
    Media(String),
    Panel(String),
    Mcp(String),
    /// `hook:<event>`, manifest spelling (`channel_open`).
    Hook(String),
    /// `filter`: the notification filter.
    Filter,
}

impl Activation {
    pub(super) const PREFIXES: &[&str] = &[
        "command",
        "slash",
        "message_action",
        "transform",
        "render",
        "media",
        "panel",
        "mcp",
        "hook",
    ];

    pub(super) fn from_parts(prefix: &str, arg: String) -> Option<Self> {
        Some(match prefix {
            "command" => Self::Command(arg),
            "slash" => Self::Slash(arg),
            "message_action" => Self::MessageAction(arg),
            "transform" => Self::Transform(arg),
            "render" => Self::Render(arg),
            "media" => Self::Media(arg),
            "panel" => Self::Panel(arg),
            "mcp" => Self::Mcp(arg),
            "hook" => Self::Hook(arg),
            _ => return None,
        })
    }
}

impl fmt::Display for Activation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (prefix, arg) = match self {
            Self::Filter => return f.write_str("filter"),
            Self::Command(a) => ("command", a),
            Self::Slash(a) => ("slash", a),
            Self::MessageAction(a) => ("message_action", a),
            Self::Transform(a) => ("transform", a),
            Self::Render(a) => ("render", a),
            Self::Media(a) => ("media", a),
            Self::Panel(a) => ("panel", a),
            Self::Mcp(a) => ("mcp", a),
            Self::Hook(a) => ("hook", a),
        };
        write!(f, "{prefix}:{arg}")
    }
}

/// `[limits]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Guest memory cap in MiB, 1 to [`MAX_MEMORY_MB`].
    pub memory_mb: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_mb: DEFAULT_MEMORY_MB,
        }
    }
}

/// `[capabilities]`. What the manifest declares; the grant
/// the user accepts is stored and checked separately.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapabilitySet {
    pub workspace_read: Vec<ReadArea>,
    pub workspace_write: Vec<WriteArea>,
    pub network: Vec<NetworkHost>,
    pub storage: bool,
    pub windows: bool,
    pub panel: bool,
    pub notifications: bool,
    pub clipboard_write: bool,
    pub exec: Vec<ExecRule>,
}

impl CapabilitySet {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// An exact host the plugin may reach over https.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetworkHost(pub(super) String);

impl NetworkHost {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `localhost`, shown on the card as "this computer".
    pub fn is_local(&self) -> bool {
        self.0 == "localhost"
    }

    /// `h` if it passes the manifest's host rules.
    pub fn parse(h: &str) -> Option<Self> {
        super::validate::network_host(h)
            .is_ok()
            .then(|| Self(h.to_owned()))
    }
}

/// Stored by name in a grant; read back through the same host rules.
impl serde::Serialize for NetworkHost {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for NetworkHost {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let h = String::deserialize(d)?;
        Self::parse(&h)
            .ok_or_else(|| serde::de::Error::custom(format_args!("`{h}` is not a host name")))
    }
}

/// One `[[capabilities.exec]]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecRule {
    /// A bare program name, resolved on the login `PATH` at consent.
    pub program: String,
    pub allow: Vec<ArgShape>,
}

/// An allowed argv shape; matched by the host at run time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgShape(pub Vec<ArgPattern>);

/// One element of an [`ArgShape`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgPattern {
    /// Must equal the argument.
    Literal(String),
    /// Contains `*`, which matches within this one argument.
    Glob(String),
}

impl ArgPattern {
    /// The manifest spelling.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Literal(s) | Self::Glob(s) => s,
        }
    }

    /// A glob when `s` contains `*`, else a literal.
    pub fn from_text(s: String) -> Self {
        if s.contains('*') {
            Self::Glob(s)
        } else {
            Self::Literal(s)
        }
    }
}

/// What the plugin adds to Parla.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contributions {
    pub commands: Vec<Command>,
    pub slash: Vec<Slash>,
    pub message_actions: Vec<MessageAction>,
    pub composer_transforms: Vec<ComposerTransform>,
    pub renderers: Vec<Renderer>,
    pub media_providers: Vec<MediaProvider>,
    pub panels: Vec<Panel>,
    pub mcp_tools: Vec<McpTool>,
    pub hooks: Option<Hooks>,
}

impl Contributions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub title: LocalisedText,
    /// A suggestion applied only when the key is unbound.
    pub default_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slash {
    /// Starts with `/`.
    pub name: String,
    pub description: LocalisedText,
    pub usage: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageAction {
    pub id: String,
    pub title: LocalisedText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransformKind {
    Check,
    Expand,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerTransform {
    pub id: String,
    pub kind: TransformKind,
    pub title: LocalisedText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Renderer {
    pub id: String,
    /// `host/path` globs: the host is literal, `*` matches within a path
    /// segment. At most [`MAX_RENDERER_PATTERNS`].
    pub patterns: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Gif,
    Sticker,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaProvider {
    pub id: String,
    pub title: LocalisedText,
    pub kinds: Vec<MediaKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub id: String,
    pub title: LocalisedText,
    /// A Parla icon name, or `plugin` for the package's `icon.svg`.
    pub icon: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpKind {
    Read,
    Write,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpTool {
    /// Exposed as `<plugin id>.<name>`.
    pub name: String,
    pub description: String,
    pub kind: McpKind,
    /// Package-relative path of the JSON schema.
    pub input_schema: String,
}

/// Event a hook receives, manifest spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    Message,
    Reaction,
    ChannelOpen,
}

impl HookEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Reaction => "reaction",
            Self::ChannelOpen => "channel_open",
        }
    }
}

/// `[hooks]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hooks {
    pub events: Vec<HookEvent>,
    pub notification_filter: bool,
}

/// One `[[settings]]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingSpec {
    /// `[a-z0-9_]`.
    pub key: String,
    pub kind: SettingKind,
    /// Matches `kind`; a secret has none.
    pub default: Option<SettingValue>,
    pub title: LocalisedText,
    pub description: Option<LocalisedText>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingKind {
    Boolean,
    Integer {
        min: Option<i64>,
        max: Option<i64>,
    },
    String,
    Enum {
        options: Vec<String>,
    },
    /// Kept in the secret store, never in the settings file.
    Secret,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingValue {
    Boolean(bool),
    Integer(i64),
    String(String),
}

/// `[themes]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Themes {
    pub files: Vec<String>,
}

/// One `[[icon_packs]]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconPack {
    pub name: String,
    pub dir: String,
}

/// `[dev]`, read only for plugins loaded from a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevSection {
    /// Path of the built component, relative to the folder.
    pub wasm: Option<String>,
}

/// A parsed and validated `plugin.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub id: PluginId,
    /// Action namespace: `namespace`, or the id with `-` turned into `_`.
    pub namespace: String,
    pub name: LocalisedText,
    pub version: Version,
    /// `(major, minor)` of the plugin API it was built for.
    pub api: (u16, u16),
    pub description: Option<LocalisedText>,
    pub authors: Vec<String>,
    pub repository: Option<String>,
    pub license: Option<String>,
    pub categories: Vec<Category>,
    pub min_app: Option<Version>,
    /// As declared, or filled from the contributions when absent.
    pub activation: Vec<Activation>,
    pub limits: Limits,
    pub capabilities: CapabilitySet,
    pub contributions: Contributions,
    pub settings: Vec<SettingSpec>,
    pub themes: Option<Themes>,
    pub icon_packs: Vec<IconPack>,
    pub dev: Option<DevSection>,
}

impl Manifest {
    /// A data plugin: themes and/or icon packs only; no code runs.
    pub fn is_data(&self) -> bool {
        self.capabilities.is_empty()
            && self.contributions.is_empty()
            && self.settings.is_empty()
            && (self.themes.is_some() || !self.icon_packs.is_empty())
    }

    /// The action name of command `id`: `<namespace>::<id>`.
    pub fn action_name(&self, id: &str) -> String {
        format!("{}::{id}", self.namespace)
    }

    /// The events the contributions imply, in manifest order (what the
    /// packer writes into `activation`).
    pub fn implied_activation(&self) -> Vec<Activation> {
        implied(&self.namespace, &self.contributions)
            .into_iter()
            .map(|(a, _)| a)
            .collect()
    }
}

/// Implied events with the key of the contribution that implies each.
pub(super) fn implied(namespace: &str, c: &Contributions) -> Vec<(Activation, String)> {
    let mut out = Vec::new();
    for (i, x) in c.commands.iter().enumerate() {
        out.push((
            Activation::Command(format!("{namespace}::{}", x.id)),
            format!("commands[{i}]"),
        ));
    }
    for (i, x) in c.slash.iter().enumerate() {
        out.push((Activation::Slash(x.name.clone()), format!("slash[{i}]")));
    }
    for (i, x) in c.message_actions.iter().enumerate() {
        out.push((
            Activation::MessageAction(x.id.clone()),
            format!("message_actions[{i}]"),
        ));
    }
    for (i, x) in c.composer_transforms.iter().enumerate() {
        out.push((
            Activation::Transform(x.id.clone()),
            format!("composer_transforms[{i}]"),
        ));
    }
    for (i, x) in c.renderers.iter().enumerate() {
        out.push((Activation::Render(x.id.clone()), format!("renderers[{i}]")));
    }
    for (i, x) in c.media_providers.iter().enumerate() {
        out.push((
            Activation::Media(x.id.clone()),
            format!("media_providers[{i}]"),
        ));
    }
    for (i, x) in c.panels.iter().enumerate() {
        out.push((Activation::Panel(x.id.clone()), format!("panels[{i}]")));
    }
    for (i, x) in c.mcp_tools.iter().enumerate() {
        out.push((Activation::Mcp(x.name.clone()), format!("mcp_tools[{i}]")));
    }
    if let Some(h) = &c.hooks {
        for (i, e) in h.events.iter().enumerate() {
            out.push((
                Activation::Hook(e.as_str().to_owned()),
                format!("hooks.events[{i}]"),
            ));
        }
        if h.notification_filter {
            out.push((Activation::Filter, "hooks.notification_filter".to_owned()));
        }
    }
    out
}
