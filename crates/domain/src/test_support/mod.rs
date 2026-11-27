//! Test support for core crates: shared builders and an in-memory
//! [`ChatProvider`]. Compiled only with the `test-support` feature.

pub mod fixture;

mod catalog;
pub use catalog::fake_catalog;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{BoxFuture, ready};
use futures::stream::BoxStream;
use settings::{ProviderId, WorkspaceKey};

use crate::{
    AccountInfo, Bookmark, BookmarkId, CallSurface, ContentEncoder, Huddle, HuddleId, InboxThread,
    NativeCallJoin, Pin, PlainEncoder, SavedItem, SecretText,
};
use crate::{
    Bootstrap, Capabilities, ChatProvider, ClientMsgId, ContentParser, Conversation,
    ConversationCounts, ConversationId, ConversationKind, CustomEmoji, EmojiRef, ExtCodec,
    HistoryPage, Member, Message, MessageId, MessageTs, NoExtCodec, PlainParser, ProviderError,
    ProviderEvent, ProviderFactory, RemoteUrl, Upload, UserId,
};
use crate::{
    ConversationPrefs, DndStatus, FileHit, MessageHit, Page, PresenceSetting, Profile, Reminder,
    ReminderId, ScheduleId, ScheduledMessage, SearchQuery, Status,
};

// ---- builders ----------------------------------------------------------

pub fn cid(s: &str) -> ConversationId {
    ConversationId(Arc::from(s))
}

pub fn uid(s: &str) -> UserId {
    UserId(Arc::from(s))
}

pub fn ts(s: &str) -> MessageTs {
    MessageTs(Arc::from(s))
}

pub fn mid(c: &str, t: &str) -> MessageId {
    MessageId {
        conversation: cid(c),
        ts: ts(t),
    }
}

pub fn cmid(s: &str) -> ClientMsgId {
    ClientMsgId(Arc::from(s))
}

/// A delivered message by user `u2` in conversation `c` at `t`.
pub fn msg(c: &str, t: &str, text: &str) -> Message {
    Message::new(mid(c, t), Some(uid("u2")), Arc::from(text))
}

/// A joined public channel with no unread state.
pub fn conv(id: &str, name: &str) -> Conversation {
    Conversation {
        id: cid(id),
        kind: ConversationKind::Channel,
        name: Arc::from(name),
        peer: None,
        is_member: true,
        is_archived: false,
        is_open: true,
        is_muted: false,
        last_read: None,
        latest: None,
        unread: false,
        mentions: 0,
        participants: Arc::from([]),
        member_ids: Arc::from([]),
        topic: Default::default(),
        member_count: None,
        purpose: Default::default(),
        ext: Default::default(),
    }
}

/// A human, active member whose real name equals the display name.
pub fn member(id: &str, name: &str) -> Member {
    Member {
        id: uid(id),
        display_name: Arc::from(name),
        real_name: Arc::from(name),
        handle: Arc::from(""),
        is_bot: false,
        deleted: false,
        status: None,
        title: None,
        tz: None,
        avatar: None,
        ext: Default::default(),
    }
}

// ---- FakeProvider ------------------------------------------------------

/// Provider operations that can have errors injected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    Bootstrap,
    History,
    Send,
    Edit,
    Delete,
    MarkRead,
    HistoryBetween,
    Replies,
    Reply,
    React,
    MarkThreadRead,
    Counts,
    CustomEmoji,
    Fetch,
    Download,
    Upload,
    SearchMessages,
    SearchFiles,
    SearchPeople,
    SearchConversations,
    Schedule,
    Scheduled,
    DeleteScheduled,
    Reminders,
    AddReminder,
    CompleteReminder,
    DeleteReminder,
    Profile,
    SetStatus,
    SetPresence,
    WatchPresence,
    OpenDirect,
    Dnd,
    DndUsers,
    Snooze,
    EndSnooze,
    NotifyPrefs,
    SetNotifyPrefs,
    Huddles,
    DeclineHuddle,
    CallSurface,
    NativeCallJoin,
    LeaveHuddle,
    Message,
    Conversation,
    Members,
    Pins,
    SetPinned,
    Bookmarks,
    AddBookmark,
    RemoveBookmark,
    Saved,
    SetSaved,
    ThreadInbox,
    SetThreadFollowed,
    /// Every channel admin write (`create_channel` ... `invite`).
    Admin,
    BrowseChannels,
}

/// An outgoing call the fake accepted (failed calls are not recorded).
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    ReconnectNow,
    /// `ChatProvider::typing`.
    Typing {
        conversation: ConversationId,
        thread: Option<MessageTs>,
    },
    Send {
        conversation: ConversationId,
        text: String,
        thread: Option<MessageTs>,
        id: ClientMsgId,
    },
    Edit {
        id: MessageId,
        text: String,
    },
    Delete(MessageId),
    MarkRead {
        conversation: ConversationId,
        ts: MessageTs,
    },
    Reply {
        conversation: ConversationId,
        root: MessageTs,
        text: String,
        broadcast: bool,
        id: ClientMsgId,
    },
    React {
        id: MessageId,
        emoji: EmojiRef,
        add: bool,
    },
    MarkThreadRead {
        conversation: ConversationId,
        root: MessageTs,
        ts: MessageTs,
    },
    Fetch(RemoteUrl),
    HistoryBetween {
        conversation: ConversationId,
        after: MessageTs,
        before: Option<MessageTs>,
    },
    Download {
        url: RemoteUrl,
        dest: PathBuf,
    },
    Upload {
        conversation: ConversationId,
        thread: Option<MessageTs>,
        files: Vec<Upload>,
        text: Option<String>,
        id: ClientMsgId,
    },
    Schedule {
        conversation: ConversationId,
        thread: Option<MessageTs>,
        text: String,
        post_at: i64,
    },
    DeleteScheduled(ScheduleId),
    AddReminder {
        text: String,
        when: String,
        user: Option<UserId>,
    },
    CompleteReminder(ReminderId),
    DeleteReminder(ReminderId),
    SetStatus(Option<Status>),
    SetPresence(PresenceSetting),
    WatchPresence(Vec<UserId>),
    OpenDirect(UserId),
    /// A channel admin write (`create_channel` ... `invite`).
    Admin(crate::AdminCall),
    /// A channel browser page for this query.
    BrowseChannels(String),
    SetConversationStarred {
        conversation: ConversationId,
        starred: bool,
    },
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
    CallSurface(ConversationId),
    NativeCallJoin(ConversationId),
    LeaveHuddle(ConversationId),
    SetPinned {
        id: MessageId,
        pinned: bool,
    },
    AddBookmark {
        conversation: ConversationId,
        title: String,
        link: String,
    },
    RemoveBookmark {
        conversation: ConversationId,
        id: BookmarkId,
    },
    SetSaved {
        id: MessageId,
        saved: bool,
    },
    SetThreadFollowed {
        root: MessageId,
        follow: bool,
    },
    Members(Vec<UserId>),
}

/// Scripted history per conversation. A conversation can be scripted lazily
/// ([`FakeProvider::add_lazy_messages`]): its messages are generated the first
/// time anything reads or changes them, like a server that only builds a page
/// when one is asked for. A large fixture then costs memory only for the
/// conversations a scenario actually opens.
#[derive(Default)]
struct History(HashMap<ConversationId, Slot>);

