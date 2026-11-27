//! The serde shape of `plugin.toml` for API 0.1 (unknown keys denied) and
//! the rules that turn it into a [`Manifest`]. Every rule reports into the
//! [`Sink`] and carries on, so one pass finds every problem.

use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr};

use semver::Version;
use serde::Deserialize;

use super::areas::{ReadArea, WriteArea};
use super::model::limits::*;
use super::model::*;
use super::{ErrorKind, Sink};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawManifest {
    id: String,
    name: LocalisedText,
    namespace: Option<String>,
    version: String,
    api: String,
    description: Option<LocalisedText>,
    #[serde(default)]
    authors: Vec<String>,
    repository: Option<String>,
    license: Option<String>,
    #[serde(default)]
    categories: Vec<Category>,
    min_app: Option<String>,
    activation: Option<Vec<String>>,
    limits: Option<RawLimits>,
    capabilities: Option<RawCapabilities>,
    #[serde(default)]
    commands: Vec<RawCommand>,
    #[serde(default)]
    slash: Vec<RawSlash>,
    #[serde(default)]
    message_actions: Vec<RawMessageAction>,
    #[serde(default)]
    composer_transforms: Vec<RawTransform>,
    #[serde(default)]
    renderers: Vec<RawRenderer>,
    #[serde(default)]
    media_providers: Vec<RawMedia>,
    #[serde(default)]
    panels: Vec<RawPanel>,
    #[serde(default)]
    mcp_tools: Vec<RawMcpTool>,
    hooks: Option<RawHooks>,
    #[serde(default)]
    settings: Vec<RawSetting>,
    themes: Option<RawThemes>,
    #[serde(default)]
    icon_packs: Vec<RawIconPack>,
    dev: Option<RawDev>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLimits {
    memory_mb: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCapabilities {
    #[serde(default)]
    workspace_read: Vec<String>,
    #[serde(default)]
    workspace_write: Vec<String>,
    #[serde(default)]
    network: Vec<String>,
    #[serde(default)]
    storage: bool,
    #[serde(default)]
    windows: bool,
    #[serde(default)]
    panel: bool,
    #[serde(default)]
    notifications: bool,
    #[serde(default)]
    clipboard_write: bool,
    #[serde(default)]
    exec: Vec<RawExec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExec {
    program: String,
    allow: Vec<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    id: String,
    title: LocalisedText,
    default_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSlash {
    name: String,
    description: LocalisedText,
    usage: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMessageAction {
    id: String,
    title: LocalisedText,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTransform {
    id: String,
    kind: TransformKind,
    title: LocalisedText,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRenderer {
    id: String,
    patterns: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMedia {
    id: String,
    title: LocalisedText,
    kinds: Vec<MediaKind>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPanel {
    id: String,
    title: LocalisedText,
    icon: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcpTool {
    name: String,
    description: String,
    kind: McpKind,
    input_schema: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHooks {
    #[serde(default)]
    events: Vec<HookEvent>,
    #[serde(default)]
    notification_filter: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSetting {
    key: String,
    #[serde(rename = "type")]
    ty: String,
    default: Option<toml::Value>,
    min: Option<i64>,
    max: Option<i64>,
    options: Option<Vec<String>>,
    title: LocalisedText,
    description: Option<LocalisedText>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawThemes {
    files: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIconPack {
    name: String,
    dir: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDev {
    wasm: Option<String>,
}

/// Applies every rule; errors go to `sink`, the manifest is returned with
/// placeholders where a value was rejected.
pub(super) fn validate(r: RawManifest, source: ManifestSource, sink: &mut Sink<'_>) -> Manifest {
    bounds(&r, source, sink);
    if !is_plugin_id(&r.id) {
        sink.push(
            "id",
            ErrorKind::InvalidId,
            "an id is 3 to 40 characters of a-z, 0-9 and -",
        );
    }
    let namespace = match r.namespace {
        Some(ns) => {
            if !is_namespace(&ns) {
                sink.push(
                    "namespace",
                    ErrorKind::InvalidNamespace,
                    "a namespace is 1 to 40 characters of a-z, 0-9 and _",
                );
            }
            ns
        }
        None => r.id.replace('-', "_"),
    };
    let version = semver(&r.version, "version", sink);
    let min_app = r.min_app.as_deref().map(|v| semver(v, "min_app", sink));
    let api = api_version(&r.api, sink);

    let limits = Limits {
        memory_mb: match r.limits.and_then(|l| l.memory_mb) {
            None => DEFAULT_MEMORY_MB,
            Some(mb) if (1..=i64::from(MAX_MEMORY_MB)).contains(&mb) => mb as u32,
            Some(_) => {
                sink.push(
                    "limits.memory_mb",
                    ErrorKind::MemoryOutOfRange,
                    format!("memory_mb is 1 to {MAX_MEMORY_MB}"),
                );
                DEFAULT_MEMORY_MB
            }
        },
    };

    let capabilities = r
        .capabilities
        .map(|c| capabilities(c, sink))
        .unwrap_or_default();

    let contributions = Contributions {
        commands: r
            .commands
            .into_iter()
            .map(|x| Command {
                id: x.id,
                title: x.title,
                default_key: x.default_key,
            })
            .collect(),
        slash: r
            .slash
            .into_iter()
            .map(|x| Slash {
                name: x.name,
                description: x.description,
                usage: x.usage,
            })
            .collect(),
        message_actions: r
            .message_actions
            .into_iter()
            .map(|x| MessageAction {
                id: x.id,
                title: x.title,
            })
            .collect(),
        composer_transforms: r
            .composer_transforms
            .into_iter()
            .map(|x| ComposerTransform {
                id: x.id,
                kind: x.kind,
                title: x.title,
            })
            .collect(),
        renderers: r
            .renderers
            .into_iter()
            .map(|x| Renderer {
                id: x.id,
                patterns: x.patterns,
            })
            .collect(),
        media_providers: r
            .media_providers
            .into_iter()
            .map(|x| MediaProvider {
                id: x.id,
                title: x.title,
                kinds: x.kinds,
            })
            .collect(),
        panels: r
            .panels
            .into_iter()
            .map(|x| Panel {
                id: x.id,
                title: x.title,
                icon: x.icon,
            })
            .collect(),
        mcp_tools: r
            .mcp_tools
            .into_iter()
            .map(|x| McpTool {
                name: x.name,
                description: x.description,
                kind: x.kind,
                input_schema: x.input_schema,
            })
            .collect(),
        hooks: r.hooks.map(|h| Hooks {
            events: h.events,
            notification_filter: h.notification_filter,
        }),
    };
    check_contributions(&contributions, sink);

    let activation = activation(r.activation, &namespace, &contributions, sink);

    let settings = r
        .settings
        .into_iter()
        .enumerate()
        .filter_map(|(i, s)| setting(i, s, sink))
        .collect::<Vec<_>>();
    unique(
        settings.iter().map(|s| s.key.as_str()),
        |i| format!("settings[{i}].key"),
        sink,
    );

    let themes = r.themes.map(|t| {
        for (i, f) in t.files.iter().enumerate() {
            let key = format!("themes.files[{i}]");
            path(f, &key, sink);
            in_folder(f, "themes", Some(".json"), &key, sink);
        }
        Themes { files: t.files }
    });
    let icon_packs = r
        .icon_packs
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let key = format!("icon_packs[{i}].dir");
            path(&p.dir, &key, sink);
            in_folder(&p.dir, "icons", None, &key, sink);
            IconPack {
                name: p.name,
                dir: p.dir,
            }
        })
        .collect();
    let dev = r.dev.map(|d| {
        if let Some(w) = &d.wasm {
            path(w, "dev.wasm", sink);
        }
        DevSection { wasm: d.wasm }
    });

    Manifest {
        id: PluginId(r.id),
        namespace,
        name: r.name,
        version,
        api,
        description: r.description,
        authors: r.authors,
        repository: r.repository,
        license: r.license,
        categories: r.categories,
        min_app,
        activation,
        limits,
        capabilities,
        contributions,
        settings,
        themes,
        icon_packs,
        dev,
    }
}

/// Caps, text and the small fields the model keeps as plain strings:
/// counts and lengths, Fluent keys, `default_key`,
/// panel icons, `repository`, repeated hook events and enum options, and
/// `[dev]` outside dev mode.
fn bounds(r: &RawManifest, source: ManifestSource, sink: &mut Sink<'_>) {
    if source == ManifestSource::Package && r.dev.is_some() {
        sink.push(
            "dev",
            ErrorKind::DevOnly,
            "[dev] is read only for a folder loaded in dev mode",
        );
    }
    text(&r.name, "name", MAX_NAME_CHARS, sink);
    if let Some(d) = &r.description {
        text(d, "description", MAX_DESCRIPTION_CHARS, sink);
    }
    count(r.authors.len(), MAX_AUTHORS, "authors", sink);
    for (i, a) in r.authors.iter().enumerate() {
        length(a, MAX_AUTHOR_CHARS, &format!("authors[{i}]"), sink);
    }
    if let Some(repo) = &r.repository
        && !is_https_url(repo)
    {
        sink.push(
            "repository",
            ErrorKind::InvalidRepository,
            "repository is an https URL with a host name",
        );
    }

    if let Some(c) = &r.capabilities {
        count(
            c.network.len(),
            MAX_NETWORK_HOSTS,
            "capabilities.network",
            sink,
        );
        count(c.exec.len(), MAX_EXEC_RULES, "capabilities.exec", sink);
        for (i, e) in c.exec.iter().enumerate() {
            let key = format!("capabilities.exec[{i}].allow");
            count(e.allow.len(), MAX_EXEC_SHAPES, &key, sink);
            for (j, shape) in e.allow.iter().enumerate() {
                count(shape.len(), MAX_SHAPE_ARGS, &format!("{key}[{j}]"), sink);
            }
        }
    }

    count(r.commands.len(), MAX_CONTRIBUTIONS, "commands", sink);
    for (i, x) in r.commands.iter().enumerate() {
        text(
            &x.title,
            &format!("commands[{i}].title"),
            MAX_TITLE_CHARS,
            sink,
        );
        if let Some(k) = &x.default_key
            && !is_keystroke(k)
        {
            sink.push(
                format!("commands[{i}].default_key"),
                ErrorKind::InvalidKey,
                "one keystroke: modifiers (cmd, ctrl, alt, secondary, fn, shift) then a key",
            );
        }
    }
    count(r.slash.len(), MAX_CONTRIBUTIONS, "slash", sink);
    for (i, x) in r.slash.iter().enumerate() {
        text(
            &x.description,
            &format!("slash[{i}].description"),
            MAX_DESCRIPTION_CHARS,
            sink,
        );
        if let Some(u) = &x.usage {
            length(u, MAX_TITLE_CHARS, &format!("slash[{i}].usage"), sink);
        }
    }
    count(
        r.message_actions.len(),
        MAX_CONTRIBUTIONS,
        "message_actions",
        sink,
    );
    for (i, x) in r.message_actions.iter().enumerate() {
        text(
            &x.title,
            &format!("message_actions[{i}].title"),
            MAX_TITLE_CHARS,
            sink,
        );
    }
    count(
        r.composer_transforms.len(),
        MAX_CONTRIBUTIONS,
        "composer_transforms",
        sink,
    );
    for (i, x) in r.composer_transforms.iter().enumerate() {
        text(
            &x.title,
            &format!("composer_transforms[{i}].title"),
            MAX_TITLE_CHARS,
            sink,
        );
    }
    count(r.renderers.len(), MAX_CONTRIBUTIONS, "renderers", sink);
    count(
        r.media_providers.len(),
        MAX_CONTRIBUTIONS,
        "media_providers",
        sink,
    );
    for (i, x) in r.media_providers.iter().enumerate() {
        text(
            &x.title,
            &format!("media_providers[{i}].title"),
            MAX_TITLE_CHARS,
            sink,
        );
    }
    count(r.panels.len(), MAX_CONTRIBUTIONS, "panels", sink);
    for (i, x) in r.panels.iter().enumerate() {
        text(
            &x.title,
            &format!("panels[{i}].title"),
            MAX_TITLE_CHARS,
            sink,
        );
        if !is_icon_name(&x.icon) {
            sink.push(
                format!("panels[{i}].icon"),
                ErrorKind::InvalidIcon,
                "a Parla icon name (a-z, 0-9, - and _) or \"plugin\"",
            );
        }
    }
    count(r.mcp_tools.len(), MAX_CONTRIBUTIONS, "mcp_tools", sink);
    for (i, x) in r.mcp_tools.iter().enumerate() {
        length(
            &x.description,
            MAX_DESCRIPTION_CHARS,
            &format!("mcp_tools[{i}].description"),
            sink,
        );
    }
    if let Some(h) = &r.hooks {
        unique(
            h.events.iter().map(|e| e.as_str()),
            |i| format!("hooks.events[{i}]"),
            sink,
        );
    }

    count(r.settings.len(), MAX_SETTINGS, "settings", sink);
    for (i, x) in r.settings.iter().enumerate() {
        text(
            &x.title,
            &format!("settings[{i}].title"),
            MAX_TITLE_CHARS,
            sink,
        );
        if let Some(d) = &x.description {
            text(
                d,
                &format!("settings[{i}].description"),
                MAX_DESCRIPTION_CHARS,
                sink,
            );
        }
        if let Some(opts) = &x.options {
            unique(
                opts.iter().map(String::as_str),
                |j| format!("settings[{i}].options[{j}]"),
                sink,
            );
        }
    }
    if let Some(t) = &r.themes {
        count(t.files.len(), MAX_CONTRIBUTIONS, "themes.files", sink);
    }
    count(r.icon_packs.len(), MAX_CONTRIBUTIONS, "icon_packs", sink);
    for (i, p) in r.icon_packs.iter().enumerate() {
        length(
            &p.name,
            MAX_TITLE_CHARS,
            &format!("icon_packs[{i}].name"),
            sink,
        );
    }
}

fn count(n: usize, max: usize, key: &str, sink: &mut Sink<'_>) {
    if n > max {
        sink.push(key, ErrorKind::TooMany, format!("at most {max} entries"));
    }
}

fn length(s: &str, max: usize, key: &str, sink: &mut Sink<'_>) {
    if s.chars().count() > max {
        sink.push(key, ErrorKind::TooLong, format!("at most {max} characters"));
    }
}

/// A literal within `max` characters, or a well-formed Fluent key.
fn text(t: &LocalisedText, key: &str, max: usize, sink: &mut Sink<'_>) {
    match t {
        LocalisedText::Literal(s) => length(s, max, key, sink),
        LocalisedText::Key(k) if !is_fluent_key(k) => sink.push(
            key,
            ErrorKind::InvalidFluentKey,
            "a Fluent message id: a letter, then letters, digits, - and _",
        ),
        LocalisedText::Key(_) => {}
    }
}

/// A Fluent message identifier, `[a-zA-Z][a-zA-Z0-9_-]*`.
fn is_fluent_key(k: &str) -> bool {
    let mut b = k.bytes();
    k.len() <= MAX_FLUENT_KEY_CHARS
        && b.next().is_some_and(|c| c.is_ascii_alphabetic())
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// One keymap keystroke (`cmd-shift-r`): distinct modifiers, at least one
/// besides shift, then a lowercase key name or one punctuation character.
fn is_keystroke(k: &str) -> bool {
    const MODIFIERS: &[&str] = &["cmd", "ctrl", "alt", "shift", "fn", "secondary"];
    let parts: Vec<&str> = k.split('-').collect();
    let Some((key, mods)) = parts.split_last() else {
        return false;
    };
    let key_ok = (key.len() == 1
        && key
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b.is_ascii_uppercase()))
        || (!key.is_empty()
            && key.len() <= 16
            && key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    let mut seen = HashSet::new();
    key_ok
        && mods
            .iter()
            .all(|m| MODIFIERS.contains(m) && seen.insert(*m))
        && mods.iter().any(|m| *m != "shift")
}

/// A Parla icon name or `plugin`: 1 to 40 of `[a-z0-9_-]`.
fn is_icon_name(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// `https://<host name>[/path]`, no credentials, port or whitespace.
fn is_https_url(u: &str) -> bool {
    let Some(rest) = u.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    u.len() <= 512 && is_host_name(host) && !u.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub(super) fn is_plugin_id(id: &str) -> bool {
    (3..=40).contains(&id.len())
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_namespace(ns: &str) -> bool {
    (1..=40).contains(&ns.len())
        && ns
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Contribution ids, tool names: 1 to 40 of `[a-z0-9_-]`.
fn is_contribution_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && !id.starts_with('-')
        && !id.ends_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn semver(v: &str, key: &str, sink: &mut Sink<'_>) -> Version {
    Version::parse(v).unwrap_or_else(|e| {
        sink.push(
            key,
            ErrorKind::InvalidVersion,
            format!("not a semver version: {e}"),
        );
        Version::new(0, 0, 0)
    })
}

fn api_version(api: &str, sink: &mut Sink<'_>) -> (u16, u16) {
    let parsed = api.split_once('.').and_then(|(a, b)| {
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        if digits(a) && digits(b) {
            Some((a.parse().ok()?, b.parse().ok()?))
        } else {
            None
        }
    });
    match parsed {
        None => {
            sink.push("api", ErrorKind::InvalidApi, "api is \"major.minor\"");
            parla_plugin_api::API_VERSION
        }
        Some(v) => {
            if !parla_plugin_api::supports(v) {
                let (ma, mi) = parla_plugin_api::API_VERSION;
                sink.push(
                    "api",
                    ErrorKind::UnsupportedApi,
                    format!(
                        "plugin API {}.{} is not supported by this host ({ma}.{mi})",
                        v.0, v.1
                    ),
                );
            }
            v
        }
    }
}

fn capabilities(c: RawCapabilities, sink: &mut Sink<'_>) -> CapabilitySet {
    let workspace_read = c
        .workspace_read
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let area = ReadArea::from_name(a);
            if area.is_none() {
                sink.push(
                    format!("capabilities.workspace_read[{i}]"),
                    ErrorKind::UnknownArea,
                    format!("`{a}` is not a read area"),
                );
            }
            area
        })
        .collect();
    let workspace_write = c
        .workspace_write
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let area = WriteArea::from_name(a);
            if area.is_none() {
                sink.push(
                    format!("capabilities.workspace_write[{i}]"),
                    ErrorKind::UnknownArea,
                    format!("`{a}` is not a write area"),
                );
            }
            area
        })
        .collect();
    let network = c
        .network
        .into_iter()
        .enumerate()
        .filter_map(|(i, h)| match network_host(&h) {
            Ok(()) => Some(NetworkHost(h)),
            Err((kind, detail)) => {
                sink.push(format!("capabilities.network[{i}]"), kind, detail);
                None
            }
        })
        .collect();
    let exec = c
        .exec
        .into_iter()
        .enumerate()
        .map(|(i, e)| exec_rule(i, e, sink))
        .collect();
    CapabilitySet {
        workspace_read,
        workspace_write,
        network,
        storage: c.storage,
        windows: c.windows,
        panel: c.panel,
        notifications: c.notifications,
        clipboard_write: c.clipboard_write,
        exec,
    }
}

/// Design §7.2: exact host names, https implied, no wildcard, no IP
/// literal; `localhost` only when listed by name.
pub(super) fn network_host(h: &str) -> Result<(), (ErrorKind, &'static str)> {
    if h.contains("://") {
        return Err((
            ErrorKind::NetworkScheme,
            "list the host name only; connections are always https",
        ));
    }
    if h.contains('*') {
        return Err((
            ErrorKind::NetworkWildcard,
            "wildcards are not allowed; list each host",
        ));
    }
    if h.starts_with('[')
        || h.parse::<Ipv4Addr>().is_ok()
        || h.parse::<Ipv6Addr>().is_ok()
        || h.rsplit('.').next().is_some_and(is_numeric_label)
    {
        return Err((
            ErrorKind::NetworkIpLiteral,
            "IP addresses are not allowed; use a host name",
        ));
    }
    if h == "localhost" || is_host_name(h) {
        Ok(())
    } else {
        Err((
            ErrorKind::NetworkInvalidHost,
            "not a host name (lowercase, no port, no path)",
        ))
    }
}

/// A label a resolver may read as a number (`1`, `0x7f`): a last label
/// like that makes the name an IP address in another spelling (`127.1`).
fn is_numeric_label(label: &str) -> bool {
    let hex = label
        .strip_prefix("0x")
        .or_else(|| label.strip_prefix("0X"))
        .is_some_and(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
    hex || (!label.is_empty() && label.bytes().all(|b| b.is_ascii_digit()))
}

/// A lowercase DNS name with at least two labels, whose last label is not
/// numeric and which is not under `localhost`.
fn is_host_name(h: &str) -> bool {
    h.len() <= 253
        && h.contains('.')
        && !h.ends_with(".localhost")
        && !h.rsplit('.').next().is_some_and(is_numeric_label)
        && h.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// Design §7.4: a bare program name and argv shapes; a flag only as a
/// literal.
fn exec_rule(i: usize, e: RawExec, sink: &mut Sink<'_>) -> ExecRule {
    let p = &e.program;
    let bare = !p.is_empty()
        && p != "."
        && p != ".."
        && !p.starts_with('-')
        && !p.contains(['/', '\\'])
        && !p.chars().any(char::is_whitespace);
    if !bare {
        sink.push(
            format!("capabilities.exec[{i}].program"),
            ErrorKind::ExecProgramPath,
            "program is a bare name found on PATH, never a path",
        );
    }
    if e.allow.is_empty() {
        sink.push(
            format!("capabilities.exec[{i}].allow"),
            ErrorKind::InvalidArgShape,
            "list at least one argument shape",
        );
    }
    let shell = shell_kind(&e.program);
    let allow = e
        .allow
        .into_iter()
        .enumerate()
        .map(|(j, shape)| {
            let bad = shape.is_empty()
                || shape.iter().any(|a| {
                    a.is_empty()
                        || (a.starts_with('-') && a.contains('*'))
                        || (a.contains('*') && a.chars().any(char::is_whitespace))
                });
            if bad {
                sink.push(
                    format!("capabilities.exec[{i}].allow[{j}]"),
                    ErrorKind::InvalidArgShape,
                    "a shape is a non-empty list of arguments; a flag must be literal",
                );
            }
            if shell.is_some_and(|k| shape.iter().any(|a| runs_command_string(k, a))) {
                sink.push(
                    format!("capabilities.exec[{i}].allow[{j}]"),
                    ErrorKind::ExecShell,
                    "a shape may not make a shell run a command string",
                );
            }
            ArgShape(
                shape
                    .into_iter()
                    .map(|a| {
                        if a.contains('*') {
                            ArgPattern::Glob(a)
                        } else {
                            ArgPattern::Literal(a)
                        }
                    })
                    .collect(),
            )
        })
        .collect();
    ExecRule {
        program: e.program,
        allow,
    }
}

#[derive(Clone, Copy)]
enum Shell {
    Posix,
    Cmd,
    Powershell,
}

/// Whether `program` is a command interpreter, by its name.
fn shell_kind(program: &str) -> Option<Shell> {
    let name = program.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    match name {
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "mksh" | "fish" | "csh" | "tcsh" | "ash"
        | "busybox" => Some(Shell::Posix),
        "cmd" => Some(Shell::Cmd),
        "powershell" | "pwsh" => Some(Shell::Powershell),
        _ => None,
    }
}

/// An argument that makes the shell run its next argument as a command
/// line (`-c`, `-lc`, `/c`, `-Command`), so a `*` after it would stand
/// for a whole command rather than a single argument.
fn runs_command_string(shell: Shell, arg: &str) -> bool {
    let a = arg.to_ascii_lowercase();
    match shell {
        Shell::Posix => a
            .strip_prefix('-')
            .is_some_and(|f| !f.starts_with('-') && f.contains('c')),
        Shell::Cmd => a == "/c" || a == "/k" || a == "/r",
        Shell::Powershell => a
            .strip_prefix('-')
            .or_else(|| a.strip_prefix('/'))
            .is_some_and(|f| {
                f == "ec"
                    || (!f.is_empty()
                        && ("command".starts_with(f) || "encodedcommand".starts_with(f)))
            }),
    }
}

fn check_contributions(c: &Contributions, sink: &mut Sink<'_>) {
    let ids = |kind: &str, field: &str, list: Vec<&str>, sink: &mut Sink<'_>| {
        for (i, id) in list.iter().enumerate() {
            if !is_contribution_id(id) {
                sink.push(
                    format!("{kind}[{i}].{field}"),
                    ErrorKind::InvalidContributionId,
                    "1 to 40 characters of a-z, 0-9, _ and -",
                );
            }
        }
        unique(list.into_iter(), |i| format!("{kind}[{i}].{field}"), sink);
    };
    ids(
        "commands",
        "id",
        c.commands.iter().map(|x| x.id.as_str()).collect(),
        sink,
    );
    ids(
        "message_actions",
        "id",
        c.message_actions.iter().map(|x| x.id.as_str()).collect(),
        sink,
    );
    ids(
        "composer_transforms",
        "id",
        c.composer_transforms
            .iter()
            .map(|x| x.id.as_str())
            .collect(),
        sink,
    );
    ids(
        "renderers",
        "id",
        c.renderers.iter().map(|x| x.id.as_str()).collect(),
        sink,
    );
    ids(
        "media_providers",
        "id",
        c.media_providers.iter().map(|x| x.id.as_str()).collect(),
        sink,
    );
    ids(
        "panels",
        "id",
        c.panels.iter().map(|x| x.id.as_str()).collect(),
        sink,
    );
    ids(
        "mcp_tools",
        "name",
        c.mcp_tools.iter().map(|x| x.name.as_str()).collect(),
        sink,
    );

    for (i, s) in c.slash.iter().enumerate() {
        let ok = s.name.strip_prefix('/').is_some_and(|n| {
            (1..=32).contains(&n.len())
                && n.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        });
        if !ok {
            sink.push(
                format!("slash[{i}].name"),
                ErrorKind::InvalidSlashName,
                "a slash command is / then 1 to 32 of a-z, 0-9, _ and -",
            );
        }
    }
    unique(
        c.slash.iter().map(|s| s.name.as_str()),
        |i| format!("slash[{i}].name"),
        sink,
    );

    for (i, r) in c.renderers.iter().enumerate() {
        if r.patterns.len() > MAX_RENDERER_PATTERNS {
            sink.push(
                format!("renderers[{i}].patterns"),
                ErrorKind::TooManyPatterns,
                format!("at most {MAX_RENDERER_PATTERNS} patterns"),
            );
        }
        if r.patterns.is_empty() {
            sink.push(
                format!("renderers[{i}].patterns"),
                ErrorKind::InvalidPattern,
                "list at least one pattern",
            );
        }
        for (j, p) in r.patterns.iter().enumerate() {
            if !is_link_pattern(p) {
                sink.push(
                    format!("renderers[{i}].patterns[{j}]"),
                    ErrorKind::InvalidPattern,
                    "a pattern is host/path with a literal host and no scheme",
                );
            }
        }
    }

    for (i, t) in c.mcp_tools.iter().enumerate() {
        path(
            &t.input_schema,
            &format!("mcp_tools[{i}].input_schema"),
            sink,
        );
    }
}

/// `host/path`: the host is a literal host name, `*` only in the path.
fn is_link_pattern(p: &str) -> bool {
    if p.contains("://") {
        return false;
    }
    match p.split_once('/') {
        Some((host, _)) => is_host_name(host) || host == "localhost",
        None => false,
    }
}

/// Reports every repeat of an earlier value.
fn unique<'a>(
    values: impl Iterator<Item = &'a str>,
    key: impl Fn(usize) -> String,
    sink: &mut Sink<'_>,
) {
    let mut seen = HashSet::new();
    for (i, v) in values.enumerate() {
        if !seen.insert(v) {
            sink.push(
                key(i),
                ErrorKind::DuplicateContribution,
                format!("`{v}` is declared twice"),
            );
        }
    }
}

/// A package-relative path: not absolute, no `..`, no drive.
fn path(p: &str, key: &str, sink: &mut Sink<'_>) {
    let ok = !p.is_empty()
        && !p.starts_with(['/', '\\'])
        && !p.contains(':')
        && p.split(['/', '\\']).all(|c| c != "..");
    if !ok {
        sink.push(
            key,
            ErrorKind::InvalidPath,
            "a path inside the package, not absolute and without ..",
        );
    }
}

/// Design §5 layout: `<folder>/<name>`, one level deep, ending in
/// `suffix` when given (theme files are `themes/*.json`, icon packs are
/// `icons/<pack>`).
fn in_folder(p: &str, folder: &str, suffix: Option<&str>, key: &str, sink: &mut Sink<'_>) {
    let ok = p
        .strip_prefix(folder)
        .and_then(|r| r.strip_prefix('/'))
        .is_some_and(|name| {
            !name.is_empty()
                && !name.contains(['/', '\\'])
                && suffix.is_none_or(|s| name.len() > s.len() && name.ends_with(s))
        });
    if !ok {
        let detail = match suffix {
            Some(s) => format!("a file `{folder}/<name>{s}`"),
            None => format!("a folder `{folder}/<name>`"),
        };
        sink.push(key, ErrorKind::InvalidPath, &detail);
    }
}

/// Design §6: absent means filled from the contributions (what the packer
/// writes, so a dev folder behaves like a package); present must name an
/// event for every contribution and only events with one.
fn activation(
    declared: Option<Vec<String>>,
    namespace: &str,
    c: &Contributions,
    sink: &mut Sink<'_>,
) -> Vec<Activation> {
    let implied = implied(namespace, c);
    let Some(declared) = declared else {
        return implied.into_iter().map(|(a, _)| a).collect();
    };
    let mut events = Vec::with_capacity(declared.len());
    for (i, text) in declared.into_iter().enumerate() {
        let key = format!("activation[{i}]");
        if text == "startup" {
            sink.push(
                key,
                ErrorKind::StartupActivation,
                "there is no startup activation; plugins load on a declared event",
            );
            continue;
        }
        let event = if text == "filter" {
            Some(Activation::Filter)
        } else {
            text.split_once(':')
                .filter(|(p, a)| Activation::PREFIXES.contains(p) && !a.is_empty())
                .and_then(|(p, a)| Activation::from_parts(p, a.to_owned()))
        };
        let Some(event) = event else {
            sink.push(
                key,
                ErrorKind::InvalidActivation,
                format!("`{text}` is not an activation event"),
            );
            continue;
        };
        if !implied.iter().any(|(a, _)| *a == event) {
            sink.push(
                key,
                ErrorKind::EventWithoutContribution,
                format!("no contribution declares `{text}`"),
            );
        }
        events.push(event);
    }
    for (event, key) in &implied {
        if !events.contains(event) {
            sink.push(
                key.clone(),
                ErrorKind::ContributionWithoutEvent,
                format!("add `{event}` to activation"),
            );
        }
    }
    events
}

fn setting(i: usize, s: RawSetting, sink: &mut Sink<'_>) -> Option<SettingSpec> {
    let at = |field: &str| format!("settings[{i}].{field}");
    let key_ok = (1..=40).contains(&s.key.len())
        && s.key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !key_ok {
        sink.push(
            at("key"),
            ErrorKind::InvalidSettingKey,
            "a setting key is 1 to 40 of a-z, 0-9 and _",
        );
    }
    let kind = match s.ty.as_str() {
        "boolean" => SettingKind::Boolean,
        "integer" => SettingKind::Integer {
            min: s.min,
            max: s.max,
        },
        "string" => SettingKind::String,
        "enum" => SettingKind::Enum {
            options: s.options.clone().unwrap_or_default(),
        },
        "secret" => SettingKind::Secret,
        other => {
            sink.push(
                at("type"),
                ErrorKind::SettingType,
                format!("`{other}` is not boolean, integer, string, enum or secret"),
            );
            return None;
        }
    };
    let integer = matches!(kind, SettingKind::Integer { .. });
    for (field, set) in [("min", s.min.is_some()), ("max", s.max.is_some())] {
        if set && !integer {
            sink.push(
                at(field),
                ErrorKind::SettingType,
                format!("{field} is for integer settings"),
            );
        }
    }
    if let (Some(lo), Some(hi)) = (s.min, s.max)
        && lo > hi
    {
        sink.push(at("max"), ErrorKind::SettingType, "max is below min");
    }
    match (&kind, &s.options) {
        (SettingKind::Enum { options }, _) if options.is_empty() => {
            sink.push(
                at("options"),
                ErrorKind::SettingType,
                "an enum lists its options",
            );
        }
        (SettingKind::Enum { .. }, _) | (_, None) => {}
        (_, Some(_)) => {
            sink.push(
                at("options"),
                ErrorKind::SettingType,
                "options are for enum settings",
            );
        }
    }

    let default = s.default.and_then(|v| {
        let value = match (&kind, v) {
            (SettingKind::Boolean, toml::Value::Boolean(b)) => Some(SettingValue::Boolean(b)),
            (SettingKind::Integer { min, max }, toml::Value::Integer(n))
                if min.is_none_or(|m| n >= m) && max.is_none_or(|m| n <= m) =>
            {
                Some(SettingValue::Integer(n))
            }
            (SettingKind::String, toml::Value::String(t)) => Some(SettingValue::String(t)),
            (SettingKind::Enum { options }, toml::Value::String(t)) if options.contains(&t) => {
                Some(SettingValue::String(t))
            }
            _ => None,
        };
        if value.is_none() {
            let why = if kind == SettingKind::Secret {
                "a secret has no default"
            } else {
                "the default does not fit the setting's type"
            };
            sink.push(at("default"), ErrorKind::SettingDefault, why);
        }
        value
    });

    Some(SettingSpec {
        key: s.key,
        kind,
        default,
        title: s.title,
        description: s.description,
    })
}
