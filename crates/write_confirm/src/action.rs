//! What a requester wants written, with the names the user needs to judge it.

use std::sync::Arc;

use domain::{EmojiRef, MessageTs, Status};

/// What a reminder write does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderKind {
    Add,
    Complete,
    Delete,
}

/// The write, with names in place of ids. Message text is carried exactly as
/// the requester sent it.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteAction {
    Send {
        conversation_name: Arc<str>,
        thread: Option<MessageTs>,
        broadcast: bool,
        text: Arc<str>,
    },
    Schedule {
        conversation_name: Arc<str>,
        thread: Option<MessageTs>,
        text: Arc<str>,
        at: jiff::Timestamp,
    },
    React {
        conversation_name: Arc<str>,
        /// The first 200 characters of the target message.
        message_excerpt: Arc<str>,
        emoji: EmojiRef,
    },
    SetStatus {
        /// `None` clears the status.
        status: Option<Status>,
    },
    /// Replaces the text of one of the user's messages.
    Edit {
        conversation_name: Arc<str>,
        message_excerpt: Arc<str>,
        /// The new text, exactly.
        text: Arc<str>,
    },
    Delete {
        conversation_name: Arc<str>,
        message_excerpt: Arc<str>,
    },
    Upload {
        conversation_name: Arc<str>,
        name: Arc<str>,
        bytes: u64,
    },
    Reminder {
        kind: ReminderKind,
        text: Arc<str>,
        /// When an added reminder fires.
        at: Option<jiff::Timestamp>,
    },
    Pin {
        conversation_name: Arc<str>,
        message_excerpt: Arc<str>,
        on: bool,
    },
    Save {
        conversation_name: Arc<str>,
        message_excerpt: Arc<str>,
        on: bool,
    },
    MarkRead {
        conversation_name: Arc<str>,
    },
    Presence {
        away: bool,
    },
    /// Pauses notifications; `None` ends the pause.
    Snooze {
        minutes: Option<u32>,
    },
}

impl WriteAction {
    /// The stable name of the write: the MCP tool for the four MCP writes,
    /// the host function for the others.
    pub fn tool(&self) -> &'static str {
        match self {
            WriteAction::Send { .. } => "send_message",
            WriteAction::Schedule { .. } => "schedule_message",
            WriteAction::React { .. } => "add_reaction",
            WriteAction::SetStatus { .. } => "set_status",
            WriteAction::Edit { .. } => "edit_message",
            WriteAction::Delete { .. } => "delete_message",
            WriteAction::Upload { .. } => "upload_file",
            WriteAction::Reminder { kind, .. } => match kind {
                ReminderKind::Add => "add_reminder",
                ReminderKind::Complete => "complete_reminder",
                ReminderKind::Delete => "delete_reminder",
            },
            WriteAction::Pin { .. } => "set_pinned",
            WriteAction::Save { .. } => "set_saved",
            WriteAction::MarkRead { .. } => "mark_read",
            WriteAction::Presence { .. } => "set_presence",
            WriteAction::Snooze { minutes: Some(_) } => "snooze",
            WriteAction::Snooze { minutes: None } => "end_snooze",
        }
    }

    /// The conversation the write goes to, if it goes to one.
    pub fn conversation(&self) -> Option<&Arc<str>> {
        match self {
            WriteAction::Send {
                conversation_name, ..
            }
            | WriteAction::Schedule {
                conversation_name, ..
            }
            | WriteAction::React {
                conversation_name, ..
            }
            | WriteAction::Edit {
                conversation_name, ..
            }
            | WriteAction::Delete {
                conversation_name, ..
            }
            | WriteAction::Upload {
                conversation_name, ..
            }
            | WriteAction::Pin {
                conversation_name, ..
            }
            | WriteAction::Save {
                conversation_name, ..
            }
            | WriteAction::MarkRead { conversation_name } => Some(conversation_name),
            WriteAction::SetStatus { .. }
            | WriteAction::Reminder { .. }
            | WriteAction::Presence { .. }
            | WriteAction::Snooze { .. } => None,
        }
    }

    /// The write's arguments for the Details disclosure (names, not ids).
    pub(crate) fn args(&self) -> serde_json::Value {
        use serde_json::{Value, json};
        let ts = |t: &Option<MessageTs>| t.as_ref().map(|t| Value::from(&*t.0));
        match self {
            WriteAction::Send {
                conversation_name,
                thread,
                broadcast,
                text,
            } => json!({
                "conversation": &**conversation_name,
                "thread_ts": ts(thread),
                "also_send_to_conversation": broadcast,
                "text": &**text,
            }),
            WriteAction::Schedule {
                conversation_name,
                thread,
                text,
                at,
            } => json!({
                "conversation": &**conversation_name,
                "thread_ts": ts(thread),
                "at": at.to_string(),
                "text": &**text,
            }),
            WriteAction::React {
                conversation_name,
                emoji,
                ..
            } => json!({
                "conversation": &**conversation_name,
                "emoji": &*emoji.shortcode,
            }),
            WriteAction::SetStatus { status } => match status {
                None => json!({ "clear": true }),
                Some(s) => json!({
                    "text": &*s.text,
                    "emoji": s.emoji.as_ref().map(|e| e.shortcode.to_string()),
                    "expires": s.expires,
                }),
            },
            WriteAction::Edit {
                conversation_name,
                text,
                ..
            } => json!({
                "conversation": &**conversation_name,
                "text": &**text,
            }),
            WriteAction::Delete {
                conversation_name, ..
            }
            | WriteAction::MarkRead { conversation_name } => json!({
                "conversation": &**conversation_name,
            }),
            WriteAction::Upload {
                conversation_name,
                name,
                bytes,
            } => json!({
                "conversation": &**conversation_name,
                "name": &**name,
                "bytes": bytes,
            }),
            WriteAction::Reminder { text, at, .. } => json!({
                "text": &**text,
                "at": at.map(|t| t.to_string()),
            }),
            WriteAction::Pin {
                conversation_name,
                on,
                ..
            } => json!({
                "conversation": &**conversation_name,
                "pinned": on,
            }),
            WriteAction::Save {
                conversation_name,
                on,
                ..
            } => json!({
                "conversation": &**conversation_name,
                "saved": on,
            }),
            WriteAction::Presence { away } => json!({ "away": away }),
            WriteAction::Snooze { minutes } => json!({ "minutes": minutes }),
        }
    }
}