type Generator = Box<dyn FnOnce() -> Vec<Message> + Send>;

#[derive(Default)]
struct Slot {
    generate: std::cell::RefCell<Option<Generator>>,
    items: std::cell::OnceCell<Vec<Message>>,
}

impl Slot {
    fn force(&self) -> &Vec<Message> {
        self.items.get_or_init(|| {
            let generate = self.generate.borrow_mut().take();
            generate.map(|g| g()).unwrap_or_default()
        })
    }
}

impl History {
    fn get(&self, c: &ConversationId) -> Option<&Vec<Message>> {
        self.0.get(c).map(Slot::force)
    }

    fn get_mut(&mut self, c: &ConversationId) -> Option<&mut Vec<Message>> {
        let slot = self.0.get_mut(c)?;
        slot.force();
        slot.items.get_mut()
    }

    fn entry_or_default(&mut self, c: &ConversationId) -> &mut Vec<Message> {
        let slot = self.0.entry(c.clone()).or_default();
        slot.force();
        slot.items.get_mut().expect("forced above")
    }
}

type SurfaceFactory = Arc<dyn Fn() -> Result<CallSurface, ProviderError> + Send + Sync>;

#[derive(Default)]
struct Inner {
    /// Channel members for `conversation_members` and `invite`.
    channel_members: HashMap<ConversationId, Vec<UserId>>,
    /// Bootstraps that never answer (see [`FakeProvider::hang_bootstrap`]).
    hang_bootstrap: bool,
    self_user: Option<UserId>,
    name: Arc<str>,
    conversations: Vec<Conversation>,
    members: Vec<Member>,
    /// Known to `members()` lookups only, not part of the bootstrap.
    lookup_members: Vec<Member>,
    /// Per conversation, ascending by timestamp.
    messages: History,
    calls: Vec<Call>,
    /// Every provider request, accepted or not (perf counters).
    requests: u64,
    errors: HashMap<Op, VecDeque<ProviderError>>,
    /// Ops whose answers wait for [`FakeProvider::release`].
    held: std::collections::HashSet<Op>,
    waiting: Vec<futures::channel::oneshot::Sender<()>>,
    subscribers: Vec<mpsc::UnboundedSender<ProviderEvent>>,
    /// Events pushed before anyone subscribed; the first subscriber gets them.
    backlog: Vec<ProviderEvent>,
    next_ts: u64,
    counts: Option<Vec<ConversationCounts>>,
    emoji: Vec<CustomEmoji>,
    user_groups: Vec<crate::UserGroup>,
    files: HashMap<RemoteUrl, Vec<u8>>,
    message_hits: Vec<MessageHit>,
    file_hits: Vec<FileHit>,
    scheduled: Vec<ScheduledMessage>,
    reminders: Vec<Reminder>,
    profiles: HashMap<UserId, Profile>,
    dnd: DndStatus,
    dnd_users: HashMap<UserId, DndStatus>,
    notify_prefs: HashMap<ConversationId, ConversationPrefs>,
    huddles: Vec<Huddle>,
    surface: Option<SurfaceFactory>,
    pins: HashMap<ConversationId, Vec<Pin>>,
    bookmarks: HashMap<ConversationId, Vec<Bookmark>>,
    saved: Vec<SavedItem>,
    inbox: Vec<InboxThread>,
    next_id: u64,
    web_origin: Option<String>,
}

/// The fake's channel name rule: lowercase, spaces become hyphens, only
/// letters, digits, `-` and `_`.
pub fn fake_channel_name(typed: &str) -> crate::ChannelName {
    let name: String = typed
        .trim()
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .flat_map(char::to_lowercase)
        .collect();
    let problem = if name.is_empty() {
        Some(crate::NameProblem::Empty)
    } else if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        Some(crate::NameProblem::Characters)
    } else {
        None
    };
    crate::ChannelName { name, problem }
}

/// In-memory [`ChatProvider`]: scripted data, recorded outgoing calls, a
/// pushable event stream and injectable errors. Provider id is `"fake"`.
pub struct FakeProvider {
    key: WorkspaceKey,
    caps: Capabilities,
    inner: Mutex<Inner>,
}

impl FakeProvider {
    pub fn new(workspace_id: &str, self_user: UserId) -> Self {
        Self {
            key: WorkspaceKey::new("fake", workspace_id),
            caps: Capabilities::default(),
            inner: Mutex::new(Inner {
                self_user: Some(self_user),
                name: Arc::from("Fake Workspace"),
                // Sent messages get timestamps after this unless scripted later.
                next_ts: 1_000_000_000_000_000,
                ..Default::default()
            }),
        }
    }

    pub fn with_capabilities(mut self, caps: Capabilities) -> Self {
        self.caps = caps;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("fake provider lock")
    }

    pub fn set_workspace_name(&self, name: &str) {
        self.lock().name = Arc::from(name);
    }

    pub fn add_conversation(&self, c: Conversation) {
        self.lock().conversations.push(c);
    }

    /// The members `conversation_members` answers for `c`.
    pub fn set_channel_members(&self, c: &ConversationId, users: &[UserId]) {
        self.lock()
            .channel_members
            .insert(c.clone(), users.to_vec());
    }

    pub fn add_member(&self, m: Member) {
        self.lock().members.push(m);
    }

    /// A member the bootstrap does not carry; only `members()` finds it.
    pub fn add_lookup_member(&self, m: Member) {
        self.lock().lookup_members.push(m);
    }

    /// Script one history message (kept sorted by timestamp).
    pub fn add_message(&self, m: Message) {
        let mut g = self.lock();
        let v = g.messages.entry_or_default(&m.id.conversation);
        v.push(m);
        v.sort_by(|a, b| a.id.ts.cmp(&b.id.ts));
    }

    /// Script many history messages of one conversation at once (sorted once).
    pub fn add_messages(&self, c: &ConversationId, ms: Vec<Message>) {
        let mut g = self.lock();
        let v = g.messages.entry_or_default(c);
        v.extend(ms);
        v.sort_by(|a, b| a.id.ts.cmp(&b.id.ts));
    }

    /// Script the history of one conversation, built by `generate` the first
    /// time it is read or changed (so an unopened conversation holds no
    /// messages). `generate` must return them ascending by timestamp.
    pub fn add_lazy_messages(
        &self,
        c: &ConversationId,
        generate: impl FnOnce() -> Vec<Message> + Send + 'static,
    ) {
        let mut g = self.lock();
        let slot = g.messages.0.entry(c.clone()).or_default();
        debug_assert!(slot.items.get().is_none(), "history already forced");
        *slot.generate.borrow_mut() = Some(Box::new(generate));
    }

    /// Outgoing calls so far, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    /// How many provider requests (bootstrap, history, sends, ...) were made,
    /// failed ones included. Unlike [`calls`](Self::calls) this counts reads too.
    pub fn request_count(&self) -> u64 {
        self.lock().requests
    }

    /// Make the next call of `op` fail with `err` (queued FIFO, one per call).
    pub fn fail_next(&self, op: Op, err: ProviderError) {
        self.lock().errors.entry(op).or_default().push_back(err);
    }

    /// Hold the answers of `op` (a slow network) until [`release`](Self::release).
    pub fn hold(&self, op: Op) {
        self.lock().held.insert(op);
    }

    /// Lets every held answer through and stops holding.
    pub fn release(&self) {
        let mut g = self.lock();
        g.held.clear();
        for tx in g.waiting.drain(..) {
            tx.send(()).ok();
        }
    }

