//! Profiles, status, presence setting, do-not-disturb and notification levels.

use std::sync::Arc;

use crate::ids::UserId;
use crate::rich::{EmojiRef, ImageRef};

#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub emoji: Option<EmojiRef>,
    pub text: Arc<str>,
    /// Unix seconds.
    pub expires: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenceSetting {
    Auto,
    Away,
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub user: UserId,
    pub display_name: Arc<str>,
    pub real_name: Arc<str>,
    pub title: Option<Arc<str>>,
    pub pronouns: Option<Arc<str>>,
    /// IANA time zone name.
    pub tz: Option<Arc<str>>,
    pub status: Option<Status>,
    pub email: Option<Arc<str>>,
    pub phone: Option<Arc<str>>,
    pub fields: Vec<(Arc<str>, Arc<str>)>,
    pub image: Option<ImageRef>,
    pub is_bot: bool,
    pub deleted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DndStatus {
    pub enabled: bool,
    pub next_start: Option<i64>,
    pub next_end: Option<i64>,
    pub snooze_until: Option<i64>,
}

impl DndStatus {
    /// Snoozed, or enabled and inside `[next_start, next_end)` (unix seconds).
    pub fn active(&self, now: i64) -> bool {
        if self.snooze_until.is_some_and(|until| now < until) {
            return true;
        }
        self.enabled
            && matches!((self.next_start, self.next_end),
                (Some(s), Some(e)) if s <= now && now < e)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotifyLevel {
    #[default]
    Default,
    All,
    Mentions,
    Nothing,
}

/// What Parla keeps per conversation beyond the provider's level and mute
/// flag. Local only: no provider is asked. The default is "no entry".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotifyExtras {
    /// Replies in followed threads do not notify (a mention still does).
    pub threads_off: bool,
    /// Notifications of this conversation are silent.
    pub sound_off: bool,
    /// Muted until this unix time (a stored deadline, so it survives a restart).
    pub mute_until: Option<i64>,
}

impl NotifyExtras {
    /// The timed mute holds at `now` (unix seconds).
    pub fn muted_at(&self, now: i64) -> bool {
        self.mute_until.is_some_and(|until| now < until)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConversationPrefs {
    pub level: NotifyLevel,
    pub muted: bool,
}
