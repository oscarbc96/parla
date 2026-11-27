//! Built-in slash commands: parse them and map them to reducer inputs.
//!
//! Only the neutral commands live here; a provider plugin may add its own.
//! Time phrases are passed to the provider verbatim, so Parla never has to
//! agree with it about "tomorrow" or a time zone.

use std::sync::Arc;

use crate::ids::UserId;
use crate::people::{PresenceSetting, Status};
use crate::reduce::Input;
use crate::remind_when::is_when;
use crate::rich::EmojiRef;
use crate::state::WorkspaceState;

pub const REMIND_USAGE: &str = "usage: /remind <me|@person|#conversation> <what> <when>, \
for example /remind me stretch in 20 minutes";
const DND_USAGE: &str = "usage: /dnd [<minutes>m|<hours>h|off]";
const AWAY_USAGE: &str = "usage: /away";
const ACTIVE_USAGE: &str = "usage: /active";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemindTarget {
    Me,
    /// Handle without the leading `@`.
    Person(String),
    /// Channel name without the leading `#`.
    Conversation(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashCommand {
    Remind {
        target: RemindTarget,
        what: String,
        when: String,
    },
    /// Minutes; `None` means the default.
    Dnd(Option<u32>),
    DndOff,
    Status {
        emoji: Option<String>,
        text: String,
    },
    ClearStatus,
    Away,
    Active,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashError {
    Unknown(String),
    Usage(&'static str),
    /// A `@name` or `#name` the workspace does not know.
    UnknownName(String),
}

/// `None` when the text is not a command; `Some(Err(..))` when it is one
/// Parla does not understand or that is malformed.
pub fn parse_slash(input: &str) -> Option<Result<SlashCommand, SlashError>> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let (name, args) = match rest.find(char::is_whitespace) {
        Some(i) => (&rest[..i], rest[i..].trim()),
        None => (rest, ""),
    };
    if name.is_empty() {
        return Some(Err(SlashError::Unknown(String::new())));
    }
    let command = name.to_ascii_lowercase();
    Some(match command.as_str() {
        "remind" => parse_remind(args),
        "dnd" => parse_dnd(args),
        "status" => Ok(parse_status(args)),
        "away" if args.is_empty() => Ok(SlashCommand::Away),
        "active" if args.is_empty() => Ok(SlashCommand::Active),
        "away" => Err(SlashError::Usage(AWAY_USAGE)),
        "active" => Err(SlashError::Usage(ACTIVE_USAGE)),
        _ => Err(SlashError::Unknown(command)),
    })
}

/// Resolve the names in `cmd` against the workspace. `now_unix` stamps
/// relative times (`/dnd 2h`); the reducer stays pure.
pub fn slash_to_input(
    cmd: SlashCommand,
    state: &WorkspaceState,
    now_unix: i64,
) -> Result<Input, SlashError> {
    Ok(match cmd {
        SlashCommand::Remind { target, what, when } => {
            let user = match target {
                RemindTarget::Me => None,
                RemindTarget::Person(name) => Some(resolve_member(&name, state)?),
                RemindTarget::Conversation(name) => {
                    resolve_conversation(&name, state)?;
                    None
                }
            };
            Input::AddReminder {
                text: Arc::from(what),
                when: Arc::from(when),
                user,
            }
        }
        SlashCommand::Dnd(minutes) => Input::Snooze {
            minutes: minutes.unwrap_or(60),
            now_unix,
        },
        SlashCommand::DndOff => Input::EndSnooze,
        SlashCommand::Status { emoji, text } => Input::SetStatus(Some(Status {
            emoji: emoji.map(|shortcode| EmojiRef {
                shortcode: Arc::from(shortcode),
                unicode: None,
                skin_tone: None,
            }),
            text: Arc::from(text),
            expires: None,
        })),
        SlashCommand::ClearStatus => Input::SetStatus(None),
        SlashCommand::Away => Input::SetPresence(PresenceSetting::Away),
        SlashCommand::Active => Input::SetPresence(PresenceSetting::Auto),
    })
}

fn resolve_member(name: &str, state: &WorkspaceState) -> Result<UserId, SlashError> {
    // `@name` is the handle first; a display name only when no handle
    // matches. Smallest id wins a tie so the answer never depends on
    // hash order.
    let live = || state.members.values().filter(|m| !m.deleted);
    live()
        .filter(|m| !m.handle.is_empty() && m.handle.eq_ignore_ascii_case(name))
        .min_by(|a, b| a.id.cmp(&b.id))
        .or_else(|| {
            live()
                .filter(|m| m.display_name.eq_ignore_ascii_case(name))
                .min_by(|a, b| a.id.cmp(&b.id))
        })
        .map(|m| m.id.clone())
        .ok_or_else(|| SlashError::UnknownName(name.to_string()))
}

fn resolve_conversation(
    name: &str,
    state: &WorkspaceState,
) -> Result<crate::ids::ConversationId, SlashError> {
    state
        .conversations
        .values()
        .find(|c| c.name.eq_ignore_ascii_case(name))
        .map(|c| c.id.clone())
        .ok_or_else(|| SlashError::UnknownName(name.to_string()))
}

fn parse_remind(args: &str) -> Result<SlashCommand, SlashError> {
    let mut it = args.splitn(2, char::is_whitespace);
    let target = it.next().unwrap_or("").trim();
    let target = parse_target(target)?;
    let rest = it.next().unwrap_or("").trim();
    if rest.is_empty() {
        return Err(SlashError::Usage(REMIND_USAGE));
    }
    let (what, when) = if let Some(quoted) = rest.strip_prefix('"') {
        let Some(end) = quoted.find('"') else {
            return Err(SlashError::Usage(REMIND_USAGE));
        };
        let what = quoted[..end].trim();
        let when = quoted[end + 1..].trim();
        if what.is_empty() || when.is_empty() || !is_when(when) {
            return Err(SlashError::Usage(REMIND_USAGE));
        }
        (what.to_string(), when.to_string())
    } else {
        let Some((start, when)) = trailing_when(rest) else {
            return Err(SlashError::Usage(REMIND_USAGE));
        };
        let what = rest[..start].trim();
        if what.is_empty() {
            return Err(SlashError::Usage(REMIND_USAGE));
        }
        (what.to_string(), when.to_string())
    };
    Ok(SlashCommand::Remind { target, what, when })
}

fn parse_target(token: &str) -> Result<RemindTarget, SlashError> {
    if token.eq_ignore_ascii_case("me") {
        Ok(RemindTarget::Me)
    } else if let Some(name) = token.strip_prefix('@').filter(|n| !n.is_empty()) {
        Ok(RemindTarget::Person(name.to_string()))
    } else if let Some(name) = token.strip_prefix('#').filter(|n| !n.is_empty()) {
        Ok(RemindTarget::Conversation(name.to_string()))
    } else {
        Err(SlashError::Usage(REMIND_USAGE))
    }
}

/// The longest `when` at the end of `text`: the earliest word whose suffix is
/// a valid phrase. Returns its byte offset and the phrase.
fn trailing_when(text: &str) -> Option<(usize, &str)> {
    let mut starts = Vec::new();
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            in_word = false;
        } else if !in_word {
            starts.push(i);
            in_word = true;
        }
    }
    starts
        .into_iter()
        .find(|&start| is_when(&text[start..]))
        .map(|start| (start, &text[start..]))
}

fn parse_dnd(args: &str) -> Result<SlashCommand, SlashError> {
    let mut it = args.split_whitespace();
    let Some(arg) = it.next() else {
        return Ok(SlashCommand::Dnd(Some(60)));
    };
    if it.next().is_some() {
        return Err(SlashError::Usage(DND_USAGE));
    }
    if arg.eq_ignore_ascii_case("off") {
        return Ok(SlashCommand::DndOff);
    }
    let lower = arg.to_ascii_lowercase();
    let minutes = if let Some(hours) = lower.strip_suffix('h') {
        hours.parse::<u32>().ok().and_then(|n| n.checked_mul(60))
    } else if let Some(minutes) = lower.strip_suffix('m') {
        minutes.parse::<u32>().ok()
    } else {
        lower.parse::<u32>().ok()
    };
    match minutes {
        Some(n) if n > 0 => Ok(SlashCommand::Dnd(Some(n))),
        _ => Err(SlashError::Usage(DND_USAGE)),
    }
}

fn parse_status(args: &str) -> SlashCommand {
    if args.is_empty() || args.eq_ignore_ascii_case("clear") {
        return SlashCommand::ClearStatus;
    }
    if let Some(rest) = args.strip_prefix(':')
        && let Some((shortcode, tail)) = rest.split_once(':')
        && !shortcode.is_empty()
        && !shortcode.contains(char::is_whitespace)
    {
        return SlashCommand::Status {
            emoji: Some(shortcode.to_string()),
            text: tail.trim().to_string(),
        };
    }
    SlashCommand::Status {
        emoji: None,
        text: args.trim().to_string(),
    }
}