    /// Make every bootstrap call hang forever (a stalled connection).
    pub fn hang_bootstrap(&self) {
        self.lock().hang_bootstrap = true;
    }

    /// Deliver `ev` to every live event stream; if none was ever created,
    /// buffer it for the first one.
    pub fn push_event(&self, ev: ProviderEvent) {
        let mut g = self.lock();
        g.subscribers.retain(|s| !s.is_closed());
        if g.subscribers.is_empty() {
            g.backlog.push(ev);
            return;
        }
        for s in &g.subscribers {
            let _ = s.unbounded_send(ev.clone());
        }
    }

    /// Alias of [`push_event`](Self::push_event).
    pub fn push(&self, ev: ProviderEvent) {
        self.push_event(ev);
    }

    /// Script the answer of `counts()` (default: derived from conversations).
    pub fn set_counts(&self, counts: Vec<ConversationCounts>) {
        self.lock().counts = Some(counts);
    }

    pub fn add_custom_emoji(&self, e: CustomEmoji) {
        self.lock().emoji.push(e);
    }

    /// Adds a user group to `user_groups()`'s answer.
    pub fn add_user_group(&self, g: crate::UserGroup) {
        self.lock().user_groups.push(g);
    }

    /// Serve `bytes` for `url` from `fetch` and `download`.
    pub fn add_file(&self, url: RemoteUrl, bytes: Vec<u8>) {
        self.lock().files.insert(url, bytes);
    }

    /// Script the answer of `search_messages`.
    pub fn set_search_hits(&self, hits: Vec<MessageHit>) {
        self.lock().message_hits = hits;
    }

    pub fn set_profile(&self, p: Profile) {
        self.lock().profiles.insert(p.user.clone(), p);
    }

    /// The self user's do-not-disturb state.
    pub fn set_dnd(&self, d: DndStatus) {
        self.lock().dnd = d;
    }

    pub fn set_huddles(&self, all: Vec<Huddle>) {
        self.lock().huddles = all;
    }

    /// Scripts `call_surface`: `f` runs on every call so each gets a fresh
    /// surface (secrets are not `Clone`).
    pub fn set_call_surface(
        &self,
        f: impl Fn() -> Result<CallSurface, ProviderError> + Send + Sync + 'static,
    ) {
        self.lock().surface = Some(Arc::new(f));
    }

    pub fn set_pins(&self, c: &ConversationId, all: Vec<Pin>) {
        self.lock().pins.insert(c.clone(), all);
    }

    pub fn set_saved_items(&self, all: Vec<SavedItem>) {
        self.lock().saved = all;
    }

    pub fn set_thread_inbox(&self, all: Vec<InboxThread>) {
        self.lock().inbox = all;
    }

    pub fn set_web_origin(&self, origin: impl Into<String>) {
        self.lock().web_origin = Some(origin.into());
    }

    /// Runs one M3 operation: `Unsupported` when `allowed` is false, else the
    /// injected error for `op`, else `f` (which may record a [`Call`]).
    fn m3<T: Send + 'static>(
        &self,
        allowed: bool,
        op: Op,
        f: impl FnOnce(&mut Inner) -> T,
    ) -> BoxFuture<'_, Result<T, ProviderError>> {
        let mut g = self.lock();
        let r = if !allowed {
            Err(ProviderError::Unsupported)
        } else {
            match Self::take_error(&mut g, op) {
                Some(e) => Err(e),
                None => Ok(f(&mut g)),
            }
        };
        if g.held.contains(&op) {
            let (tx, rx) = futures::channel::oneshot::channel();
            g.waiting.push(tx);
            return Box::pin(async move {
                rx.await.ok();
                r
            });
        }
        Box::pin(ready(r))
    }

    /// `r` at once, or once [`release`](Self::release)d while `op` is held.
    fn answer<T: Send + 'static>(
        g: &mut Inner,
        op: Op,
        r: Result<T, ProviderError>,
    ) -> BoxFuture<'static, Result<T, ProviderError>> {
        if g.held.contains(&op) {
            let (tx, rx) = futures::channel::oneshot::channel();
            g.waiting.push(tx);
            return Box::pin(async move {
                rx.await.ok();
                r
            });
        }
        Box::pin(ready(r))
    }

    /// Append a new message by the self user with a fresh increasing timestamp.
    fn post(
        g: &mut Inner,
        c: &ConversationId,
        text: &str,
        thread: Option<MessageTs>,
        id: ClientMsgId,
    ) -> Message {
        let latest = g
            .messages
            .get(c)
            .and_then(|v| v.last())
            .map(|m| m.id.ts.clone());
        let t = loop {
            g.next_ts += 1;
            let t = MessageTs(Arc::from(format!(
                "{}.{:06}",
                g.next_ts / 1_000_000,
                g.next_ts % 1_000_000
            )));
            if latest.as_ref().is_none_or(|l| t > *l) {
                break t;
            }
        };
        let mut m = Message::new(
            MessageId {
                conversation: c.clone(),
                ts: t,
            },
            g.self_user.clone(),
            Arc::from(text),
        );
        m.thread_root = thread;
        m.client_msg_id = Some(id);
        g.messages.entry_or_default(c).push(m.clone());
        m
    }

    fn take_error(g: &mut Inner, op: Op) -> Option<ProviderError> {
        g.requests += 1;
        g.errors.get_mut(&op)?.pop_front()
    }
}

impl ChatProvider for FakeProvider {
    fn workspace(&self) -> &WorkspaceKey {
        &self.key
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn bootstrap(&self) -> BoxFuture<'_, Result<Bootstrap, ProviderError>> {
        let mut g = self.lock();
        if g.hang_bootstrap {
            g.requests += 1;
            return Box::pin(std::future::pending());
        }
        let r = match Self::take_error(&mut g, Op::Bootstrap) {
            Some(e) => Err(e),
            None => Ok(Bootstrap {
                self_user: g.self_user.clone().expect("self user"),
                workspace_name: g.name.clone(),
                conversations: g.conversations.clone(),
                members: g.members.clone(),
            }),
        };
        if g.held.contains(&Op::Bootstrap) {
            let (tx, rx) = futures::channel::oneshot::channel();
            g.waiting.push(tx);
            return Box::pin(async move {
                rx.await.ok();
                r
            });
        }
        Box::pin(ready(r))
    }

