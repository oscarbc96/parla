//! Pure workspace reducer: no I/O, no GPUI. The `parla` binary executes the
//! returned effects.

use std::sync::Arc;
use std::time::Instant;

use crate::calls::{CallOutput, Load, Loaded, ProviderCall};
use crate::collections;
use crate::dnd_logic;
use crate::huddles_logic;
use crate::huddles_logic::RingEnd;
use crate::ids::{ClientMsgId, ConversationId, MessageId, MessageTs, UserId};
use crate::message_events;
use crate::model::{
    Bootstrap, DeliveryState, HistoryPage, Message, PersistOp, ProviderError, ProviderEvent,
};
use crate::outbox::{OutboxItem, OutboxState};
use crate::outbox_logic::{self, OutboxOp};
use crate::people::{PresenceSetting, Status};
use crate::presence_logic;
use crate::profiles;
use crate::reactions::CustomEmoji;
use crate::reactions_logic;
use crate::reminder::ReminderId;
use crate::reminders_logic;
use crate::rich::EmojiRef;
use crate::schedule::{LocalSchedule, ScheduleId, ScheduledMessage};
use crate::scheduling;
use crate::state::{Connection, Session, WorkspaceState};
use crate::sync::{self, CatchUpResult};
use crate::sync_types::ConversationCounts;
use crate::threads;
use crate::unread;
use crate::window::MessageWindow;

