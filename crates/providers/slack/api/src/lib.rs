//! Typed client for the Slack web-client API. Provider crate of the
//! Slack provider.

mod bookmarks;
mod boot;
mod channel_admin;
mod client;
pub mod dnd;
mod emoji;
mod error;
mod files;
mod huddles;
mod inbox;
mod limit;
pub mod method;
mod page;
mod people;
mod pins;
mod prefs;
mod reactions;
mod reads;
mod reminders;
mod retry;
mod saved;
mod scheduled;
mod search;
mod secret;
mod signin;
mod team;
mod threads;
pub mod types;
mod usergroups;
pub mod validate;
mod writes;

pub use bookmarks::BookmarkRaw;
pub use client::{ClientConfig, DEFAULT_USER_AGENT, WebClient};
pub use dnd::DndRaw;
pub use error::{AUTH_ERRORS, ApiError};
pub use files::{FileHosts, is_file_host};
pub use huddles::{HISTORY_LIMIT, Huddles, Room, RoomsJoin};
pub use inbox::{InboxPage, InboxRaw};
pub use limit::RateLimits;
pub use method::{Method, Priority, Tier};
pub use page::{Paged, ResponseMetadata};
pub use people::{PresenceMode, ProfileRaw};
pub use pins::PinRaw;
pub use prefs::{ChannelPrefRaw, NotifyPrefsRaw};
pub use reminders::ReminderRaw;
pub use saved::{SavedPage, SavedRaw};
pub use scheduled::ScheduledRaw;
pub use search::{SearchFile, SearchMessage, SearchResults};
pub use secret::{CredentialError, Secret, SessionCookie, XoxcToken};
pub use signin::{
    EmailConfirmation, EmailWorkspace, SendCodeOutcome, SigninWorkspace, confirm_code,
    find_workspaces, find_workspaces_for_email, send_code,
};
pub use team::TeamInfo;
pub use types::{
    AuthTest, Boot, BootSelf, BootTeam, Bot, BotProfile, ChannelCount, Conversation, Counts,
    Edited, File, HistoryPage, Message, Profile, ReactionRaw, Thumb, User,
};
pub use usergroups::UserGroupRaw;
pub use writes::PostedMessage;

/// Recorder for `PARLA_RECORD=<dir>` with the Slack rules; `None` unless set (debug builds only).
pub fn recorder_from_env() -> Option<std::sync::Arc<redact::record::Recorder>> {
    redact::record::Recorder::from_env(std::sync::Arc::new(slack_redact::Rules::default()))
        .map(std::sync::Arc::new)
}
