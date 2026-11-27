//! Reminders.

use std::sync::Arc;

use crate::ids::{MessageId, UserId};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReminderId(pub Arc<str>);

#[derive(Clone, Debug, PartialEq)]
pub struct Reminder {
    pub id: ReminderId,
    pub text: Arc<str>,
    /// Unix seconds; `None` when the provider gave no time.
    pub due: Option<i64>,
    pub recurring: bool,
    /// Seconds between occurrences of a recurring local reminder (`0` when not
    /// recurring, or a provider keeps the schedule).
    pub interval_secs: i64,
    pub completed: bool,
    /// Parla's own reminders: the desktop alert was already raised, so it is
    /// not raised again (kept across restarts). Always false for a provider's.
    pub fired: bool,
    pub for_user: UserId,
    /// The message this reminder is about (Parla's own reminders only).
    pub link: Option<ReminderLink>,
    /// One occurrence of a recurring local reminder moved by a snooze; the
    /// series keeps `due`. Always `None` for a provider's.
    pub snoozed_until: Option<i64>,
}

impl Reminder {
    /// A reminder about a message with no text of its own: views show a
    /// label in the current language instead.
    pub fn about_message(&self) -> bool {
        self.text.is_empty() && self.link.is_some()
    }

    /// When it alerts next: the snoozed occurrence or the due time,
    /// whichever comes first.
    pub fn next_at(&self) -> Option<i64> {
        match (self.snoozed_until, self.due) {
            (Some(s), Some(d)) => Some(s.min(d)),
            (s, d) => s.or(d),
        }
    }
}

/// The message a reminder is about, with what the list shows of it (kept with
/// the reminder, so the context shows when the message is not loaded).
#[derive(Clone, Debug, PartialEq)]
pub struct ReminderLink {
    pub message: MessageId,
    pub author: Option<UserId>,
    /// The start of the message text.
    pub excerpt: Arc<str>,
}

/// Longest excerpt a [`ReminderLink`] keeps, in characters.
pub const REMINDER_EXCERPT_CHARS: usize = 140;

impl ReminderLink {
    /// A link to `message` with its plain `text` cut to
    /// [`REMINDER_EXCERPT_CHARS`] on a word (or a char when one word is too
    /// long), an ellipsis marking the cut.
    pub fn new(message: MessageId, author: Option<UserId>, text: &str) -> Self {
        Self {
            message,
            author,
            excerpt: excerpt(text),
        }
    }
}

/// The start of `text` for a [`ReminderLink`]: one line, at most
/// [`REMINDER_EXCERPT_CHARS`] characters, cut on a word.
pub fn excerpt(text: &str) -> Arc<str> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= REMINDER_EXCERPT_CHARS {
        return Arc::from(flat.as_str());
    }
    let end = flat
        .char_indices()
        .nth(REMINDER_EXCERPT_CHARS)
        .map_or(flat.len(), |(i, _)| i);
    let head = &flat[..end];
    // Back to the last word break, unless the cut is inside the first word.
    let head = match head.rfind(' ') {
        Some(i) if i > 0 => &head[..i],
        _ => head,
    };
    Arc::from(format!("{head}\u{2026}").as_str())
}

/// How one of Parla's own reminders repeats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ReminderRepeat {
    #[default]
    Never,
    Daily,
    Weekly,
}

impl ReminderRepeat {
    pub const ALL: [ReminderRepeat; 3] = [Self::Never, Self::Daily, Self::Weekly];

    /// Seconds between occurrences; `0` for [`ReminderRepeat::Never`].
    pub fn interval_secs(self) -> i64 {
        match self {
            Self::Never => 0,
            Self::Daily => 86_400,
            Self::Weekly => 7 * 86_400,
        }
    }

    /// How `r` repeats now (a recurring one without a known daily step is weekly).
    pub fn of(r: &Reminder) -> Self {
        match (r.recurring, r.interval_secs) {
            (false, _) => Self::Never,
            (true, 86_400) => Self::Daily,
            (true, _) => Self::Weekly,
        }
    }
}