#[derive(Clone, Debug)]
pub enum Input {
    Bootstrapped(Bootstrap),
    /// What the local store held at startup: painted before the first
    /// network answer. Never written back; the newest window of one
    /// conversation, ascending.
    CacheLoaded {
        conversations: Vec<crate::model::Conversation>,
        members: Vec<crate::model::Member>,
        conversation: Option<ConversationId>,
        messages: Vec<Message>,
    },
    Provider(ProviderEvent),
    HistoryLoaded {
        conversation: ConversationId,
        page: HistoryPage,
    },
    Open(ConversationId),
    /// A conversation found by a search or browse, to show before the user
    /// joins it: held as a non-member when the list lacks it. Preview itself
    /// persists nothing; a later meta change (`apply_meta`) may store the
    /// held record, which is harmless (a non-member is never listed).
    Preview(crate::model::Conversation),
    /// A catch-up finished; never notifies.
    CaughtUp(CatchUpResult),
    /// A page requested by `Effect::FetchBetween` for a gap fill.
    GapPageLoaded {
        conversation: ConversationId,
        after: Option<MessageTs>,
        before: MessageTs,
        page: HistoryPage,
    },
    Send {
        conversation: ConversationId,
        text: Arc<str>,
        thread: Option<MessageTs>,
        id: ClientMsgId,
        /// The local timestamp of the optimistic row.
        now: MessageTs,
        /// Unix milliseconds of the same instant, for outbox ordering.
        now_ms: i64,
    },
    SendFinished {
        conversation: ConversationId,
        id: ClientMsgId,
        result: Result<Message, ProviderError>,
    },
    OutboxLoaded(Vec<OutboxItem>),
    Retry(ClientMsgId),
    /// The retry timer of `Effect::RetryOutbox` fired: send the next queued item.
    OutboxRetry,
    Discard(ClientMsgId),
    Edit {
        id: MessageId,
        text: Arc<str>,
    },
    Delete(MessageId),
    CountsLoaded(Vec<ConversationCounts>),
    Focus {
        conversation: Option<ConversationId>,
        at_bottom: bool,
        window_active: bool,
    },
    MarkUnread(MessageId),
    /// Marks a conversation read up to `ts` (the aggregate views' "Mark all read").
    MarkRead {
        conversation: ConversationId,
        ts: MessageTs,
    },
    OpenThread(MessageId),
    CloseThread(MessageId),
    /// Ascending, root first when it was requested without `after`.
    RepliesLoaded {
        root: MessageId,
        page: HistoryPage,
    },
    Reply {
        root: MessageId,
        text: Arc<str>,
        broadcast: bool,
        id: ClientMsgId,
        /// The local timestamp of the optimistic row.
        now: MessageTs,
        /// Unix milliseconds of the same instant, for outbox ordering.
        now_ms: i64,
    },
    /// Follow (`follow`) or unfollow the thread under `root`.
    SetThreadFollowed {
        root: MessageId,
        follow: bool,
    },
    React {
        id: MessageId,
        emoji: EmojiRef,
        add: bool,
    },
    ReactFinished {
        id: MessageId,
        emoji: EmojiRef,
        add: bool,
        result: Result<(), ProviderError>,
    },
    CustomEmojiLoaded(Vec<CustomEmoji>),
    /// The provider's user groups (`Effect::FetchUserGroups`).
    UserGroupsLoaded(Vec<crate::user_groups::UserGroup>),
    /// The user groups the store held at startup; never written back.
    UserGroupsCached(Vec<crate::user_groups::UserGroup>),
    /// A `Effect::Call` finished.
    CallFinished {
        call: ProviderCall,
        result: Result<CallOutput, ProviderError>,
    },
    /// An `Effect::Load` finished.
    Loaded(Result<Loaded, (Load, ProviderError)>),
    /// Enter or leave lean mode.
    SetLean(bool),
    /// Schedule a send. Files (or a provider that cannot schedule) keep it
    /// local; `local_id` names the local copy.
    ScheduleSend {
        conversation: ConversationId,
        thread: Option<MessageTs>,
        text: Arc<str>,
        files: Vec<crate::files::Upload>,
        post_at: i64,
        local_id: ScheduleId,
        now_ms: i64,
    },
    Reschedule {
        item: ScheduledMessage,
        text: Arc<str>,
        post_at: i64,
    },
    DeleteSchedule(ScheduleId),
    LocalSchedulesLoaded(Vec<LocalSchedule>),
    /// The wake timer fired (unix seconds).
    LocalTick {
        now: i64,
    },
    SendLateAnswer {
        ids: Vec<ScheduleId>,
        send: bool,
    },
    LocalSent {
        id: ScheduleId,
        result: Result<(), ProviderError>,
    },
    /// Open a message that may be far from the live end.
    JumpTo(MessageId),
    /// The answer of `Effect::FetchAround`.
    AroundLoaded {
        target: MessageId,
        page: crate::search::AroundPage,
    },
    /// The answer of `Effect::FetchNewer`, ascending.
    NewerLoaded {
        conversation: ConversationId,
        page: HistoryPage,
    },
    /// Leave a detached window: back to the newest messages.
    JumpToLatest(ConversationId),
    /// Ask the provider to remind self or `user`; `when` is verbatim.
    AddReminder {
        text: Arc<str>,
        when: Arc<str>,
        user: Option<crate::ids::UserId>,
    },
    CompleteReminder(ReminderId),
    DeleteReminder(ReminderId),
    /// Add a reminder kept in Parla itself (the provider has none): the session
    /// parsed `when` into `due`; persisted, no provider call.
    AddLocalReminder {
        id: ReminderId,
        text: Arc<str>,
        due: i64,
        recurring: bool,
        /// Seconds between occurrences when `recurring`.
        interval_secs: i64,
        user: crate::ids::UserId,
        /// The message the reminder is about, if any.
        link: Option<crate::reminder::ReminderLink>,
    },
    /// Move one of Parla's own reminders to `until` (unix seconds); it alerts again.
    SnoozeReminder {
        id: ReminderId,
        until: i64,
    },
    /// Change one of Parla's own reminders; `due: None` keeps its time.
    EditReminder {
        id: ReminderId,
        text: Arc<str>,
        due: Option<i64>,
        repeat: crate::reminder::ReminderRepeat,
        /// Unix seconds now: a reminder set to repeat moves past it.
        now: i64,
    },
    /// The reminders read from the store at startup (Parla-side list).
    LocalRemindersLoaded(Vec<crate::reminder::Reminder>),
    /// The reminder timer fired, or Parla started or reminders changed (unix
    /// seconds): Parla's own reminders now due raise their alert, once.
    ReminderTick {
        now: i64,
    },
    /// Show the reminder list, loading it once when it is not held yet.
    OpenReminders,
    /// Pause notifications (self DND) for `minutes`; optimistic.
    Snooze {
        minutes: u32,
        now_unix: i64,
    },
    /// End a self-DND snooze; optimistic.
    EndSnooze,
    /// Set a conversation's notification level; optimistic.
    SetNotifyPrefs {
        conversation: ConversationId,
        prefs: crate::people::ConversationPrefs,
    },
    /// Set a conversation's level in Parla itself (the provider keeps none).
    SetLocalNotifyPrefs {
        conversation: ConversationId,
        prefs: crate::people::ConversationPrefs,
    },
    /// Parla's own levels, read from the store once per session.
    LocalNotifyPrefsLoaded(
        std::collections::HashMap<ConversationId, crate::people::ConversationPrefs>,
    ),
    /// Set a conversation's thread, sound and timed-mute extras (Parla's own).
    SetNotifyExtras {
        conversation: ConversationId,
        extras: crate::people::NotifyExtras,
    },
    /// Parla's own extras, read from the store once per session.
    NotifyExtrasLoaded(std::collections::HashMap<ConversationId, crate::people::NotifyExtras>),
    /// A timed mute reached its deadline (fired by the one-shot timer).
    NotifyMuteTick {
        now: i64,
    },
    /// Star or unstar a conversation in Parla itself: no provider call, persisted.
    SetStarred {
        conversation: ConversationId,
        starred: bool,
    },
    /// Parla's own stars, read from the store once per session.
    StarredLoaded(std::collections::HashSet<ConversationId>),
    /// Set or clear self status; optimistic, a failure refetches.
    SetStatus(Option<Status>),
    /// Set self presence; optimistic, a failure leaves it unknown.
    SetPresence(PresenceSetting),
    /// The people whose presence is on screen now (the host debounces it).
    WatchPresence(Vec<UserId>),
    /// Start (or find) the direct message with a person and have it known.
    OpenDirect(UserId),
    /// A channel admin write (create, join, leave, ...), when the provider
    /// has `Capabilities::channel_admin`.
    Admin(crate::channel_admin::AdminCall),
    /// Show a conversation's pins, loading them once.
    OpenPins(ConversationId),
    /// Show a conversation's bookmarks, loading them once.
    OpenBookmarks(ConversationId),
    /// Show Saved for later, loading it once.
    OpenSaved,
    /// Show the Threads inbox, loading it once.
    OpenThreadInbox,
    /// The next page of Saved for later.
    MoreSaved,
    /// The next page of the Threads inbox.
    MoreThreadInbox,
    /// Pin or unpin a message; optimistic, a failure refetches.
    SetPinned {
        id: MessageId,
        pinned: bool,
    },
    /// Save or unsave a message for later; optimistic, a failure refetches.
    SetSaved {
        id: MessageId,
        saved: bool,
    },
    /// Add a bookmark to a conversation.
    AddBookmark {
        conversation: ConversationId,
        title: Arc<str>,
        link: Arc<str>,
    },
    /// Remove a bookmark; optimistic, a failure refetches.
    RemoveBookmark {
        conversation: ConversationId,
        id: crate::saved::BookmarkId,
    },
    /// Show a profile, loading it unless it is cached.
    OpenProfile(crate::ids::UserId),
    /// The profile view closed.
    CloseProfile,
    /// Wall and monotonic time, sent before each batch of provider events.
    Clock {
        unix: i64,
        mono: Instant,
    },
    /// From `calls.ring`, on start and on settings change.
    SetRingEnabled(bool),
    /// Join the huddle of a conversation (also ends its ring).
    JoinHuddle(ConversationId),
    /// Decline the active ring.
    DeclineRing,
    /// The ring timer fired.
    RingTimeout {
        now: Instant,
    },
    /// The user typed in the draft of `conversation` (its thread `thread`
    /// when set); a restored or programmatic text change is not typing.
    Typed {
        conversation: ConversationId,
        thread: Option<MessageTs>,
        now: Instant,
    },
    /// The typing expiry timer of `Effect::TypingWakeAt` fired.
    TypingExpired {
        now: Instant,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Changed {
    Conversations,
    Timeline(ConversationId),
    Members,
    Connection,
    Session,
    Thread(MessageId),
    Emoji,
    Lean,
    Scheduled,
    /// `WorkspaceState::jump` or a window's detached state changed.
    Jump(ConversationId),
    Reminders,
    /// DND status changed (self or someone on screen).
    Dnd,
    /// Per-conversation notification levels changed.
    NotifyPrefs,
    /// A conversation's pins changed.
    Pins(ConversationId),
    /// A conversation's bookmarks changed.
    Bookmarks(ConversationId),
    /// Saved for later changed.
    Saved,
    /// The Threads inbox changed.
    ThreadInbox,
    /// A profile, or the self presence setting, changed.
    Profile(crate::ids::UserId),
    /// Someone on screen became active or away, or the watched set changed.
    Presence,
    /// Active huddles or `in_huddle` changed.
    Huddles,
    /// `WorkspaceState::ring` changed.
    Ring,
    /// Who is typing in this conversation (or one of its threads) changed.
    Typing(ConversationId),
}

#[derive(Clone, Debug)]
pub enum Effect {
    Persist(PersistOp),
    FetchHistory {
        conversation: ConversationId,
        before: Option<MessageTs>,
    },
    /// Gap filling: messages strictly between `after` and `before`.
    FetchBetween {
        conversation: ConversationId,
        after: Option<MessageTs>,
        before: MessageTs,
    },
    MarkRead {
        conversation: ConversationId,
        ts: MessageTs,
    },
    CallSend {
        conversation: ConversationId,
        text: Arc<str>,
        id: ClientMsgId,
    },
    Outbox(OutboxOp),
    FetchReplies {
        root: MessageId,
        after: Option<MessageTs>,
    },
    CallReply {
        root: MessageId,
        text: Arc<str>,
        broadcast: bool,
        id: ClientMsgId,
    },
    CallReact {
        id: MessageId,
        emoji: EmojiRef,
        add: bool,
    },
    MarkThreadRead {
        root: MessageId,
        ts: MessageTs,
    },
    FetchCustomEmoji,
    /// Lists the workspace's user groups; answered with `Input::UserGroupsLoaded`.
    FetchUserGroups,
    CallEdit {
        id: MessageId,
        text: Arc<str>,
    },
    CallDelete {
        id: MessageId,
    },
    /// A provider write; the answer returns as `Input::CallFinished`.
    Call(ProviderCall),
    /// A provider read; the answer returns as `Input::Loaded`.
    Load(Load),
    /// Live posts only; never emitted for history or catch-up.
    MaybeNotify {
        conversation: ConversationId,
        message: MessageId,
    },
    /// Messages around `target` (a jump outside the held window).
    FetchAround {
        target: MessageId,
    },
    /// Messages newer than `after` for a detached window.
    FetchNewer {
        conversation: ConversationId,
        after: MessageTs,
    },
    /// Reload a conversation's window after leaving lean mode (store first).
    Reload(ConversationId),
    /// Send a local schedule now; the answer returns as `Input::LocalSent`.
    SendLocal(LocalSchedule),
    /// Ask whether to send overdue local schedules (`Input::SendLateAnswer`).
    AskLate(Vec<ScheduleId>),
    /// Arm one one-shot timer; when it fires answer `Input::OutboxRetry`. Sent
    /// when a send failed transiently while still online, since no reconnect
    /// will follow to flush the queue.
    RetryOutbox {
        after_secs: u32,
    },
    /// Arm one timer for this unix time (`Input::LocalTick`); `None` cancels.
    WakeAt(Option<i64>),
    /// Raise the desktop alert of a due local reminder (already marked fired).
    ReminderDue(crate::reminder::Reminder),
    /// Show and sound a huddle ring. The shell arms one one-shot deadline
    /// at `since + RING_TIMEOUT` (answered by `Input::RingTimeout`) and
    /// cancels it on `StopRing`.
    StartRing(huddles_logic::Ring),
    StopRing {
        conversation: ConversationId,
        reason: RingEnd,
    },
    /// Open the huddle per `calls.join_with`.
    OpenHuddle {
        conversation: ConversationId,
    },
    /// Tell the provider the user is typing (already throttled); nothing is
    /// fed back.
    SendTyping {
        conversation: ConversationId,
        thread: Option<MessageTs>,
    },
    /// Arm the one typing expiry timer for this instant (answered by
    /// `Input::TypingExpired`), replacing any earlier one; `None` cancels it.
    TypingWakeAt(Option<Instant>),
    Changed(Changed),
}

pub fn reduce(state: &mut WorkspaceState, input: Input) -> Vec<Effect> {
    let mut fx = Vec::new();
    collections::note_input(state, &input);
    match input {
        Input::Bootstrapped(b) => bootstrapped(state, b, &mut fx),
        Input::CacheLoaded {
            conversations,
            members,
            conversation,
            messages,
        } => {
            for c in conversations {
                state.conversations.entry(c.id.clone()).or_insert(c);
            }
            for m in members {
                state.members.entry(m.id.clone()).or_insert(m);
            }
            if let Some(c) = conversation
                && !messages.is_empty()
            {
                let w = window(state, &c);
                if w.is_empty() {
                    for m in messages {
                        w.insert(m);
                    }
                    w.set_has_more_before(true);
                    fx.push(Effect::Changed(Changed::Timeline(c)));
                }
            }
            fx.push(Effect::Changed(Changed::Conversations));
            fx.push(Effect::Changed(Changed::Members));
        }
        Input::Provider(e) => provider(state, e, &mut fx),
        Input::HistoryLoaded { conversation, page } => {
            let w = window(state, &conversation);
            w.mark_loaded();
            let before = w.has_more_before();
            w.set_has_more_before(false);
            for m in page.messages.iter().cloned() {
                w.insert(m);
            }
            let dropped = w.has_more_before();
            w.set_has_more_before(page.has_more || dropped || (before && page.messages.is_empty()));
            fx.push(Effect::Persist(PersistOp::Messages {
                conversation: conversation.clone(),
                messages: page.messages,
            }));
            fx.push(Effect::Changed(Changed::Timeline(conversation)));
        }
        Input::SetLean(on) => crate::lean::set_lean(state, on, &mut fx),
        Input::JumpTo(id) => crate::jump::jump_to(state, id, &mut fx),
        Input::AroundLoaded { target, page } => {
            crate::jump::around_loaded(state, target, page, &mut fx)
        }
        Input::NewerLoaded { conversation, page } => {
            crate::jump::newer_loaded(state, conversation, page, &mut fx)
        }
        Input::JumpToLatest(c) => crate::jump::to_latest(state, c, &mut fx),
        Input::Snooze { minutes, now_unix } => {
            dnd_logic::snooze(state, minutes, now_unix, &mut fx);
        }
        Input::EndSnooze => dnd_logic::end_snooze(state, &mut fx),
        Input::SetNotifyPrefs {
            conversation,
            prefs,
        } => {
            dnd_logic::set_prefs(state, conversation, prefs, &mut fx);
        }
        Input::SetLocalNotifyPrefs {
            conversation,
            prefs,
        } => dnd_logic::set_local_prefs(state, conversation, prefs, &mut fx),
        Input::SetNotifyExtras {
            conversation,
            extras,
        } => dnd_logic::set_extras(state, conversation, extras, &mut fx),
        Input::NotifyExtrasLoaded(all) => {
            if state.local_notify_extras != all {
                state.local_notify_extras = all;
                fx.push(Effect::Changed(Changed::NotifyPrefs));
            }
        }
        Input::NotifyMuteTick { now } => dnd_logic::mute_tick(state, now, &mut fx),
        Input::SetStarred {
            conversation,
            starred,
        } => {
            // A first write for an id is stored even when it changes nothing
            // in memory: the stored set may not have loaded yet.
            let first = state.starred_touched.insert(conversation.clone());
            let changed = if starred {
                state.starred.insert(conversation.clone())
            } else {
                state.starred.remove(&conversation)
            };
            if changed {
                fx.push(Effect::Changed(Changed::Conversations));
                // A provider that keeps stars gets the same change; a failure
                // leaves Parla's own list as it is.
                if state.caps.conversation_stars {
                    fx.push(Effect::Call(ProviderCall::SetConversationStarred {
                        conversation: conversation.clone(),
                        starred,
                    }));
                }
            }
            if changed || first {
                fx.push(Effect::Persist(crate::PersistOp::LocalStarredPut {
                    conversation,
                    starred,
                }));
            }
        }
        Input::StarredLoaded(mut all) => {
            // What the user changed before the load arrived wins over it.
            for c in &state.starred_touched {
                if state.starred.contains(c) {
                    all.insert(c.clone());
                } else {
                    all.remove(c);
                }
            }
            if state.starred != all {
                state.starred = all;
                fx.push(Effect::Changed(Changed::Conversations));
            }
        }
        Input::LocalNotifyPrefsLoaded(all) => {
            if state.local_notify_prefs != all {
                state.local_notify_prefs = all;
                fx.push(Effect::Changed(Changed::NotifyPrefs));
            }
        }
        Input::Open(c) => open(state, c, &mut fx),
        Input::Preview(c) => preview(state, c, &mut fx),
        Input::CaughtUp(r) => {
            let c = r.conversation;
            if let Some(w) = state.windows.get_mut(&c).filter(|w| w.is_live()) {
                if let Some(g) = r.gap.as_ref().filter(|_| !w.is_empty()) {
                    // The held messages end before the gap, so keeping them
                    // would leave a hole that scrolling up never crosses
                    // (`sync::next_older` looks for a gap at the oldest held
                    // message). Start from the new tail, keep what is newer
                    // than the gap or still pending, and let the gap fill
                    // as the oldest held message is passed.
                    let keep: Vec<Message> = w
                        .messages()
                        .iter()
                        .filter(|m| m.id.ts >= g.before || m.state != DeliveryState::Delivered)
                        .cloned()
                        .collect();
                    w.replace(r.messages.clone(), None, true, false);
                    for m in keep {
                        w.insert(m);
                    }
                } else {
                    for m in r.messages.iter().cloned() {
                        w.insert(m);
                    }
                }
                fx.push(Effect::Changed(Changed::Timeline(c.clone())));
            }
            if let Some(conv) = state.conversations.get_mut(&c)
                && let Some(newest) = r.messages.last()
                && conv.latest.as_ref().is_none_or(|l| *l < newest.id.ts)
            {
                conv.latest = Some(newest.id.ts.clone());
                fx.push(Effect::Changed(Changed::Conversations));
            }
            // Catch-up never rings: no `MaybeNotify` here.
            fx.push(Effect::Persist(PersistOp::Messages {
                conversation: c,
                messages: r.messages,
            }));
            if let Some(g) = r.gap {
                fx.push(Effect::Persist(PersistOp::AddGap(g)));
            }
        }
        Input::GapPageLoaded {
            conversation,
            after,
            before,
            page,
        } => {
            if let Some(w) = state.windows.get_mut(&conversation) {
                for m in page.messages.iter().cloned() {
                    w.insert(m);
                }
                fx.push(Effect::Changed(Changed::Timeline(conversation.clone())));
            }
            let oldest = page.messages.iter().map(|m| &m.id.ts).min();
            let closed = sync::covered(
                &conversation,
                after.as_ref(),
                &before,
                oldest,
                page.has_more,
            );
            fx.push(Effect::Persist(PersistOp::Messages {
                conversation,
                messages: page.messages,
            }));
            if let Some(g) = closed {
                fx.push(Effect::Persist(PersistOp::CloseGap(g)));
            }
        }
        Input::Send {
            conversation,
            text,
            thread,
            id,
            now,
            now_ms,
        } => outbox_logic::enqueue(
            state,
            OutboxItem {
                id,
                conversation,
                thread,
                broadcast: false,
                text,
                created_ms: 0,
                attempts: 0,
                state: OutboxState::Queued,
            },
            now,
            now_ms,
            &mut fx,
        ),
        Input::Reply {
            root,
            text,
            broadcast,
            id,
            now,
            now_ms,
        } => outbox_logic::enqueue(
            state,
            OutboxItem {
                id,
                conversation: root.conversation,
                thread: Some(root.ts),
                broadcast,
                text,
                created_ms: 0,
                attempts: 0,
                state: OutboxState::Queued,
            },
            now,
            now_ms,
            &mut fx,
        ),
        Input::SendFinished {
            conversation,
            id,
            result,
        } => {
            let item = state.outbox.iter().find(|i| i.id == id).cloned();
            outbox_logic::finished(state, &id, &result, &mut fx);
            if let Ok(mut m) = result {
                m.client_msg_id = Some(id);
                m.state = DeliveryState::Delivered;
                if let Some(i) = item {
                    if m.thread_root.is_none() {
                        m.thread_root = i.thread;
                    }
                    m.broadcast |= i.broadcast;
                }
                if let Some(root) = m.thread_root.clone() {
                    threads::deliver_reply(state, &m, true, &mut fx);
                    let root = MessageId {
                        conversation: conversation.clone(),
                        ts: root,
                    };
                    crate::thread_follow::replied(state, root, &mut fx);
                }
                if (m.thread_root.is_none() || m.broadcast)
                    && state.windows.get(&conversation).is_none_or(|w| w.is_live())
                {
                    window(state, &conversation).insert(m.clone());
                }
                fx.push(Effect::Persist(PersistOp::Messages {
                    conversation: conversation.clone(),
                    messages: vec![m],
                }));
                fx.push(Effect::Changed(Changed::Timeline(conversation)));
            }
        }
        Input::OutboxLoaded(items) => outbox_logic::loaded(state, items, &mut fx),
        Input::Retry(id) => outbox_logic::retry(state, &id, &mut fx),
        Input::OutboxRetry => outbox_logic::flush(state, &mut fx),
        Input::Discard(id) => outbox_logic::discard(state, &id, &mut fx),
        Input::CountsLoaded(all) => unread::counts_loaded(state, all, &mut fx),
        Input::Focus {
            conversation,
            at_bottom,
            window_active,
        } => unread::focus(state, conversation, at_bottom, window_active, &mut fx),
        Input::MarkUnread(id) => unread::mark_unread(state, id, &mut fx),
        Input::MarkRead { conversation, ts } => unread::mark_read(state, conversation, ts, &mut fx),
        Input::OpenThread(root) => threads::open(state, root, &mut fx),
        Input::SetThreadFollowed { root, follow } => {
            crate::thread_follow::set(state, root, follow, &mut fx)
        }
        Input::CloseThread(root) => threads::close(state, &root, &mut fx),
        Input::RepliesLoaded { root, page } => threads::loaded(state, root, page, &mut fx),
        Input::React { id, emoji, add } => reactions_logic::react(state, id, emoji, add, &mut fx),
        Input::ReactFinished {
            id,
            emoji,
            add,
            result,
        } => reactions_logic::finished(state, id, emoji, add, result, &mut fx),
        Input::CustomEmojiLoaded(all) => reactions_logic::emoji_loaded(state, all, &mut fx),
        Input::UserGroupsLoaded(all) => crate::user_groups::loaded(state, all, &mut fx),
        Input::UserGroupsCached(all) => crate::user_groups::cached(state, all, &mut fx),
        Input::CallFinished { call, result } => {
            dnd_logic::call_finished(state, &call, &result, &mut fx);
            collections::call_finished(state, &call, &result, &mut fx);
            profiles::call_finished(state, &call, &result, &mut fx);
            presence_logic::call_finished(state, &call, &result, &mut fx);
            crate::channel_admin::call_finished(state, &call, &result, &mut fx);
            scheduling::call_finished(state, call.clone(), result.clone(), &mut fx);
            crate::thread_follow::call_finished(state, &call, &result, &mut fx);
            reminders_logic::call_finished(state, call, result, &mut fx);
        }
        Input::Clock { unix, mono } => huddles_logic::clock(state, unix, mono),
        Input::SetRingEnabled(on) => huddles_logic::set_ring_enabled(state, on, &mut fx),
        Input::JoinHuddle(c) => huddles_logic::join(state, c, &mut fx),
        Input::DeclineRing => huddles_logic::decline(state, &mut fx),
        Input::RingTimeout { now } => huddles_logic::timeout(state, now, &mut fx),
        Input::Typed {
            conversation,
            thread,
            now,
        } => crate::typing::typed(state, conversation, thread, now, &mut fx),
        Input::TypingExpired { now } => crate::typing::expired(state, now, &mut fx),
        Input::Loaded(r) => {
            match &r {
                Ok(Loaded::Conversation(c)) => {
                    crate::channel_admin::conversation_loaded(state, c.clone(), &mut fx)
                }
                // Forgotten, so the next change asks again.
                Err((Load::Conversation(id), _)) => {
                    state.conversation_fetching.remove(id);
                }
                _ => {}
            }
            huddles_logic::handle_loaded(state, &r, &mut fx);
            if matches!(
                &r,
                Ok(Loaded::Dnd(_) | Loaded::DndUsers(_) | Loaded::NotifyPrefs(_))
            ) {
                dnd_logic::handle_loaded(state, &r, &mut fx);
            }
            crate::members_logic::handle_loaded(state, &r, &mut fx);
            collections::handle_loaded(state, &r, &mut fx);
            profiles::handle_loaded(state, &r, &mut fx);
            scheduling::handle_loaded(state, r.clone(), &mut fx);
            reminders_logic::handle_loaded(state, r, &mut fx);
        }
        Input::ScheduleSend {
            conversation,
            thread,
            text,
            files,
            post_at,
            local_id,
            now_ms,
        } => scheduling::schedule_send(
            state,
            scheduling::SendArgs {
                conversation,
                thread,
                text,
                files,
                post_at,
                local_id,
                now_ms,
            },
            &mut fx,
        ),
        Input::Reschedule {
            item,
            text,
            post_at,
        } => scheduling::reschedule(state, item, text, post_at, &mut fx),
        Input::DeleteSchedule(id) => scheduling::delete(state, id, &mut fx),
        Input::LocalSchedulesLoaded(all) => scheduling::local_loaded(state, all, &mut fx),
        Input::LocalTick { now } => scheduling::tick(state, now, &mut fx),
        Input::SendLateAnswer { ids, send } => {
            scheduling::late_answer(state, ids, send, &mut fx);
        }
        Input::LocalSent { id, result } => scheduling::local_sent(state, id, result, &mut fx),
        Input::AddReminder { text, when, user } => {
            reminders_logic::add(text, when, user, &mut fx);
        }
        Input::CompleteReminder(id) => reminders_logic::complete(state, id, &mut fx),
        Input::DeleteReminder(id) => reminders_logic::delete(state, id, &mut fx),
        Input::OpenReminders => reminders_logic::open(state, &mut fx),
        Input::AddLocalReminder {
            id,
            text,
            due,
            recurring,
            interval_secs,
            user,
            link,
        } => reminders_logic::add_local(
            state,
            id,
            text,
            due,
            (recurring, interval_secs),
            (user, link),
            &mut fx,
        ),
        Input::SnoozeReminder { id, until } => reminders_logic::snooze(state, &id, until, &mut fx),
        Input::EditReminder {
            id,
            text,
            due,
            repeat,
            now,
        } => reminders_logic::edit(state, &id, text, due, repeat, now, &mut fx),
        Input::LocalRemindersLoaded(all) => reminders_logic::local_loaded(state, all, &mut fx),
        Input::ReminderTick { now } => reminders_logic::tick(state, now, &mut fx),
        Input::OpenPins(c) => collections::open_pins(state, c, &mut fx),
        Input::OpenBookmarks(c) => collections::open_bookmarks(state, c, &mut fx),
        Input::OpenSaved => collections::open_saved(state, &mut fx),
        Input::OpenThreadInbox => collections::open_inbox(state, &mut fx),
        Input::MoreSaved => collections::more_saved(state, &mut fx),
        Input::MoreThreadInbox => collections::more_inbox(state, &mut fx),
        Input::SetPinned { id, pinned } => collections::set_pinned(state, id, pinned, &mut fx),
        Input::SetSaved { id, saved } => collections::set_saved(state, id, saved, &mut fx),
        Input::AddBookmark {
            conversation,
            title,
            link,
        } => collections::add_bookmark(conversation, title, link, &mut fx),
        Input::RemoveBookmark { conversation, id } => {
            collections::remove_bookmark(state, conversation, id, &mut fx);
        }
        Input::SetStatus(s) => profiles::set_status(state, s, &mut fx),
        Input::SetPresence(p) => profiles::set_presence(state, p, &mut fx),
        Input::WatchPresence(users) => presence_logic::watch(state, users, &mut fx),
        Input::OpenDirect(u) => presence_logic::open_direct(state, u, &mut fx),
        Input::Admin(call) => crate::channel_admin::request(state, call, &mut fx),
        Input::OpenProfile(u) => profiles::open(state, u, &mut fx),
        Input::CloseProfile => state.open_profile = None,
        Input::Edit { id, text } => fx.push(Effect::CallEdit { id, text }),
        Input::Delete(id) => fx.push(Effect::CallDelete { id }),
    }
    huddles_logic::dnd_check(state, &mut fx);
    crate::members_logic::scan(state, &mut fx);
    fx
}

pub(crate) fn window<'a>(
    state: &'a mut WorkspaceState,
    c: &ConversationId,
) -> &'a mut MessageWindow {
    let cap = state.window_capacity;
    state
        .windows
        .entry(c.clone())
        .or_insert_with(|| MessageWindow::new(cap))
}

