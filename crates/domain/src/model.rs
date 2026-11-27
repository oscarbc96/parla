use std::sync::Arc;

use crate::ext::ProviderExt;
use crate::files::FileRef;
use crate::huddle::{Huddle, HuddleId};
use crate::ids::{ClientMsgId, ConversationId, FileId, MessageId, MessageTs, UserId};
use crate::people::{ConversationPrefs, DndStatus, Status};
use crate::reactions::Reaction;
use crate::rich::{EmojiRef, ImageRef, Mention};
use crate::sync_types::Gap;

/// Direct and group conversations the sidebar lists before "Show more"
/// (unread and the open one are always listed too).
pub const DIRECT_MESSAGES_SHOWN: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationKind {
    Channel,
    PrivateChannel,
    Direct,
    Group,
}

#[derive(Clone, Debug)]
pub struct Conversation {
    pub id: ConversationId,
    pub kind: ConversationKind,
    pub name: Arc<str>,
    pub peer: Option<UserId>,
    pub is_member: bool,
    pub is_archived: bool,
    /// Shown in the sidebar. A closed direct or group conversation is hidden
    /// until a new message arrives (it stays reachable by search and jump).
    /// Always true for channels; not persisted.
    pub is_open: bool,
    pub is_muted: bool,
    pub last_read: Option<MessageTs>,
    pub latest: Option<MessageTs>,
    pub unread: bool,
    pub mentions: u32,
    /// Handles of a group's participants (see [`Member::handle`]); empty for
    /// other kinds. The sidebar resolves them to display names.
    pub participants: Arc<[Arc<str>]>,
    /// Ids of a group's members, the signed-in user included, when the
    /// provider lists them; empty for other kinds and for rows read from the
    /// cache. Looked up on demand like authors; not persisted.
    pub member_ids: Arc<[UserId]>,
    /// The conversation topic (empty when none or unknown); persisted so the
    /// header paints from the cache.
    pub topic: Arc<str>,
    /// Member count when the provider reports it; persisted like the topic.
    pub member_count: Option<u32>,
    /// What the channel is for (its description); empty when none or not
    /// known yet. Not persisted: the provider refills it.
    pub purpose: Arc<str>,
    pub ext: ProviderExt,
}

/// The metadata of a conversation that can change under us; `None` leaves a
/// field as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationMeta {
    pub id: ConversationId,
    pub name: Option<Arc<str>>,
    pub topic: Option<Arc<str>>,
    pub is_archived: Option<bool>,
    pub is_member: Option<bool>,
    pub member_count: Option<u32>,
    /// The description (what the channel is for).
    pub purpose: Option<Arc<str>>,
}

