//! Provider-neutral chat model, capabilities and the `ChatProvider` trait.
//!
//! Nothing provider-specific may appear here (see the boundary tests).

mod accounts;
mod aggregate;
mod call_surface;
mod calls;
mod caps;
mod channel_admin;
pub mod clock;
mod collections;
mod compose;
mod content;
mod dnd_logic;
mod ext;
mod files;
mod flight;
mod huddle;
mod huddles_logic;
mod ids;
mod jump;
mod lean;
mod members_logic;
mod message_events;
mod model;
pub mod notify;
mod outbox;
mod outbox_logic;
mod people;
mod presence_logic;
mod profiles;
mod provider;
mod reactions;
mod reactions_logic;
mod reduce;
mod remind_when;
mod reminder;
mod reminders_logic;
mod rich;
mod saved;
mod schedule;
mod scheduling;
mod search;
mod search_query;
mod secret;
mod slash;
mod state;
pub mod sync;
mod sync_types;
#[cfg(feature = "test-support")]
pub mod test_support;
mod thread_follow;
mod threads;
mod typing;
pub mod unread;
mod user_groups;
mod when_parse;
mod window;

pub use accounts::{AccountInfo, OrgInfo};
pub use aggregate::{
    AggregateRow, AggregateScheduled, WorkspaceRef, all_mentions, all_scheduled, all_unreads,
    unread_previews,
};
pub use call_surface::{CallSurface, NativeCallJoin, SurfaceCookie};
pub use calls::{CallOutput, Load, Loaded, ProviderCall};
pub use caps::Capabilities;
pub use channel_admin::{AdminCall, ChannelName, InviteReport, NameProblem};
#[cfg(feature = "test-support")]
pub use clock::FixedNow;
pub use clock::{Now, SystemNow};
pub use compose::{
    ComposedSpan, ComposedText, ContentEncoder, PlainEncoder, spans_from_json, spans_to_json,
};
pub use content::{ContentParser, ExtCodec, ExtKind, NoExtCodec, PlainParser};
pub use dnd_logic::next_mute_deadline;
pub use dnd_logic::{DndDay, DndEnds, dnd_ends};
pub use ext::ProviderExt;
pub use files::{FileKind, FileRef, Rendition, Upload, VideoInfo, tombstone as tombstone_file};
pub use huddle::{Huddle, HuddleId};
pub use huddles_logic::{RING_TIMEOUT, Ring, RingEnd};
pub use ids::{ClientMsgId, ConversationId, FileId, MessageId, MessageTs, RemoteUrl, UserId};
#[cfg(feature = "test-support")]
pub use members_logic::MAX_BATCH as MEMBER_LOOKUP_BATCH;
pub use model::{
    AVATAR_LARGE_FROM_PX, Avatar, Bootstrap, Conversation, ConversationKind, ConversationMeta,
    DIRECT_MESSAGES_SHOWN, DeliveryState, HistoryPage, Member, Message, PersistOp, ProviderError,
    ProviderEvent,
};
pub use outbox::{DraftKey, OutboxItem, OutboxState};
pub use outbox_logic::{MAX_IN_FLIGHT, OutboxOp, RESTART_REASON};
pub use people::{
    ConversationPrefs, DndStatus, NotifyExtras, NotifyLevel, PresenceSetting, Profile, Status,
};
#[cfg(feature = "test-support")]
pub use profiles::PROFILE_CACHE;
pub use profiles::{local_time, status_visible};
pub use provider::{AROUND_MAX_PAGES, ChatProvider, ParsedPermalink, ProviderFactory};
pub use reactions::{CustomEmoji, Reaction};
pub use reduce::{Changed, Effect, Input, reduce};
pub use remind_when::{Due, RemindPreset, is_repeating_when, is_when, parse_when};
pub use reminder::{
    REMINDER_EXCERPT_CHARS, Reminder, ReminderId, ReminderLink, ReminderRepeat, excerpt,
};
pub use reminders_logic::next_reminder_due;
pub use rich::{
    Block, Broadcast, CallCard, CallKind, Card, ContextItem, Control, ControlKind, EmojiRef,
    ImageRef, Inline, ListStyle, Mention, RichText, Style,
};
pub use saved::{Bookmark, BookmarkId, InboxThread, Pin, SavedItem};
pub use schedule::{LocalSchedule, LocalScheduleState, ScheduleId, ScheduledMessage};
#[cfg(feature = "test-support")]
pub use scheduling::LATE_AFTER_SECS;
pub use scheduling::{
    MAX_AHEAD_DAYS, ScheduleError, parse_preset, schedule_zone, state_schedule_zone, validate,
};
pub use search::{AroundPage, FileHit, Has, HitSource, MessageHit, Page, SearchQuery};
pub use search_query::{ParsedQuery, local_supported, merge_hits, parse_query, resolve_query};
pub use secret::SecretText;
pub use slash::{
    REMIND_USAGE, RemindTarget, SlashCommand, SlashError, parse_slash, slash_to_input,
};
pub use state::{Connection, Session, WorkspaceState};
pub use sync_types::{ConversationCounts, Gap, SearchHit};
pub use thread_follow::MAX_THREAD_FOLLOW;
pub use threads::ThreadWindow;
pub use typing::{TYPERS_MAX, TYPING_PLACES_MAX, TYPING_SEND_EVERY, TYPING_TTL, Typer};
pub use unread::{UnreadTotals, mentions_self};
pub use user_groups::{MAX_USER_GROUPS, UserGroup, UserGroups, resolve_group_text};
pub use window::MessageWindow;