    fn history(
        &self,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: u32,
    ) -> BoxFuture<'_, Result<HistoryPage, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::History) {
            Some(e) => Err(e),
            None => {
                let all = g.messages.get(c).map(Vec::as_slice).unwrap_or(&[]);
                let older: Vec<&Message> = all
                    .iter()
                    .filter(|m| before.as_ref().is_none_or(|b| m.id.ts < *b))
                    .collect();
                let keep = (limit as usize).min(older.len());
                let start = older.len() - keep;
                Ok(HistoryPage {
                    messages: older[start..].iter().map(|m| (*m).clone()).collect(),
                    has_more: start > 0,
                })
            }
        };
        if g.held.contains(&Op::History) {
            let (tx, rx) = futures::channel::oneshot::channel();
            g.waiting.push(tx);
            return Box::pin(async move {
                rx.await.ok();
                r
            });
        }
        Box::pin(ready(r))
    }

    fn send(
        &self,
        c: &ConversationId,
        text: &str,
        thread: Option<MessageTs>,
        id: ClientMsgId,
    ) -> BoxFuture<'_, Result<Message, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::Send) {
            Some(e) => Err(e),
            None => {
                let m = Self::post(&mut g, c, text, thread.clone(), id.clone());
                g.calls.push(Call::Send {
                    conversation: c.clone(),
                    text: text.to_owned(),
                    thread,
                    id,
                });
                Ok(m)
            }
        };
        Self::answer(&mut g, Op::Send, r)
    }

    fn edit(&self, id: &MessageId, text: &str) -> BoxFuture<'_, Result<(), ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::Edit) {
            Some(e) => Err(e),
            None => {
                if let Some(m) = g
                    .messages
                    .get_mut(&id.conversation)
                    .and_then(|v| v.iter_mut().find(|m| m.id == *id))
                {
                    m.text = Arc::from(text);
                }
                g.calls.push(Call::Edit {
                    id: id.clone(),
                    text: text.to_owned(),
                });
                Ok(())
            }
        };
        Box::pin(ready(r))
    }

    fn delete(&self, id: &MessageId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::Delete) {
            Some(e) => Err(e),
            None => {
                if let Some(v) = g.messages.get_mut(&id.conversation) {
                    v.retain(|m| m.id != *id);
                }
                g.calls.push(Call::Delete(id.clone()));
                Ok(())
            }
        };
        Box::pin(ready(r))
    }

    fn mark_read(
        &self,
        c: &ConversationId,
        ts: &MessageTs,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::MarkRead) {
            Some(e) => Err(e),
            None => {
                g.calls.push(Call::MarkRead {
                    conversation: c.clone(),
                    ts: ts.clone(),
                });
                // Like a real server: later counts say the conversation is
                // read, so a late counts answer cannot resurrect its unread.
                if let Some(conv) = g.conversations.iter_mut().find(|k| k.id == *c) {
                    conv.last_read = Some(ts.clone());
                    if conv.latest.as_ref().is_none_or(|l| l <= ts) {
                        conv.unread = false;
                        conv.mentions = 0;
                    }
                }
                Ok(())
            }
        };
        Box::pin(ready(r))
    }

    fn history_between(
        &self,
        c: &ConversationId,
        after: MessageTs,
        before: Option<MessageTs>,
        limit: u32,
    ) -> BoxFuture<'_, Result<HistoryPage, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::HistoryBetween) {
            Some(e) => Err(e),
            None => {
                g.calls.push(Call::HistoryBetween {
                    conversation: c.clone(),
                    after: after.clone(),
                    before: before.clone(),
                });
                let mut v: Vec<Message> = g
                    .messages
                    .get(c)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|m| m.id.ts > after && before.as_ref().is_none_or(|b| m.id.ts < *b))
                    .cloned()
                    .collect();
                v.reverse();
                let has_more = v.len() > limit as usize;
                v.truncate(limit as usize);
                Ok(HistoryPage {
                    messages: v,
                    has_more,
                })
            }
        };
        Box::pin(ready(r))
    }

    fn replies(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        after: Option<MessageTs>,
        limit: u32,
    ) -> BoxFuture<'_, Result<HistoryPage, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::Replies) {
            Some(e) => Err(e),
            None => {
                let mut v: Vec<Message> = g
                    .messages
                    .get(c)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|m| {
                        let is_root = m.id.ts == *root;
                        let is_reply = m.thread_root.as_ref() == Some(root);
                        match &after {
                            None => is_root || is_reply,
                            Some(a) => is_reply && m.id.ts > *a,
                        }
                    })
                    .cloned()
                    .collect();
                let has_more = v.len() > limit as usize;
                v.truncate(limit as usize);
                Ok(HistoryPage {
                    messages: v,
                    has_more,
                })
            }
        };
        Box::pin(ready(r))
    }

    fn reply(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        text: &str,
        broadcast: bool,
        id: ClientMsgId,
    ) -> BoxFuture<'_, Result<Message, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::Reply) {
            Some(e) => Err(e),
            None => {
                let m = Self::post(&mut g, c, text, Some(root.clone()), id.clone());
                if let Some(stored) = g.messages.get_mut(c).and_then(|v| v.last_mut()) {
                    stored.broadcast = broadcast;
                }
                let mut m = m;
                m.broadcast = broadcast;
                g.calls.push(Call::Reply {
                    conversation: c.clone(),
                    root: root.clone(),
                    text: text.to_owned(),
                    broadcast,
                    id,
                });
                Ok(m)
            }
        };
        Self::answer(&mut g, Op::Reply, r)
    }

    fn react(
        &self,
        id: &MessageId,
        emoji: &EmojiRef,
        add: bool,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::React) {
            Some(e) => Err(e),
            None => {
                g.calls.push(Call::React {
                    id: id.clone(),
                    emoji: emoji.clone(),
                    add,
                });
                Ok(())
            }
        };
        Box::pin(ready(r))
    }

    fn mark_thread_read(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        ts: &MessageTs,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::MarkThreadRead) {
            Some(e) => Err(e),
            None => {
                g.calls.push(Call::MarkThreadRead {
                    conversation: c.clone(),
                    root: root.clone(),
                    ts: ts.clone(),
                });
                Ok(())
            }
        };
        Box::pin(ready(r))
    }

    fn counts(&self) -> BoxFuture<'_, Result<Vec<ConversationCounts>, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::Counts) {
            Some(e) => Err(e),
            None => Ok(g.counts.clone().unwrap_or_else(|| {
                g.conversations
                    .iter()
                    .map(|c| ConversationCounts {
                        conversation: c.id.clone(),
                        latest: c.latest.clone(),
                        last_read: c.last_read.clone(),
                        unread: c.unread,
                        mentions: c.mentions,
                    })
                    .collect()
            })),
        };
        Box::pin(ready(r))
    }

    fn custom_emoji(&self) -> BoxFuture<'_, Result<Vec<CustomEmoji>, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::CustomEmoji) {
            Some(e) => Err(e),
            None => Ok(g.emoji.clone()),
        };
        Box::pin(ready(r))
    }

    fn user_groups(&self) -> BoxFuture<'_, Result<Vec<crate::UserGroup>, ProviderError>> {
        Box::pin(ready(Ok(self.lock().user_groups.clone())))
    }

    fn fetch(
        &self,
        url: &RemoteUrl,
        max_bytes: u64,
    ) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::Fetch) {
            Some(e) => Err(e),
            None => match g.files.get(url).cloned() {
                None => Err(ProviderError::Other("not found".into())),
                Some(b) if b.len() as u64 > max_bytes => Err(ProviderError::TooLarge),
                Some(b) => {
                    g.calls.push(Call::Fetch(url.clone()));
                    Ok(b)
                }
            },
        };
        Box::pin(ready(r))
    }

    fn download(&self, url: &RemoteUrl, dest: &Path) -> BoxFuture<'_, Result<u64, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::Download) {
            Some(e) => Err(e),
            None => match g.files.get(url).cloned() {
                None => Err(ProviderError::Other("not found".into())),
                Some(b) => match std::fs::write(dest, &b) {
                    Err(e) => Err(ProviderError::Other(e.to_string())),
                    Ok(()) => {
                        g.calls.push(Call::Download {
                            url: url.clone(),
                            dest: dest.to_path_buf(),
                        });
                        Ok(b.len() as u64)
                    }
                },
            },
        };
        Box::pin(ready(r))
    }

    fn upload(
        &self,
        c: &ConversationId,
        thread: Option<MessageTs>,
        files: Vec<Upload>,
        text: Option<&str>,
        id: ClientMsgId,
    ) -> BoxFuture<'_, Result<Message, ProviderError>> {
        let mut g = self.lock();
        let r = match Self::take_error(&mut g, Op::Upload) {
            Some(e) => Err(e),
            None => {
                let m = Self::post(&mut g, c, text.unwrap_or(""), thread.clone(), id.clone());
                g.calls.push(Call::Upload {
                    conversation: c.clone(),
                    thread,
                    files,
                    text: text.map(str::to_owned),
                    id,
                });
                Ok(m)
            }
        };
        Box::pin(ready(r))
    }

    fn search_messages(
        &self,
        _q: &SearchQuery,
        _cursor: Option<Arc<str>>,
        limit: u32,
    ) -> BoxFuture<'_, Result<Page<MessageHit>, ProviderError>> {
        self.m3(self.caps.search, Op::SearchMessages, |g| Page {
            items: g
                .message_hits
                .iter()
                .take(limit as usize)
                .cloned()
                .collect(),
            next: None,
        })
    }

    fn search_files(
        &self,
        _q: &SearchQuery,
        _cursor: Option<Arc<str>>,
        limit: u32,
    ) -> BoxFuture<'_, Result<Page<FileHit>, ProviderError>> {
        self.m3(self.caps.search, Op::SearchFiles, |g| Page {
            items: g.file_hits.iter().take(limit as usize).cloned().collect(),
            next: None,
        })
    }

    fn search_people(
        &self,
        text: &str,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<Member>, ProviderError>> {
        let needle = text.to_lowercase();
        self.m3(self.caps.search, Op::SearchPeople, move |g| {
            g.members
                .iter()
                .filter(|m| {
                    m.display_name.to_lowercase().contains(&needle)
                        || m.real_name.to_lowercase().contains(&needle)
                })
                .take(limit as usize)
                .cloned()
                .collect()
        })
    }

    fn search_conversations(
        &self,
        text: &str,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<Conversation>, ProviderError>> {
        let needle = text.to_lowercase();
        self.m3(self.caps.search, Op::SearchConversations, move |g| {
            g.conversations
                .iter()
                .filter(|c| c.name.to_lowercase().contains(&needle))
                .take(limit as usize)
                .cloned()
                .collect()
        })
    }

    fn schedule(
        &self,
        c: &ConversationId,
        thread: Option<MessageTs>,
        text: &str,
        post_at: i64,
    ) -> BoxFuture<'_, Result<ScheduledMessage, ProviderError>> {
        let (c, text) = (c.clone(), text.to_owned());
        self.m3(self.caps.scheduled_send, Op::Schedule, move |g| {
            g.next_id += 1;
            let s = ScheduledMessage {
                id: ScheduleId(Arc::from(format!("sched-{}", g.next_id))),
                conversation: c.clone(),
                thread: thread.clone(),
                text: Arc::from(text.as_str()),
                post_at,
            };
            g.scheduled.push(s.clone());
            g.calls.push(Call::Schedule {
                conversation: c,
                thread,
                text,
                post_at,
            });
            s
        })
    }

    fn scheduled(&self) -> BoxFuture<'_, Result<Vec<ScheduledMessage>, ProviderError>> {
        self.m3(self.caps.scheduled_send, Op::Scheduled, |g| {
            g.scheduled.clone()
        })
    }

    fn delete_scheduled(&self, s: &ScheduledMessage) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = s.id.clone();
        self.m3(self.caps.scheduled_send, Op::DeleteScheduled, move |g| {
            g.scheduled.retain(|x| x.id != id);
            g.calls.push(Call::DeleteScheduled(id));
        })
    }

    fn reminders(&self) -> BoxFuture<'_, Result<Vec<Reminder>, ProviderError>> {
        self.m3(self.caps.reminders, Op::Reminders, |g| g.reminders.clone())
    }

    fn add_reminder(
        &self,
        text: &str,
        when: &str,
        user: Option<&UserId>,
    ) -> BoxFuture<'_, Result<Reminder, ProviderError>> {
        let (text, when, user) = (text.to_owned(), when.to_owned(), user.cloned());
        self.m3(self.caps.reminders, Op::AddReminder, move |g| {
            g.next_id += 1;
            let r = Reminder {
                id: ReminderId(Arc::from(format!("rem-{}", g.next_id))),
                text: Arc::from(text.as_str()),
                due: None,
                recurring: false,
                completed: false,
                fired: false,
                interval_secs: 0,
                for_user: user
                    .clone()
                    .or_else(|| g.self_user.clone())
                    .expect("self user"),
                link: None,
                snoozed_until: None,
            };
            g.reminders.push(r.clone());
            g.calls.push(Call::AddReminder { text, when, user });
            r
        })
    }

    fn complete_reminder(&self, id: &ReminderId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        self.m3(self.caps.reminders, Op::CompleteReminder, move |g| {
            for r in g.reminders.iter_mut().filter(|r| r.id == id) {
                r.completed = true;
            }
            g.calls.push(Call::CompleteReminder(id));
        })
    }

    fn delete_reminder(&self, id: &ReminderId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        self.m3(self.caps.reminders, Op::DeleteReminder, move |g| {
            g.reminders.retain(|r| r.id != id);
            g.calls.push(Call::DeleteReminder(id));
        })
    }

    fn members(&self, ids: &[UserId]) -> BoxFuture<'_, Result<Vec<Member>, ProviderError>> {
        let ids = ids.to_vec();
        self.m3(self.caps.member_lookup, Op::Members, move |g| {
            g.calls.push(Call::Members(ids.clone()));
            ids.iter()
                .filter_map(|u| {
                    g.members
                        .iter()
                        .chain(g.lookup_members.iter())
                        .find(|m| m.id == *u)
                        .cloned()
                })
                .collect()
        })
    }

    fn profile(&self, u: &UserId) -> BoxFuture<'_, Result<Profile, ProviderError>> {
        let u = u.clone();
        let r = self.m3(self.caps.status, Op::Profile, move |g| {
            g.profiles
                .get(&u)
                .cloned()
                .ok_or_else(|| ProviderError::Other("not found".into()))
        });
        Box::pin(async move { r.await? })
    }

    fn set_status(&self, s: Option<Status>) -> BoxFuture<'_, Result<(), ProviderError>> {
        self.m3(self.caps.status, Op::SetStatus, move |g| {
            g.calls.push(Call::SetStatus(s));
        })
    }

    fn set_presence(&self, p: PresenceSetting) -> BoxFuture<'_, Result<(), ProviderError>> {
        self.m3(self.caps.presence, Op::SetPresence, move |g| {
            g.calls.push(Call::SetPresence(p));
        })
    }

    fn watch_presence(&self, users: &[UserId]) -> BoxFuture<'_, Result<(), ProviderError>> {
        let users = users.to_vec();
        self.m3(self.caps.presence_watch, Op::WatchPresence, move |g| {
            g.calls.push(Call::WatchPresence(users));
        })
    }

    /// Creates the one-to-one conversation (id `dm-<user>`) unless one exists.
    fn open_direct(&self, user: &UserId) -> BoxFuture<'_, Result<Conversation, ProviderError>> {
        let user = user.clone();
        self.m3(self.caps.open_direct, Op::OpenDirect, move |g| {
            g.calls.push(Call::OpenDirect(user.clone()));
            let existing = g
                .conversations
                .iter()
                .find(|c| c.kind == ConversationKind::Direct && c.peer.as_ref() == Some(&user))
                .cloned();
            existing.unwrap_or_else(|| {
                let mut c = conv(&format!("dm-{}", user.0), "");
                c.kind = ConversationKind::Direct;
                c.peer = Some(user.clone());
                g.conversations.push(c.clone());
                c
            })
        })
    }

    fn create_channel(
        &self,
        name: &str,
        private: bool,
    ) -> BoxFuture<'_, Result<Conversation, ProviderError>> {
        let name: Arc<str> = Arc::from(name);
        self.m3(self.caps.channel_admin, Op::Admin, move |g| {
            g.calls.push(Call::Admin(crate::AdminCall::Create {
                name: name.clone(),
                private,
            }));
            g.next_id += 1;
            let mut c = conv(&format!("new-{}", g.next_id), &name);
            if private {
                c.kind = ConversationKind::PrivateChannel;
            }
            c.member_count = Some(1);
            g.conversations.push(c.clone());
            c
        })
    }

    fn browse_channels(
        &self,
        query: &str,
        _cursor: Option<Arc<str>>,
        limit: u32,
    ) -> BoxFuture<'_, Result<crate::search::Page<Conversation>, ProviderError>> {
        let needle = query.trim().to_lowercase();
        let typed = query.to_string();
        self.m3(self.caps.channel_admin, Op::BrowseChannels, move |g| {
            g.calls.push(Call::BrowseChannels(typed));
            crate::search::Page {
                items: g
                    .conversations
                    .iter()
                    .filter(|c| {
                        matches!(
                            c.kind,
                            ConversationKind::Channel | ConversationKind::PrivateChannel
                        ) && !c.is_archived
                            && (c.kind == ConversationKind::Channel || c.is_member)
                            && c.name.to_lowercase().contains(&needle)
                    })
                    .take(limit as usize)
                    .cloned()
                    .collect(),
                next: None,
            }
        })
    }

    fn join_conversation(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Conversation, ProviderError>> {
        let id = c.clone();
        self.m3(self.caps.channel_admin, Op::Admin, move |g| {
            g.calls
                .push(Call::Admin(crate::AdminCall::Join(id.clone())));
            let found = g.conversations.iter_mut().find(|c| c.id == id);
            match found {
                Some(c) => {
                    c.is_member = true;
                    c.clone()
                }
                None => {
                    let c = conv(&id.0, &id.0);
                    g.conversations.push(c.clone());
                    c
                }
            }
        })
    }

    fn leave_conversation(&self, c: &ConversationId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = c.clone();
        self.m3(self.caps.channel_admin, Op::Admin, move |g| {
            g.calls
                .push(Call::Admin(crate::AdminCall::Leave(id.clone())));
            if let Some(c) = g.conversations.iter_mut().find(|c| c.id == id) {
                c.is_member = false;
            }
        })
    }

    fn archive_conversation(&self, c: &ConversationId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = c.clone();
        self.m3(self.caps.channel_admin, Op::Admin, move |g| {
            g.calls
                .push(Call::Admin(crate::AdminCall::Archive(id.clone())));
            if let Some(c) = g.conversations.iter_mut().find(|c| c.id == id) {
                c.is_archived = true;
            }
        })
    }

    fn rename_conversation(
        &self,
        c: &ConversationId,
        name: &str,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (id, name): (_, Arc<str>) = (c.clone(), Arc::from(name));
        self.m3(self.caps.channel_admin, Op::Admin, move |g| {
            g.calls.push(Call::Admin(crate::AdminCall::Rename {
                conversation: id.clone(),
                name: name.clone(),
            }));
            if let Some(c) = g.conversations.iter_mut().find(|c| c.id == id) {
                c.name = name;
            }
        })
    }

    fn set_topic(
        &self,
        c: &ConversationId,
        topic: &str,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (id, topic): (_, Arc<str>) = (c.clone(), Arc::from(topic));
        self.m3(self.caps.channel_admin, Op::Admin, move |g| {
            g.calls.push(Call::Admin(crate::AdminCall::SetTopic {
                conversation: id.clone(),
                topic: topic.clone(),
            }));
            if let Some(c) = g.conversations.iter_mut().find(|c| c.id == id) {
                c.topic = topic;
            }
        })
    }

    fn set_purpose(
        &self,
        c: &ConversationId,
        purpose: &str,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (id, purpose): (_, Arc<str>) = (c.clone(), Arc::from(purpose));
        self.m3(self.caps.channel_admin, Op::Admin, move |g| {
            g.calls.push(Call::Admin(crate::AdminCall::SetPurpose {
                conversation: id.clone(),
                purpose: purpose.clone(),
            }));
            if let Some(c) = g.conversations.iter_mut().find(|c| c.id == id) {
                c.purpose = purpose;
            }
        })
    }

    fn invite(
        &self,
        c: &ConversationId,
        users: &[UserId],
    ) -> BoxFuture<'_, Result<crate::InviteReport, ProviderError>> {
        let (id, users) = (c.clone(), users.to_vec());
        self.m3(self.caps.channel_admin, Op::Admin, move |g| {
            g.calls.push(Call::Admin(crate::AdminCall::Invite {
                conversation: id.clone(),
                users: users.clone(),
            }));
            let mut r = crate::InviteReport::default();
            let inside = g.channel_members.entry(id.clone()).or_default();
            for u in users {
                if inside.contains(&u) {
                    r.failed
                        .push((u, ProviderError::NotAllowed("already_in_channel".into())));
                } else {
                    inside.push(u.clone());
                    r.added.push(u);
                }
            }
            let added = r.added.len() as u32;
            if let Some(c) = g.conversations.iter_mut().find(|c| c.id == id) {
                c.member_count = Some(c.member_count.unwrap_or(0) + added);
            }
            r
        })
    }

    fn conversation_members(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Vec<UserId>, ProviderError>> {
        let id = c.clone();
        self.m3(self.caps.channel_admin, Op::Admin, move |g| {
            g.channel_members.get(&id).cloned().unwrap_or_default()
        })
    }

    /// Simple rules: lowercase, spaces as hyphens; letters, digits, `-`, `_`.
    fn check_channel_name(&self, typed: &str) -> crate::ChannelName {
        fake_channel_name(typed)
    }

    fn set_conversation_starred(
        &self,
        c: &ConversationId,
        starred: bool,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let conversation = c.clone();
        self.m3(self.caps.conversation_stars, Op::Admin, move |g| {
            g.calls.push(Call::SetConversationStarred {
                conversation,
                starred,
            });
        })
    }

    fn dnd(&self) -> BoxFuture<'_, Result<DndStatus, ProviderError>> {
        self.m3(self.caps.dnd, Op::Dnd, |g| g.dnd.clone())
    }

    fn dnd_users(
        &self,
        users: &[UserId],
    ) -> BoxFuture<'_, Result<HashMap<UserId, DndStatus>, ProviderError>> {
        let users = users.to_vec();
        self.m3(self.caps.dnd, Op::DndUsers, move |g| {
            users
                .iter()
                .filter_map(|u| g.dnd_users.get(u).map(|d| (u.clone(), d.clone())))
                .collect()
        })
    }

    /// The fake has no clock: `snooze_until` is `minutes * 60`.
    fn snooze(&self, minutes: u32) -> BoxFuture<'_, Result<DndStatus, ProviderError>> {
        self.m3(self.caps.dnd, Op::Snooze, move |g| {
            g.dnd.snooze_until = Some(i64::from(minutes) * 60);
            g.calls.push(Call::Snooze(minutes));
            g.dnd.clone()
        })
    }

    fn end_snooze(&self) -> BoxFuture<'_, Result<DndStatus, ProviderError>> {
        self.m3(self.caps.dnd, Op::EndSnooze, |g| {
            g.dnd.snooze_until = None;
            g.calls.push(Call::EndSnooze);
            g.dnd.clone()
        })
    }

    fn notify_prefs(
        &self,
    ) -> BoxFuture<'_, Result<HashMap<ConversationId, ConversationPrefs>, ProviderError>> {
        self.m3(self.caps.notify_prefs, Op::NotifyPrefs, |g| {
            g.notify_prefs.clone()
        })
    }

    fn set_notify_prefs(
        &self,
        c: &ConversationId,
        p: ConversationPrefs,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let c = c.clone();
        self.m3(self.caps.notify_prefs, Op::SetNotifyPrefs, move |g| {
            g.notify_prefs.insert(c.clone(), p);
            g.calls.push(Call::SetNotifyPrefs {
                conversation: c,
                prefs: p,
            });
        })
    }

    fn huddles(&self) -> BoxFuture<'_, Result<Vec<Huddle>, ProviderError>> {
        self.m3(self.caps.huddles, Op::Huddles, |g| g.huddles.clone())
    }

    fn decline_huddle(
        &self,
        c: &ConversationId,
        id: &HuddleId,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (conversation, id) = (c.clone(), id.clone());
        self.m3(self.caps.huddles, Op::DeclineHuddle, move |g| {
            g.calls.push(Call::DeclineHuddle { conversation, id });
        })
    }

    fn call_surface(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<CallSurface, ProviderError>> {
        let c = c.clone();
        let r = self.m3(self.caps.call_surface, Op::CallSurface, move |g| {
            g.calls.push(Call::CallSurface(c));
            match &g.surface {
                Some(f) => f(),
                None => Err(ProviderError::Unsupported),
            }
        });
        Box::pin(async move { r.await? })
    }

    fn native_call_join(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<NativeCallJoin, ProviderError>> {
        let c = c.clone();
        let me = self.lock().self_user.clone();
        let r = self.m3(self.caps.native_calls, Op::NativeCallJoin, move |g| {
            g.calls.push(Call::NativeCallJoin(c.clone()));
            let id = c.0.to_string();
            let user = me.map(|u| u.0.to_string()).unwrap_or_default();
            Ok(NativeCallJoin {
                meeting_id: format!("meeting-{id}").into(),
                attendee_id: format!("attendee-{user}").into(),
                external_user_id: user.into(),
                join_token: SecretText::new(format!("join-token-{id}")),
                signaling_url: format!("wss://signal.example.test/control/meeting-{id}").into(),
                audio_host_url: "audio.example.test:443".into(),
                turn_control_url: Some("https://turn.example.test/v2/turn_sessions".into()),
            })
        });
        Box::pin(async move { r.await? })
    }

    fn leave_huddle(&self, c: &ConversationId) -> BoxFuture<'_, Result<(), ProviderError>> {
        let c = c.clone();
        self.m3(true, Op::LeaveHuddle, move |g| {
            g.calls.push(Call::LeaveHuddle(c));
        })
    }

    fn message(&self, id: &MessageId) -> BoxFuture<'_, Result<Message, ProviderError>> {
        let id = id.clone();
        let r = self.m3(true, Op::Message, move |g| {
            g.messages
                .get(&id.conversation)
                .and_then(|v| v.iter().find(|m| m.id == id))
                .cloned()
                .ok_or_else(|| ProviderError::Other("not found".into()))
        });
        Box::pin(async move { r.await? })
    }

    fn conversation(
        &self,
        c: &ConversationId,
    ) -> BoxFuture<'_, Result<Conversation, ProviderError>> {
        let id = c.clone();
        let r = self.m3(self.caps.conversation_lookup, Op::Conversation, move |g| {
            g.conversations
                .iter()
                .find(|c| c.id == id)
                .cloned()
                .ok_or_else(|| ProviderError::Other("not found".into()))
        });
        Box::pin(async move { r.await? })
    }

    fn pins(&self, c: &ConversationId) -> BoxFuture<'_, Result<Vec<Pin>, ProviderError>> {
        let c = c.clone();
        self.m3(self.caps.pins, Op::Pins, move |g| {
            g.pins.get(&c).cloned().unwrap_or_default()
        })
    }

    fn set_pinned(&self, id: &MessageId, pinned: bool) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        self.m3(self.caps.pins, Op::SetPinned, move |g| {
            let by = g.self_user.clone();
            let list = g.pins.entry(id.conversation.clone()).or_default();
            list.retain(|p| p.message != id);
            if pinned {
                list.push(Pin {
                    message: id.clone(),
                    by,
                    at: 0,
                });
            }
            if let Some(m) = g
                .messages
                .get_mut(&id.conversation)
                .and_then(|v| v.iter_mut().find(|m| m.id == id))
            {
                m.pinned = pinned;
            }
            g.calls.push(Call::SetPinned { id, pinned });
        })
    }

    fn bookmarks(&self, c: &ConversationId) -> BoxFuture<'_, Result<Vec<Bookmark>, ProviderError>> {
        let c = c.clone();
        self.m3(self.caps.bookmarks, Op::Bookmarks, move |g| {
            g.bookmarks.get(&c).cloned().unwrap_or_default()
        })
    }

    fn add_bookmark(
        &self,
        c: &ConversationId,
        title: &str,
        link: &str,
    ) -> BoxFuture<'_, Result<Bookmark, ProviderError>> {
        let (c, title, link) = (c.clone(), title.to_owned(), link.to_owned());
        self.m3(self.caps.bookmarks, Op::AddBookmark, move |g| {
            g.next_id += 1;
            let b = Bookmark {
                id: BookmarkId(Arc::from(format!("bm-{}", g.next_id))),
                conversation: c.clone(),
                title: Arc::from(title.as_str()),
                link: Arc::from(link.as_str()),
                emoji: None,
            };
            g.bookmarks.entry(c.clone()).or_default().push(b.clone());
            g.calls.push(Call::AddBookmark {
                conversation: c,
                title,
                link,
            });
            b
        })
    }

    fn remove_bookmark(
        &self,
        c: &ConversationId,
        id: &BookmarkId,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let (c, id) = (c.clone(), id.clone());
        self.m3(self.caps.bookmarks, Op::RemoveBookmark, move |g| {
            if let Some(v) = g.bookmarks.get_mut(&c) {
                v.retain(|b| b.id != id);
            }
            g.calls.push(Call::RemoveBookmark {
                conversation: c,
                id,
            });
        })
    }

    fn saved(
        &self,
        _cursor: Option<Arc<str>>,
    ) -> BoxFuture<'_, Result<Page<SavedItem>, ProviderError>> {
        self.m3(self.caps.saved, Op::Saved, |g| Page {
            items: g.saved.clone(),
            next: None,
        })
    }

    fn set_thread_followed(
        &self,
        root: &MessageId,
        follow: bool,
        _last_read: Option<&MessageTs>,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        let root = root.clone();
        self.m3(self.caps.thread_follow, Op::SetThreadFollowed, move |g| {
            g.calls.push(Call::SetThreadFollowed { root, follow });
        })
    }

    fn set_saved(&self, id: &MessageId, saved: bool) -> BoxFuture<'_, Result<(), ProviderError>> {
        let id = id.clone();
        self.m3(self.caps.saved, Op::SetSaved, move |g| {
            g.saved.retain(|s| s.message != id);
            if saved {
                g.saved.push(SavedItem {
                    message: id.clone(),
                    saved_at: 0,
                    due: None,
                    done: false,
                });
            }
            if let Some(m) = g
                .messages
                .get_mut(&id.conversation)
                .and_then(|v| v.iter_mut().find(|m| m.id == id))
            {
                m.saved = saved;
            }
            g.calls.push(Call::SetSaved { id, saved });
        })
    }

    fn thread_inbox(
        &self,
        _cursor: Option<Arc<str>>,
    ) -> BoxFuture<'_, Result<Page<InboxThread>, ProviderError>> {
        self.m3(self.caps.thread_inbox, Op::ThreadInbox, |g| Page {
            items: g.inbox.clone(),
            next: None,
        })
    }

    fn huddle_url(&self, c: &ConversationId) -> Option<String> {
        self.caps
            .huddles
            .then(|| format!("https://fake.example.test/huddle/{}", c.0))
    }

    fn app_url(&self, c: &ConversationId) -> Option<String> {
        self.caps
            .huddles
            .then(|| format!("fake-app://open/{}", c.0))
    }

    fn conversation_web_url(&self, c: &ConversationId) -> Option<String> {
        let origin = self
            .lock()
            .web_origin
            .clone()
            .unwrap_or_else(|| "https://fake.example.test".into());
        Some(format!("{}/{}", origin.trim_end_matches('/'), c.0))
    }

    fn web_url(&self, id: &MessageId) -> Option<String> {
        let origin = self
            .lock()
            .web_origin
            .clone()
            .unwrap_or_else(|| "https://fake.example.test".into());
        Some(format!(
            "{}/{}/p{}",
            origin.trim_end_matches('/'),
            id.conversation.0,
            id.ts.0
        ))
    }

    fn web_origin(&self) -> Option<String> {
        self.lock().web_origin.clone()
    }

    fn reconnect_now(&self) {
        self.lock().calls.push(Call::ReconnectNow);
    }

    fn typing(&self, conversation: &ConversationId, thread: Option<&MessageTs>) {
        if self.caps.typing {
            self.lock().calls.push(Call::Typing {
                conversation: conversation.clone(),
                thread: thread.cloned(),
            });
        }
    }

    fn events(&self) -> BoxStream<'static, ProviderEvent> {
        let (tx, rx) = mpsc::unbounded();
        let mut g = self.lock();
        for ev in g.backlog.drain(..) {
            let _ = tx.unbounded_send(ev);
        }
        g.subscribers.push(tx);
        rx.boxed()
    }
}

