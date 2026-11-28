//! The built-in slash commands the composer menu lists.
//!
//! Only commands whose capability the provider has are offered; anything else
//! typed after a `/` is "not supported in Parla".

use domain::Capabilities;
use gpui::App;
use i18n::Localizer;

/// Where a slash command comes from, shown as the row's badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlashSource {
    /// Parla or the provider's own command.
    Builtin,
    /// A command the workspace defines.
    Workspace,
    /// A command a plugin adds.
    Plugin,
}

impl SlashSource {
    /// The badge text, in the active language.
    pub fn label(self, l: &Localizer) -> String {
        match self {
            Self::Builtin => t!(l, "slash-source-builtin"),
            Self::Workspace => t!(l, "slash-source-workspace"),
            Self::Plugin => t!(l, "slash-source-plugin"),
        }
        .into_owned()
    }
}

/// One row of the slash menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashEntry {
    /// Command name without the slash.
    pub name: &'static str,
    /// Where the command comes from.
    pub source: SlashSource,
}

/// Every built-in command with the capability that enables it.
type Enabled = fn(&Capabilities) -> bool;

const COMMANDS: [(&str, Enabled); 6] = [
    ("remind", |c| c.reminders),
    ("dnd", |c| c.dnd),
    ("status", |c| c.status),
    ("away", |c| c.presence),
    ("active", |c| c.presence),
    // Appends a shrug to the message; Parla does it, so no capability.
    (SHRUG, |_| true),
];

/// The command that appends [`SHRUG_TEXT`] to a message.
pub const SHRUG: &str = "shrug";

/// The command that opens the GIF picker.
pub const GIF: &str = "gif";

/// What `/shrug` appends.
pub const SHRUG_TEXT: &str = "\u{af}\\_(\u{30c4})_/\u{af}";

/// `text` of a `/shrug [text]` line with the shrug appended, or `None` when the
/// line is another command. The text after the name keeps its spans (`shift`
/// moves them back by the removed prefix).
pub fn expand_shrug(body: &domain::ComposedText) -> Option<domain::ComposedText> {
    let rest = body.text.strip_prefix('/')?;
    let name_len = rest.find(char::is_whitespace).unwrap_or(rest.len());
    if !rest[..name_len].eq_ignore_ascii_case(SHRUG) {
        return None;
    }
    let after = &rest[name_len..];
    let arg = after.trim_start();
    let removed = body.text.len() - arg.len();
    let arg = arg.trim_end();
    let mut text = arg.to_string();
    if !text.is_empty() {
        text.push(' ');
    }
    text.push_str(SHRUG_TEXT);
    let spans = body
        .spans
        .iter()
        .filter(|s| s.range.start >= removed && s.range.end <= removed + arg.len())
        .map(|s| domain::ComposedSpan {
            range: s.range.start - removed..s.range.end - removed,
            inline: s.inline.clone(),
        })
        .collect();
    Some(domain::ComposedText::new(&text, spans))
}

/// Whether the provider supports the built-in command `name` (any case).
pub fn available(caps: &Capabilities, name: &str) -> bool {
    COMMANDS
        .iter()
        .any(|(n, on)| n.eq_ignore_ascii_case(name) && on(caps))
}

/// The commands offered for the typed `prefix` (the name so far, without the slash).
pub fn entries(caps: &Capabilities, prefix: &str, cx: &App) -> Vec<SlashEntry> {
    let p = prefix.to_ascii_lowercase();
    let mut out: Vec<SlashEntry> = COMMANDS
        .iter()
        .filter(|(n, on)| n.starts_with(&p) && on(caps))
        .map(|(name, _)| SlashEntry {
            name,
            source: SlashSource::Builtin,
        })
        .collect();
    // `/gif` is Parla's own picker, offered only when a GIF source is installed.
    if GIF.starts_with(&p) && crate::gif_picker::enabled(cx) {
        out.push(SlashEntry {
            name: GIF,
            source: SlashSource::Builtin,
        });
    }
    out
}

/// The command name being typed when `text` is a `/` plus name characters only
/// (`Some("")` for a bare `/`); `None` once an argument starts or for ordinary text.
pub fn query(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('/')?;
    (!rest.contains(char::is_whitespace)).then_some(rest)
}

/// One line saying what a built-in command does, in the active language.
pub fn description(l: &Localizer, name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "remind" => t!(l, "slash-desc-remind"),
        "dnd" => t!(l, "slash-desc-dnd"),
        "status" => t!(l, "slash-desc-status"),
        "away" => t!(l, "slash-desc-away"),
        "active" => t!(l, "slash-desc-active"),
        "shrug" => t!(l, "slash-desc-shrug"),
        "gif" => t!(l, "slash-desc-gif"),
        _ => return String::new(),
    }
    .into_owned()
}

/// The usage line of a built-in command, in the active language.
pub fn usage(l: &Localizer, name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "remind" => t!(l, "slash-usage-remind"),
        "dnd" => t!(l, "slash-usage-dnd"),
        "status" => t!(l, "slash-usage-status"),
        "away" => t!(l, "slash-usage-away"),
        "active" => t!(l, "slash-usage-active"),
        "shrug" => t!(l, "slash-usage-shrug"),
        "gif" => t!(l, "slash-usage-gif"),
        other => return format!("/{other}"),
    }
    .into_owned()
}