impl ConversationMeta {
    /// A change of nothing yet, for `id`; set the fields that changed.
    pub fn new(id: ConversationId) -> Self {
        Self {
            id,
            name: None,
            topic: None,
            is_archived: None,
            is_member: None,
            member_count: None,
            purpose: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryState {
    Delivered,
    Pending,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: MessageId,
    pub author: Option<UserId>,
    pub text: Arc<str>,
    pub thread_root: Option<MessageTs>,
    pub reply_count: u32,
    pub edited: Option<MessageTs>,
    pub client_msg_id: Option<ClientMsgId>,
    pub state: DeliveryState,
    pub ext: ProviderExt,
    /// Provider-produced searchable text.
    pub plain: Arc<str>,
    pub mentions: Arc<[Mention]>,
    pub reactions: Arc<[Reaction]>,
    pub files: Arc<[FileRef]>,
    pub latest_reply: Option<MessageTs>,
    pub reply_users: Arc<[UserId]>,
    /// Thread reply also sent to the conversation.
    pub broadcast: bool,
    pub pinned: bool,
    pub saved: bool,
}

impl Message {
    /// Delivered plain message: `plain = text`, everything else empty.
    pub fn new(id: MessageId, author: Option<UserId>, text: Arc<str>) -> Message {
        Message {
            id,
            author,
            plain: text.clone(),
            text,
            thread_root: None,
            reply_count: 0,
            edited: None,
            client_msg_id: None,
            state: DeliveryState::Delivered,
            ext: ProviderExt::none(),
            mentions: Arc::from([]),
            reactions: Arc::from([]),
            files: Arc::from([]),
            latest_reply: None,
            reply_users: Arc::from([]),
            broadcast: false,
            pinned: false,
            saved: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Member {
    pub id: UserId,
    pub display_name: Arc<str>,
    pub real_name: Arc<str>,
    /// The provider's short user name (empty when unknown), used to match a
    /// group's participants.
    pub handle: Arc<str>,
    pub is_bot: bool,
    pub deleted: bool,
    pub status: Option<Status>,
    /// Job title from the directory (`None` when unknown or blank).
    pub title: Option<Arc<str>>,
    /// IANA time zone name from the directory (`None` when unknown).
    pub tz: Option<Arc<str>>,
    /// The profile picture, sized for list rows (`None` when the person has
    /// none): shown in place of the initials once loaded.
    pub avatar: Option<Arc<Avatar>>,
    pub ext: ProviderExt,
}

/// Logical side from which [`Member::avatar_for`] uses the large picture.
pub const AVATAR_LARGE_FROM_PX: f32 = 56.0;

/// A person's profile pictures: one sized for list rows, one for big avatars.
/// Both are always set (a provider with one size gives it twice), and a member
/// holds them behind one pointer so the many without a picture pay 8 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avatar {
    pub row: ImageRef,
    pub large: ImageRef,
}

impl Avatar {
    /// The pictures, or `None` when there is none; a missing size reuses the other.
    pub fn new(row: Option<ImageRef>, large: Option<ImageRef>) -> Option<Arc<Avatar>> {
        let (row, large) = match (row, large) {
            (Some(r), Some(l)) => (r, l),
            (Some(r), None) => (r.clone(), r),
            (None, Some(l)) => (l.clone(), l),
            (None, None) => return None,
        };
        Some(Arc::new(Avatar { row, large }))
    }
}

impl Member {
    /// The picture for an avatar `side` logical pixels wide: the large one from
    /// [`AVATAR_LARGE_FROM_PX`] up, the row-sized one below.
    pub fn avatar_for(&self, side: f32) -> Option<&ImageRef> {
        self.avatar.as_deref().map(|a| {
            if side >= AVATAR_LARGE_FROM_PX {
                &a.large
            } else {
                &a.row
            }
        })
    }
}

#[derive(Clone, Debug)]
pub struct Bootstrap {
    pub self_user: UserId,
    pub workspace_name: Arc<str>,
    pub conversations: Vec<Conversation>,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug)]
pub struct HistoryPage {
    pub messages: Vec<Message>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub enum ProviderEvent {
    Connected {
        reconnect: bool,
    },
    Disconnected,
    SessionExpired,
    MessagePosted(Message),
    MessageEdited {
        id: MessageId,
        text: Arc<str>,
        /// The searchable text of the edited message.
        plain: Arc<str>,
        edited: MessageTs,
    },
    MessageDeleted(MessageId),
    ReadMarker {
        conversation: ConversationId,
        ts: MessageTs,
    },
    /// Name, topic, archived, membership or member count changed. Merged into
    /// the held conversation; read state and mute are left alone. A change for
    /// a conversation not held yet asks the provider for it
    /// (`Load::Conversation`, `Capabilities::conversation_lookup`).
    ConversationMetaChanged(ConversationMeta),
    /// Someone other than the signed-in user joined (`joined`) or left a
    /// conversation: its member list, when held, and its member count follow.
    ConversationMemberChanged {
        conversation: ConversationId,
        user: UserId,
        joined: bool,
    },
    MemberUpdated(Member),
    Typing {
        conversation: ConversationId,
        user: UserId,
        thread: Option<MessageTs>,
    },
    Presence {
        user: UserId,
        active: bool,
    },
    /// Full replacement (unfurls, file changes).
    MessageUpdated(Message),
    ReactionChanged {
        id: MessageId,
        emoji: EmojiRef,
        user: UserId,
        added: bool,
    },
    ThreadUpdated {
        root: MessageId,
        reply_count: u32,
        latest_reply: Option<MessageTs>,
        reply_users: Arc<[UserId]>,
    },
    ThreadReadMarker {
        root: MessageId,
        ts: MessageTs,
    },
    FileDeleted(FileId),
    CustomEmojiChanged,
    DndChanged {
        user: UserId,
        status: DndStatus,
    },
    ProfileChanged(UserId),
    NotifyPrefsChanged(std::collections::HashMap<ConversationId, ConversationPrefs>),
    /// Reserved: no bundled provider emits it yet; a provider with
    /// reminders may, and tests drive it directly.
    RemindersChanged,
    /// Reserved: no bundled provider emits it yet; a provider with
    /// scheduled messages may, and tests drive it directly.
    ScheduledChanged,
    HuddleUpdated(Huddle),
    HuddleEnded {
        conversation: ConversationId,
        id: HuddleId,
        /// Unix seconds.
        ended: i64,
    },
    HuddleRing {
        conversation: ConversationId,
        huddle: HuddleId,
        from: UserId,
    },
    HuddleRingCancelled {
        conversation: ConversationId,
    },
    /// Whether `user` is in any huddle.
    InHuddle {
        user: UserId,
        active: bool,
    },
    PinChanged {
        id: MessageId,
        pinned: bool,
    },
    BookmarksChanged(ConversationId),
    SavedChanged,
    ThreadInboxChanged,
    /// The user now follows (or no longer follows) the thread under `root`.
    ThreadFollowChanged {
        root: MessageId,
        followed: bool,
    },
    WorkspaceRenamed {
        name: Arc<str>,
    },
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ProviderError {
    #[error("session expired")]
    SessionExpired,
    #[error("not allowed: {0}")]
    NotAllowed(String),
    /// The server is rate limiting. The provider has already waited out the
    /// server's hold and holds its next call too, so a caller may retry at once.
    #[error("rate limited")]
    RateLimited,
    #[error("network error: {0}")]
    Network(String),
    /// A download was larger than the limit asked for.
    #[error("too large")]
    TooLarge,
    #[error("{0}")]
    Other(String),
    /// The provider does not implement this operation.
    #[error("not supported")]
    Unsupported,
    /// Text from the provider's own catalog, rendered by the shell in the
    /// active language (`ns` is the provider catalog's namespace).
    #[error("{}.{}", .0.ns, .0.id)]
    Localized(i18n::LocalizedText),
}

/// What the reducer asks the store to do (`store` depends on `domain`, never
/// the reverse).
#[derive(Clone, Debug)]
pub enum PersistOp {
    Conversations(Vec<Conversation>),
    Members(Vec<Member>),
    Messages {
        conversation: ConversationId,
        messages: Vec<Message>,
    },
    EditMessage {
        id: MessageId,
        text: Arc<str>,
        plain: Arc<str>,
        edited: MessageTs,
    },
    DeleteMessage(MessageId),
    ReadMarker {
        conversation: ConversationId,
        ts: MessageTs,
    },
    ReplaceMessage(Message),
    Reactions {
        id: MessageId,
        reactions: Arc<[Reaction]>,
    },
    ThreadSummary {
        root: MessageId,
        reply_count: u32,
        latest_reply: Option<MessageTs>,
    },
    AddGap(Gap),
    /// Removes exactly that range, splitting any stored gap that covers more.
    CloseGap(Gap),
    Opened {
        conversation: ConversationId,
        at_ms: i64,
    },
    FileDeleted(FileId),
    LocalSchedulePut(crate::schedule::LocalSchedule),
    LocalScheduleRemove(crate::schedule::ScheduleId),
    /// Sets a conversation's Parla-side notification level and mute flag, for
    /// providers that cannot keep them. `Default` and not muted deletes the row.
    LocalNotifyPrefsPut {
        conversation: ConversationId,
        prefs: ConversationPrefs,
    },
    /// Sets a conversation's Parla-side extras; the default deletes the row.
    LocalNotifyExtrasPut {
        conversation: ConversationId,
        extras: crate::people::NotifyExtras,
    },
    /// Stars or unstars a conversation (Parla's own, the provider keeps none).
    LocalStarredPut {
        conversation: ConversationId,
        starred: bool,
    },
    /// Adds or replaces a reminder kept by Parla itself (the provider keeps none).
    LocalReminderPut(crate::reminder::Reminder),
    LocalReminderRemove(crate::reminder::ReminderId),
    /// Replaces the workspace's user groups (already bounded).
    UserGroups(Vec<crate::user_groups::UserGroup>),
}