/// [`ProviderFactory`] for the fake: display name `"Fake"`, no ext codec,
/// plain-text parser. `start` hands out the provider set with
/// [`with_provider`](Self::with_provider).
pub struct FakeFactory {
    id: ProviderId,
    /// One per account; `with_provider` may be called several times.
    providers: Vec<Arc<FakeProvider>>,
    /// Message markup parser (plain text by default).
    parser: Option<Arc<dyn ContentParser>>,
    /// Scripted accounts; when set they replace the provider's own.
    accounts: Mutex<Option<Vec<AccountInfo>>>,
    removed: Mutex<Vec<WorkspaceKey>>,
    permalink: Option<PermalinkParser>,
}

type PermalinkParser = Arc<dyn Fn(&url::Url) -> Option<crate::ParsedPermalink> + Send + Sync>;

impl Default for FakeFactory {
    fn default() -> Self {
        Self {
            id: ProviderId("fake".into()),
            providers: Vec::new(),
            parser: None,
            accounts: Mutex::new(None),
            removed: Mutex::new(Vec::new()),
            permalink: None,
        }
    }
}

impl FakeFactory {
    pub fn with_provider(mut self, p: Arc<FakeProvider>) -> Self {
        self.providers.push(p);
        self
    }

    /// Parse message text with `parser` instead of as plain text.
    pub fn with_parser(mut self, parser: Arc<dyn ContentParser>) -> Self {
        self.parser = Some(parser);
        self
    }