fn bootstrapped(state: &mut WorkspaceState, b: Bootstrap, fx: &mut Vec<Effect>) {
    state.self_user = Some(b.self_user);
    state.name = b.workspace_name;
    for c in &b.conversations {
        state.conversations.insert(c.id.clone(), c.clone());
    }
    for m in &b.members {
        state.members.insert(m.id.clone(), m.clone());
    }
    fx.push(Effect::Persist(PersistOp::Conversations(b.conversations)));
    fx.push(Effect::Persist(PersistOp::Members(b.members)));
    fx.push(Effect::Changed(Changed::Conversations));
    fx.push(Effect::Changed(Changed::Members));
    // Reads the provider offers, when it supports them.
    if state.caps.dnd {
        fx.push(Effect::Load(Load::Dnd));
    }
    if state.caps.notify_prefs {
        fx.push(Effect::Load(Load::NotifyPrefs));
    }
    if state.caps.scheduled_send {
        fx.push(Effect::Load(Load::Scheduled));
    }
    // The workspace's custom emoji, so reactions and text show images.
    if state.caps.custom_emoji {
        fx.push(Effect::FetchCustomEmoji);
    }
    // User groups, for `@` completion and mentions of groups the user is in.
    if state.caps.user_groups {
        fx.push(Effect::FetchUserGroups);
    }
}

