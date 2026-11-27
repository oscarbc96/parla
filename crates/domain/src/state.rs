use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use settings::WorkspaceKey;

use crate::caps::Capabilities;
use crate::collections::CollectionsBook;
use crate::dnd_logic::NotifyBook;
use crate::huddle::Huddle;
use crate::huddles_logic::{HuddleLoad, Ring, RingClock};
use crate::ids::ClientMsgId;
use crate::ids::MessageId;
use crate::ids::{ConversationId, UserId};
use crate::model::{Conversation, Member, Message};
use crate::outbox::OutboxItem;
use crate::people::{ConversationPrefs, DndStatus, PresenceSetting, Profile};
use crate::profiles::ProfileBook;
use crate::reactions::CustomEmoji;
use crate::reminder::{Reminder, ReminderId};
use crate::saved::{Bookmark, InboxThread, Pin, SavedItem};
use crate::schedule::{LocalSchedule, ScheduledMessage};
use crate::scheduling::SchedBook;
use crate::search::Page;
use crate::threads::ThreadWindow;
use crate::unread::Focus;
use crate::window::MessageWindow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Connecting,
    Online,
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    Valid,
    Expired,
}

#[derive(Clone, Debug)]
pub struct WorkspaceState {
    pub workspace: WorkspaceKey,
    pub self_user: Option<UserId>,
    pub name: Arc<str>,
    pub conversations: BTreeMap<ConversationId, Conversation>,
    pub members: HashMap<UserId, Member>,
    pub windows: HashMap<ConversationId, MessageWindow>,
    /// Open threads, keyed by root.
    pub threads: HashMap<MessageId, ThreadWindow>,
    /// Follow state per thread root, as the user or the provider last set it
    /// (at most `MAX_THREAD_FOLLOW`); see `thread_followed`.
    pub(crate) thread_follow: crate::thread_follow::FollowStates,
    pub custom_emoji: HashMap<Arc<str>, CustomEmoji>,
    /// The workspace's user groups (at most `MAX_USER_GROUPS`), for `@` completion
    /// and group mentions.
    pub user_groups: crate::user_groups::UserGroups,
    /// The provider answered with the groups, so a late store read is ignored.
    pub(crate) user_groups_fresh: bool,
    pub connection: Connection,
    pub session: Session,
    /// Lean mode: no message windows held, counts and notifications only.
    pub lean: bool,
    /// Conversations asked for by id and not yet answered (a live change to one
    /// the app does not hold), with what the change said (at most
    /// `MAX_CONVERSATION_FETCHES`).
    pub(crate) conversation_fetching: HashMap<ConversationId, crate::model::ConversationMeta>,
    /// The row the timeline scrolls to and highlights once (a jump).
    pub jump: Option<MessageId>,
    /// From `performance.message_window` (default 200).
    pub window_capacity: usize,
    /// Provider-side scheduled sends, by `post_at`.
    pub scheduled: Vec<ScheduledMessage>,
    /// Schedules Parla keeps and sends itself, by `post_at`.
    pub local_schedules: Vec<LocalSchedule>,
    /// Reminders for self; `None` until loaded.
    pub reminders: Option<Vec<Reminder>>,
    /// Reminders removed optimistically, with the id they followed, for
    /// rollback when the provider refuses the delete.
    pub(crate) reminders_undo: Vec<(Option<ReminderId>, Reminder)>,
    /// Do-not-disturb status per user (self and people on screen).
    pub dnd: HashMap<UserId, DndStatus>,
    /// Per-conversation notification levels and mute flags.
    pub notify_prefs: HashMap<ConversationId, ConversationPrefs>,
    /// Levels and mute flags kept by Parla when the provider cannot store them
    /// (`caps.notify_prefs` is false); persisted in the store.
    pub local_notify_prefs: HashMap<ConversationId, ConversationPrefs>,
    /// Parla's own per-conversation extras (thread replies, sound, timed
    /// mute); persisted in the store. A default entry is never kept.
    pub local_notify_extras: HashMap<ConversationId, crate::people::NotifyExtras>,
    /// Conversations the user starred. Kept by Parla itself (local, per
    /// workspace, persisted in the store); no provider is asked.
    pub starred: std::collections::HashSet<ConversationId>,
    /// Ids the user starred or unstarred this session: they win over the
    /// stored set when it loads after them.
    pub(crate) starred_touched: std::collections::HashSet<ConversationId>,
    /// Pinned messages per conversation, loaded on demand.
    pub pins: HashMap<ConversationId, Vec<Pin>>,
    /// Bookmarks per conversation, loaded on open.
    pub bookmarks: HashMap<ConversationId, Vec<Bookmark>>,
    /// Saved for later; `None` until loaded.
    pub saved: Option<Page<SavedItem>>,
    /// The aggregate Threads view; `None` until loaded.
    pub thread_inbox: Option<Page<InboxThread>>,
    /// Messages fetched for those lists, at most `MAX_REFERENCED`, oldest
    /// fetched dropped first, cleared in lean mode.
    pub referenced: HashMap<MessageId, Message>,
    /// Loaded profiles, at most `PROFILE_CACHE`, oldest loaded dropped first.
    pub profiles: HashMap<UserId, Profile>,
    /// The profile on screen, if any.
    pub open_profile: Option<UserId>,
    /// Own presence setting; `None` until set or after a failed write.
    pub presence_setting: Option<PresenceSetting>,
    /// The people whose presence is on screen (DM rows, open DM, open
    /// profile), sorted; what the provider was last asked to watch.
    pub presence_watch: BTreeSet<UserId>,
    /// Whether each watched person is active; absent while unknown.
    pub presence: HashMap<UserId, bool>,
    /// Live huddle per conversation.
    pub huddles: HashMap<ConversationId, Huddle>,
    /// Users currently in any huddle.
    pub in_huddle: HashSet<UserId>,
    /// The ring being shown, if any.
    pub ring: Option<Ring>,
    /// From `calls.ring` (`Input::SetRingEnabled`).
    pub ring_enabled: bool,
    /// Unix seconds, from `Input::Clock`.
    pub now_unix: i64,
    /// False after a reconnect until the huddle list reloads or 5 s pass.
    pub ring_armed: bool,
    pub(crate) now_mono: Option<Instant>,
    pub(crate) ring_clock: RingClock,
    /// Huddle list loads in flight and the live updates that overtook them.
    pub(crate) huddle_load: HuddleLoad,
    /// Who is typing where, and the outbound typing throttle.
    pub(crate) typing: crate::typing::TypingBook,
    /// What the provider supports, so the reducer can gate reads.
    pub caps: Capabilities,
    /// In-flight pin, save and bookmark writes (a failure refetches).
    pub(crate) collections: CollectionsBook,
    /// In-flight optimistic notify writes (a failure refetches).
    pub(crate) notify: NotifyBook,
    /// Load order of `profiles`, oldest first.
    pub(crate) profile_order: VecDeque<UserId>,
    /// In-flight optimistic own status and presence writes.
    pub(crate) profile_book: ProfileBook,
    pub(crate) member_book: crate::members_logic::MemberBook,
    pub(crate) sched: SchedBook,
    pub(crate) outbox: Vec<OutboxItem>,
    /// The outbox items whose `CallSend` has not finished yet, in start
    /// order (at most `MAX_IN_FLIGHT`).
    pub(crate) inflight: Vec<ClientMsgId>,
    /// The last few sends that settled (by echo or answer): a duplicate echo
    /// of one must not be taken for the echo of the send now in flight.
    pub(crate) settled: VecDeque<MessageId>,
    pub(crate) focus: Option<Focus>,
    /// Conversations where "mark unread" suppresses automatic marking.
    pub(crate) unread_hold: HashSet<ConversationId>,
}