    pub fn with_permalink_parser(
        mut self,
        f: impl Fn(&url::Url) -> Option<crate::ParsedPermalink> + Send + Sync + 'static,
    ) -> Self {
        self.permalink = Some(Arc::new(f));
        self
    }

    /// Script the accounts (replaces the provider's own).
    pub fn set_accounts(&self, all: Vec<AccountInfo>) {
        *self.accounts.lock().expect("fake factory lock") = Some(all);
    }

    /// Workspaces passed to `remove`, in order.
    pub fn removed(&self) -> Vec<WorkspaceKey> {
        self.removed.lock().expect("fake factory lock").clone()
    }
}

impl ProviderFactory for FakeFactory {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn display_name(&self) -> &str {
        "Fake"
    }

    fn accounts(&self) -> Vec<WorkspaceKey> {
        if let Some(a) = &*self.accounts.lock().expect("fake factory lock") {
            return a.iter().map(|i| i.key.clone()).collect();
        }
        self.providers
            .iter()
            .map(|p| p.workspace().clone())
            .collect()
    }

    fn account_info(&self) -> Vec<AccountInfo> {
        if let Some(a) = &*self.accounts.lock().expect("fake factory lock") {
            return a.clone();
        }
        self.accounts()
            .into_iter()
            .map(|key| AccountInfo {
                name: Arc::from(key.id.0.as_str()),
                key,
                org: None,
                icon: None,
            })
            .collect()
    }

