//! One generic effect pair for provider writes (`ProviderCall`) and reads
//! (`Load`); feature tasks add the reducer rules that consume the results.

use std::collections::HashMap;
use std::sync::Arc;

use crate::huddle::{Huddle, HuddleId};
use crate::ids::{ConversationId, MessageId, MessageTs, UserId};
use crate::model::{Conversation, Member, Message};
use crate::people::{ConversationPrefs, DndStatus, PresenceSetting, Profile, Status};
use crate::reminder::{Reminder, ReminderId};
use crate::saved::{Bookmark, BookmarkId, InboxThread, Pin, SavedItem};
use crate::schedule::ScheduledMessage;
use crate::search::Page;

#[derive(Clone, Debug)]
pub enum ProviderCall {
    Schedule {
        conversation: ConversationId,
        thread: Option<MessageTs>,
        text: Arc<str>,
        post_at: i64,
    },
    DeleteScheduled(ScheduledMessage),
    AddReminder {
        text: Arc<str>,
        when: Arc<str>,
        user: Option<UserId>,
    },
    CompleteReminder(ReminderId),
    DeleteReminder(ReminderId),
    SetStatus(Option<Status>),
    SetPresence(PresenceSetting),
    /// Watch these people's presence (replaces the previous set).
    WatchPresence(Vec<UserId>),
    /// Open (or create) the direct message with a person.
    OpenDirect(UserId),
    Snooze(u32),
    EndSnooze,
    SetNotifyPrefs {
        conversation: ConversationId,
        prefs: ConversationPrefs,
    },
    DeclineHuddle {
        conversation: ConversationId,
        id: HuddleId,
    },
    SetPinned {
        id: MessageId,
        pinned: bool,
    },
    SetSaved {
        id: MessageId,
        saved: bool,
    },
    AddBookmark {
        conversation: ConversationId,
        title: Arc<str>,
        link: Arc<str>,
    },
    RemoveBookmark {
        conversation: ConversationId,
        id: BookmarkId,
    },
    SetThreadFollowed {
        root: MessageId,
        follow: bool,
        /// The user's read cursor in the thread (its newest delivered reply), when known.
        last_read: Option<MessageTs>,
    },
    /// Star or unstar a conversation on the provider too
    /// (`Capabilities::conversation_stars`).
    SetConversationStarred {
        conversation: ConversationId,
        starred: bool,
    },
    /// Channel admin; `Create` and `Join` answer `CallOutput::Conversation`.
    Admin(crate::channel_admin::AdminCall),
}

#[derive(Clone, Debug)]
pub enum CallOutput {
    None,
    Scheduled(ScheduledMessage),
    Reminder(Reminder),
    Dnd(DndStatus),
    Bookmark(Bookmark),
    Conversation(Conversation),
    /// A channel invite, person by person.
    Invite(crate::channel_admin::InviteReport),
}

#[derive(Clone, Debug)]
pub enum Load {
    Scheduled,
    Reminders,
    Dnd,
    DndUsers(Vec<UserId>),
    NotifyPrefs,
    Profile(UserId),
    Huddles,
    Pins(ConversationId),
    Bookmarks(ConversationId),
    Saved {
        cursor: Option<Arc<str>>,
    },
    ThreadInbox {
        cursor: Option<Arc<str>>,
    },
    Message(MessageId),
    /// A conversation by id, for a live change to one not held yet.
    Conversation(ConversationId),
    /// Members by id, for authors the directory did not carry.
    Members(Vec<UserId>),
}

#[derive(Clone, Debug)]
pub enum Loaded {
    Scheduled(Vec<ScheduledMessage>),
    Reminders(Vec<Reminder>),
    Dnd(DndStatus),
    DndUsers(HashMap<UserId, DndStatus>),
    NotifyPrefs(HashMap<ConversationId, ConversationPrefs>),
    Profile(Profile),
    Huddles(Vec<Huddle>),
    Pins(ConversationId, Vec<Pin>),
    Bookmarks(ConversationId, Vec<Bookmark>),
    Saved(Page<SavedItem>),
    ThreadInbox(Page<InboxThread>),
    Message(Message),
    /// A conversation by id; `None` fields are unknown, not empty.
    Conversation(Conversation),
    /// The ids asked for and the members found (ids not found are absent).
    Members {
        asked: Vec<UserId>,
        found: Vec<Member>,
    },
}