impl WorkspaceState {
    /// The conversation's notification level and mute flag: the provider's
    /// when it keeps them (`caps.notify_prefs`), else Parla's own.
    pub fn prefs_for(&self, c: &ConversationId) -> Option<&ConversationPrefs> {
        if self.caps.notify_prefs {
            self.notify_prefs.get(c)
        } else {
            self.local_notify_prefs.get(c)
        }
    }

    /// Unsent messages in send order.
    /// The client ids of the sends whose calls have not finished yet.
    pub fn outbox_inflight(&self) -> &[ClientMsgId] {
        &self.inflight
    }

    pub fn outbox(&self) -> &[OutboxItem] {
        &self.outbox
    }

    pub fn new(workspace: WorkspaceKey, window_capacity: usize) -> Self {
        Self {
            workspace,
            self_user: None,
            name: Arc::from(""),
            conversations: BTreeMap::new(),
            members: HashMap::new(),
            windows: HashMap::new(),
            threads: HashMap::new(),
            thread_follow: Default::default(),
            custom_emoji: HashMap::new(),
            user_groups: HashMap::new(),
            user_groups_fresh: false,
            connection: Connection::Connecting,
            session: Session::Valid,
            lean: false,
            jump: None,
            conversation_fetching: HashMap::new(),
            window_capacity,
            scheduled: Vec::new(),
            local_schedules: Vec::new(),
            reminders: None,
            reminders_undo: Vec::new(),
            dnd: HashMap::new(),
            notify_prefs: HashMap::new(),
            local_notify_prefs: HashMap::new(),
            local_notify_extras: HashMap::new(),
            starred: std::collections::HashSet::new(),
            starred_touched: std::collections::HashSet::new(),
            pins: HashMap::new(),
            bookmarks: HashMap::new(),
            saved: None,
            thread_inbox: None,
            referenced: HashMap::new(),
            profiles: HashMap::new(),
            open_profile: None,
            presence_setting: None,
            presence_watch: BTreeSet::new(),
            presence: HashMap::new(),
            huddles: HashMap::new(),
            in_huddle: HashSet::new(),
            ring: None,
            ring_enabled: true,
            now_unix: 0,
            ring_armed: true,
            now_mono: None,
            ring_clock: RingClock {
                arm_at: None,
                dismissed: Vec::new(),
            },
            huddle_load: HuddleLoad::default(),
            typing: Default::default(),
            caps: Capabilities::default(),
            collections: CollectionsBook::default(),
            notify: NotifyBook::default(),
            profile_order: VecDeque::new(),
            profile_book: ProfileBook::default(),
            member_book: Default::default(),
            sched: SchedBook::default(),
            outbox: Vec::new(),
            inflight: Vec::new(),
            settled: VecDeque::new(),
            focus: None,
            unread_hold: HashSet::new(),
        }
    }
}