fn preview(state: &mut WorkspaceState, mut c: crate::model::Conversation, fx: &mut Vec<Effect>) {
    if state.conversations.contains_key(&c.id) {
        return;
    }
    c.is_member = false;
    c.unread = false;
    c.mentions = 0;
    state.conversations.insert(c.id.clone(), c);
    fx.push(Effect::Changed(Changed::Conversations));
}

fn open(state: &mut WorkspaceState, c: ConversationId, fx: &mut Vec<Effect>) {
    collections::on_open(state, &c, fx);
    profiles::on_open_direct(state, &c, fx);
    let latest = state.conversations.get(&c).and_then(|x| x.latest.clone());
    let w = window(state, &c);
    let empty = w.is_empty();
    let newest = w.newest_ts().cloned().or(latest);
    if empty {
        fx.push(Effect::FetchHistory {
            conversation: c.clone(),
            before: None,
        });
    }
    if let Some(ts) = newest {
        unread::mark_read(state, c, ts, fx);
    }
}

fn provider(state: &mut WorkspaceState, e: ProviderEvent, fx: &mut Vec<Effect>) {
    match e {
        ProviderEvent::Connected { reconnect } => {
            huddles_logic::connected(state, reconnect, fx);
            state.connection = Connection::Online;
            fx.push(Effect::Changed(Changed::Connection));
            outbox_logic::flush(state, fx);
        }
        ProviderEvent::Disconnected => {
            state.connection = Connection::Offline;
            fx.push(Effect::Changed(Changed::Connection));
            crate::typing::clear(state, fx);
        }
        ProviderEvent::SessionExpired => {
            state.session = Session::Expired;
            fx.push(Effect::Changed(Changed::Session));
        }
        ProviderEvent::MessagePosted(mut m) => {
            let c = m.id.conversation.clone();
            if m.client_msg_id.is_none() {
                m.client_msg_id = outbox_logic::echo_of(state, &m);
            }
            if let Some(a) = &m.author {
                crate::typing::posted(state, &c, a, fx);
            }
            if let Some(cid) = &m.client_msg_id {
                outbox_logic::note_settled(state, &m.id);
                outbox_logic::on_echo(state, cid, fx);
            }
            let reply = m.thread_root.is_some();
            threads::deliver_reply(state, &m, false, fx);
            // A plain reply stays in its thread; only a broadcast reaches
            // the channel (and its unread state).
            if !reply || m.broadcast {
                if let Some(w) = state.windows.get_mut(&c).filter(|w| w.is_live()) {
                    w.insert(m.clone());
                    fx.push(Effect::Changed(Changed::Timeline(c.clone())));
                }
                if let Some(conv) = state.conversations.get_mut(&c)
                    && (conv.latest.as_ref().is_none_or(|l| *l < m.id.ts) || !conv.is_open)
                {
                    conv.latest = Some(m.id.ts.clone());
                    // A new message reopens a closed direct conversation.
                    conv.is_open = true;
                    fx.push(Effect::Changed(Changed::Conversations));
                }
                unread::on_live_post(state, &m);
                unread::maybe_mark_read(state, &c, fx);
            }
            fx.push(Effect::MaybeNotify {
                conversation: c.clone(),
                message: m.id.clone(),
            });
            fx.push(Effect::Persist(PersistOp::Messages {
                conversation: c,
                messages: vec![m],
            }));
        }
        ProviderEvent::MessageEdited {
            id,
            text,
            plain,
            edited,
        } => {
            reminders_logic::message_changed(state, &id, Some(&plain), fx);
            message_events::edited(state, id, text, plain, edited, fx)
        }
        ProviderEvent::MessageDeleted(id) => {
            reminders_logic::message_changed(state, &id, None, fx);
            if state
                .windows
                .get_mut(&id.conversation)
                .is_some_and(|w| w.remove(&id))
            {
                fx.push(Effect::Changed(Changed::Timeline(id.conversation.clone())));
            }
            // Open thread panels hold their own copies of replies and root.
            for (root, t) in state.threads.iter_mut() {
                if root.conversation != id.conversation {
                    continue;
                }
                let mut changed = t.replies.remove(&id);
                if t.root.as_ref().is_some_and(|m| m.id == id) {
                    t.root = None;
                    changed = true;
                }
                if changed {
                    fx.push(Effect::Changed(Changed::Thread(root.clone())));
                }
            }
            collections::forget(state, &id);
            fx.push(Effect::Persist(PersistOp::DeleteMessage(id)));
        }
        ProviderEvent::ReactionChanged {
            id,
            emoji,
            user,
            added,
        } => reactions_logic::changed(state, id, emoji, user, added, fx),
        ProviderEvent::CustomEmojiChanged => fx.push(Effect::FetchCustomEmoji),
        ProviderEvent::ReadMarker { conversation, ts } => {
            if let Some(c) = state.conversations.get_mut(&conversation) {
                c.last_read = Some(ts.clone());
                unread::recompute(state, &conversation);
                fx.push(Effect::Changed(Changed::Conversations));
            }
            fx.push(Effect::Persist(PersistOp::ReadMarker { conversation, ts }));
        }
        ProviderEvent::ConversationMetaChanged(meta) => {
            crate::channel_admin::apply_meta(state, meta, fx);
        }
        ProviderEvent::ConversationMemberChanged {
            conversation,
            user,
            joined,
        } => crate::channel_admin::member_changed(state, conversation, user, joined, fx),
        ProviderEvent::MemberUpdated(mut m) => {
            profiles::member_updated(state, &mut m, fx);
            // An update that carries no picture (a bot announced from a message)
            // keeps the one already known.
            if let Some(old) = state.members.get(&m.id)
                && m.avatar.is_none()
            {
                m.avatar = old.avatar.clone();
            }
            state.members.insert(m.id.clone(), m.clone());
            fx.push(Effect::Persist(PersistOp::Members(vec![m])));
            fx.push(Effect::Changed(Changed::Members));
        }
        ProviderEvent::Typing {
            conversation,
            user,
            thread,
        } => crate::typing::event(state, conversation, user, thread, fx),
        ProviderEvent::Presence { user, active } => presence_logic::event(state, user, active, fx),
        ProviderEvent::ScheduledChanged => scheduling::refetch(fx),
        ProviderEvent::RemindersChanged => reminders_logic::changed(fx),
        ProviderEvent::DndChanged { user, status } => {
            let changed = state.dnd.get(&user) != Some(&status);
            state.dnd.insert(user, status);
            if changed {
                fx.push(Effect::Changed(Changed::Dnd));
            }
        }
        // A true snapshot, like `Loaded(NotifyPrefs)`: replaces the map, keeping
        // conversations with a write in flight.
        ProviderEvent::NotifyPrefsChanged(prefs) => {
            dnd_logic::handle_loaded(state, &Ok(Loaded::NotifyPrefs(prefs)), fx)
        }
        ProviderEvent::MessageUpdated(m) => {
            reminders_logic::message_changed(state, &m.id, Some(&m.plain), fx);
            message_events::updated(state, m, fx)
        }
        ProviderEvent::ThreadUpdated {
            root,
            reply_count,
            latest_reply,
            reply_users,
        } => {
            message_events::thread_updated(state, root, reply_count, latest_reply, reply_users, fx)
        }
        ProviderEvent::FileDeleted(f) => message_events::file_deleted(state, f, fx),
        ProviderEvent::ThreadFollowChanged { root, followed } => {
            crate::thread_follow::provider_changed(state, root, followed, fx)
        }
        ProviderEvent::ThreadReadMarker { .. } | ProviderEvent::ThreadInboxChanged => {
            collections::inbox_event(state, fx)
        }
        ProviderEvent::ProfileChanged(u) => profiles::changed(state, u, fx),
        ProviderEvent::HuddleUpdated(h) => huddles_logic::updated(state, h, fx),
        ProviderEvent::HuddleEnded {
            conversation, id, ..
        } => huddles_logic::ended(state, &conversation, &id, fx),
        ProviderEvent::HuddleRing {
            conversation,
            huddle,
            from,
        } => huddles_logic::ring(state, conversation, huddle, from, fx),
        ProviderEvent::HuddleRingCancelled { conversation } => {
            huddles_logic::cancelled(state, &conversation, fx)
        }
        ProviderEvent::InHuddle { user, active } => {
            huddles_logic::in_huddle(state, user, active, fx)
        }
        ProviderEvent::PinChanged { id, pinned } => collections::pin_event(state, id, pinned, fx),
        ProviderEvent::BookmarksChanged(c) => collections::bookmarks_event(state, c, fx),
        ProviderEvent::SavedChanged => collections::saved_event(state, fx),
        ProviderEvent::WorkspaceRenamed { name } => {
            if state.name != name {
                state.name = name;
                fx.push(Effect::Changed(Changed::Conversations));
            }
        }
    }
}