    fn remove(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<(), ProviderError>> {
        if let Some(a) = &mut *self.accounts.lock().expect("fake factory lock") {
            a.retain(|i| &i.key != ws);
        }
        self.removed
            .lock()
            .expect("fake factory lock")
            .push(ws.clone());
        Box::pin(ready(Ok(())))
    }

    fn encoder(&self) -> Arc<dyn ContentEncoder> {
        Arc::new(PlainEncoder)
    }

    fn start(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Arc<dyn ChatProvider>, ProviderError>> {
        let r = match self.providers.iter().find(|p| p.workspace() == ws) {
            Some(p) => Ok(p.clone() as Arc<dyn ChatProvider>),
            None => Err(ProviderError::Other("unknown workspace".into())),
        };
        Box::pin(ready(r))
    }

    fn ext_codec(&self) -> Arc<dyn ExtCodec> {
        Arc::new(NoExtCodec)
    }

    fn content_parser(&self) -> Arc<dyn ContentParser> {
        self.parser.clone().unwrap_or_else(|| Arc::new(PlainParser))
    }

    fn catalog(&self) -> &'static i18n::Catalog {
        fake_catalog()
    }

    fn parse_permalink(&self, url: &url::Url) -> Option<crate::ParsedPermalink> {
        self.permalink.as_ref().and_then(|f| f(url))
    }
}
