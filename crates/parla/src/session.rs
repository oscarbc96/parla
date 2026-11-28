//! `WorkspaceSession`: one GPUI entity per active workspace. It feeds provider
//! events into the pure reducer, runs the returned effects (provider calls on
//! the Tokio runtime, store writes) and emits one [`SessionEvent`] per change.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use crate::notifications::Notifications;
use components::{
    Completion, CompletionKind, ComposerSource, FileNotice, MemberIndex, SearchResults, SearchRow,
    SearchScope, SearchSource, SearchTarget, SidebarRow, SidebarSource, ThreadSource,
    TimelineSource, author_name, format_date_time,
};
use domain::notify::{NotifyContext, decide};
use domain::sync::{self, Older};
use domain::{
    Capabilities, Changed, ChatProvider, ClientMsgId, ContentParser, Conversation, ConversationId,
    CustomEmoji, DraftKey, Effect, EmojiRef, HistoryPage, HitSource, Input, Member, Message,
    MessageHit, MessageId, MessageTs, MessageWindow, OutboxOp, PlainParser, ProviderError,
    ProviderEvent, SearchQuery, Session, UserId, WorkspaceState, reduce,
};
use futures::StreamExt as _;
use gpui::{App, AppContext as _, Context, EventEmitter, Task, WeakEntity};
use settings::SettingsStore;
use store::{MemoryStore, Store};
use store_sqlite::SqliteStore;

/// The error an MCP send resolves with while the workspace is offline.
pub const QUEUED_OFFLINE: &str = "queued_offline";
const HISTORY_PAGE: u32 = 50;

/// Messages kept per open conversation: `performance.message_window` with
/// this workspace's layer (the default 200 without a settings store).
fn message_window(workspace: &settings::WorkspaceKey, cx: &App) -> usize {
    let n = match cx.try_global::<SettingsStore>() {
        Some(store) => store.for_workspace(workspace).performance.message_window,
        None => settings::Settings::default().performance.message_window,
    };
    (n as usize).max(1)
}

/// What changed, so a view re-renders only for its own data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    Conversations,
    /// The first-sync progress moved (store opened, bootstrap answered,
    /// a catch-up job finished).
    SyncProgress,
    Timeline(ConversationId),
    Members,
    Connection,
    Session,
    Thread(MessageId),
    Emoji,
    Lean,
    Scheduled,
    /// Late local schedules need an answer.
    LatePrompt,
    /// The Scheduled view was asked for (sidebar entry).
    ShowScheduled,
    /// The Reminders view was asked for (sidebar entry).
    ShowReminders,
    /// A conversation row was picked in the sidebar (even the open one).
    Picked(ConversationId),
    /// The Threads view was asked for (sidebar entry).
    ShowThreads,
    /// The Later view was asked for (sidebar entry).
    ShowLater,
    /// The Drafts view was asked for (sidebar entry).
    ShowDrafts,
    /// The People directory was asked for (sidebar entry).
    ShowPeople,
    /// The direct message asked for (New message, a person) is open now.
    DirectOpened(UserId),
    /// The direct message asked for could not be opened.
    /// The reason is in the active language.
    DirectFailed(UserId, String),
    /// A channel admin call finished; the error is in the active language.
    AdminFinished {
        call: domain::AdminCall,
        result: Result<(), String>,
    },
    /// A draft was deleted from the Drafts view: a composer showing it empties.
    DraftDeleted(DraftKey),
    /// A person's profile was asked for (author, mention, search, switcher).
    ShowProfile,
    /// A jump target was set or a window detached or reattached.
    Jump(ConversationId),
    Reminders,
    /// DND status changed (self or someone on screen).
    Dnd,
    /// Per-conversation notification levels changed.
    NotifyPrefs,
    /// A profile or the own presence setting changed.
    Profile(UserId),
    /// Someone on screen became active or away.
    Presence,
    /// Who is typing in this conversation (or one of its threads) changed.
    Typing(ConversationId),
    Huddles,
    Ring,
    /// A conversation's pins changed.
    Pins(ConversationId),
    /// A conversation's bookmarks changed.
    Bookmarks(ConversationId),
    /// Saved for later changed.
    Saved,
    /// The Threads inbox changed.
    ThreadInbox,
    /// The store's health changed (unreadable, rebuilt).
    Store,
}

/// Whether the on-disk cache could be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreHealth {
    Ok,
    /// Wrong key or corrupt file: the session runs from memory and nothing on
    /// disk is touched until the user rebuilds.
    Unreadable,
}

impl From<Changed> for SessionEvent {
    fn from(c: Changed) -> Self {
        match c {
            Changed::Conversations => SessionEvent::Conversations,
            Changed::Timeline(id) => SessionEvent::Timeline(id),
            Changed::Members => SessionEvent::Members,
            Changed::Connection => SessionEvent::Connection,
            Changed::Session => SessionEvent::Session,
            Changed::Thread(id) => SessionEvent::Thread(id),
            Changed::Emoji => SessionEvent::Emoji,
            Changed::Lean => SessionEvent::Lean,
            Changed::Scheduled => SessionEvent::Scheduled,
            Changed::Jump(id) => SessionEvent::Jump(id),
            Changed::Reminders => SessionEvent::Reminders,
            Changed::Dnd => SessionEvent::Dnd,
            Changed::NotifyPrefs => SessionEvent::NotifyPrefs,
            Changed::Profile(u) => SessionEvent::Profile(u),
            Changed::Presence => SessionEvent::Presence,
            Changed::Huddles => SessionEvent::Huddles,
            Changed::Ring => SessionEvent::Ring,
            Changed::Typing(id) => SessionEvent::Typing(id),
            Changed::Pins(id) => SessionEvent::Pins(id),
            Changed::Bookmarks(id) => SessionEvent::Bookmarks(id),
            Changed::Saved => SessionEvent::Saved,
            Changed::ThreadInbox => SessionEvent::ThreadInbox,
        }
    }
}

/// How long a change of who is on screen settles before the provider is
/// told (scrolling or opening a profile in quick succession is one batch).
const PRESENCE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

/// How long the first bootstrap may take before the window says so.
const BOOTSTRAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub struct WorkspaceSession {
    pub(crate) provider: Arc<dyn ChatProvider>,
    store: Arc<dyn Store>,
    pub(crate) state: WorkspaceState,
    selected: Option<ConversationId>,
    /// The opened conversation that was unread, kept in "Unreads" while open.
    pinned_unread: Option<ConversationId>,
    /// Read marker of the conversation as it was when last opened (before the
    /// open marked it read): `None` inside when nothing was unread.
    opened_marker: Option<(ConversationId, Option<domain::MessageTs>)>,
    loading_older: HashSet<ConversationId>,
    /// Conversations whose first history page failed to load (cleared by a
    /// retry or a page arriving); the timeline offers Retry for them.
    history_failed: HashSet<ConversationId>,
    /// Threads whose replies failed to load, the same for the thread panel.
    replies_failed: HashSet<MessageId>,
    /// First history pages and thread reply fetches in flight (one at a time).
    history_pending: HashSet<ConversationId>,
    replies_pending: HashSet<MessageId>,
    /// A store reopen (leaving lean mode) is in flight.
    reopening: bool,
    /// Stay lean when the reopen in flight lands.
    lean_again: bool,
    pub(crate) content: Arc<dyn ContentParser>,
    /// Turns composed text into the provider's markup before it is sent.
    encoder: Arc<dyn domain::ContentEncoder>,
    /// Emoji used in this workspace, newest first (mirrors the store).
    recent_emoji: Vec<EmojiRef>,
    provider_name: String,
    this: WeakEntity<WorkspaceSession>,
    sqlite: Option<SqliteStore>,
    health: StoreHealth,
    /// Saved drafts and when each was last edited (Unix seconds).
    pub(crate) drafts: HashMap<DraftKey, (domain::ComposedText, i64)>,
    /// Sidebar sections the user collapsed (kept per workspace in the store).
    pub(crate) collapsed: std::collections::BTreeSet<components::SidebarSection>,
    /// Sections toggled this session: they win over the stored set when it
    /// loads after them.
    pub(crate) collapsed_touched: HashSet<components::SidebarSection>,
    /// Drafts deleted this session: a store load that answers later must not
    /// bring them back.
    pub(crate) drafts_deleted: HashSet<DraftKey>,
    bootstrapped: bool,
    /// How far the first sync got, for the first-run progress screen.
    sync_progress: components::onboarding::SyncProgress,
    /// Why the last bootstrap failed (not an expired session), for the window.
    bootstrap_error: Option<i18n::LocalizedText>,
    /// Bumped by every bootstrap attempt; a timeout of an older one is ignored.
    bootstrap_seq: u64,
    pub(crate) files: crate::files::FileState,
    /// The live post being reduced, so `MaybeNotify` can read it even when
    /// it lives in no loaded window (a reply in a closed thread).
    live_post: Option<Message>,
    /// The one typing expiry timer (`Effect::TypingWakeAt`); dropping it cancels it.
    typing_wake: Option<gpui::Task<()>>,
    /// Latest answer of the search panel's query, and the number of searches
    /// started (an older answer arriving late is dropped).
    search_results: SearchResults,
    /// The provider's conversations behind the remote Conversations rows
    /// (at most one search's worth), so a hit the list lacks can open.
    search_found: Vec<domain::Conversation>,
    /// Channels whose join is on its way (the composer's Join waits).
    joining: HashSet<ConversationId>,
    search_seq: u64,
    /// Messages around the search hit shown in context when its
    /// conversation is not held: one hit, a few messages.
    /// Recent answers, empty ones too (asked once), newest last; bounded by
    /// [`SEARCH_CONTEXT_KEEP`] and cleared by a new search.
    search_context: std::collections::VecDeque<(MessageId, Arc<[Message]>)>,
    /// The hit whose context is being read and the read itself; selecting
    /// another hit drops it, cancelling the store read or the request.
    search_context_pending: Option<MessageId>,
    search_context_task: Option<Task<()>>,
    /// Every message hit shown for the current search (device first, then
    /// the server's pages), so later pages do not repeat them.
    search_local: Vec<MessageHit>,
    /// Query and cursor of the server's next page of message hits.
    search_next: Option<(SearchQuery, Arc<str>)>,
    /// Built sidebar rows and the member index behind them (see [`SidebarCache`]).
    sidebar_cache: std::cell::RefCell<SidebarCache>,
    /// Set while a single member update is applied (the index was patched).
    member_patched: bool,
    /// The "Direct messages" section lists every conversation ("Show more").
    dms_expanded: bool,
    /// The people whose presence is on screen now (sorted); the provider is
    /// told after [`PRESENCE_DEBOUNCE`].
    presence_desired: Vec<UserId>,
    /// The People directory's cards on screen (their presence is watched).
    directory_people: Vec<UserId>,
    /// Bumped whenever members change (the People directory's cache key).
    members_rev: u64,
    /// Single-member changes `(revision, id)` since `member_log_floor`, at
    /// most `MEMBER_LOG`, so the directory patches instead of rebuilding.
    member_log: std::collections::VecDeque<(u64, UserId)>,
    /// Changes at or before this revision are not known one by one.
    member_log_floor: u64,
    /// The one member the input being applied can change, if any.
    member_touch: Option<UserId>,
    /// A debounce timer is pending (one at a time, none while idle).
    presence_timer: bool,
    /// Settings with this workspace's file layered on top, rebuilt when the
    /// settings store's generation moves (hot reload of either file).
    settings_cache: std::cell::RefCell<Option<(u64, Arc<settings::Settings>)>>,
    /// Kept to reopen the store when leaving lean mode.
    codec: Option<Arc<dyn domain::ExtCodec>>,
    pub(crate) local_runner: crate::local_schedules::LocalRunner,
    pub(crate) notify_runner: crate::notify_controls::NotifyRunner,
    pub(crate) mute_runner: crate::notify_controls::MuteRunner,
    pub(crate) reminder_runner: crate::reminders::ReminderRunner,
    /// The latest `CallFinished` / `Loaded` fed back (one slot, no history).
    /// Only kept for tests, which build with the `fixture` feature.
    #[cfg(feature = "fixture")]
    pub(crate) last_feedback: Option<Input>,
    /// MCP sends waiting for their delivery result, by client message id.
    send_waiters: HashMap<ClientMsgId, SendWaiter>,
    /// Timing lines of each send (phases and milliseconds only).
    send_timing: crate::send_timing::SendTiming,
    lost_sends: crate::send_timing::LostSends,
}

type SendWaiter = futures::channel::oneshot::Sender<Result<Message, ProviderError>>;

/// What the sidebar rows were built from besides the state, so a settings or
/// language change rebuilds them.
type RowsKey = (settings::SidebarSettings, i18n::LanguageIdentifier);

/// The sidebar's rows and member index, rebuilt only after a change that can
/// affect them (conversations, members, selection, drafts, settings, locale),
/// not on every session notification. One slot each, so it stays bounded.
#[derive(Default)]
struct SidebarCache {
    index: Option<Arc<MemberIndex>>,
    /// Key, rows, and the ids of the conversation rows (for the presence
    /// watch, so it never clones the rows).
    rows: Option<(RowsKey, Vec<SidebarRow>, Vec<ConversationId>)>,
}

impl EventEmitter<SessionEvent> for WorkspaceSession {}

impl WorkspaceSession {
    /// Subscribes to the provider's events, then bootstraps. Uses `store` as is.
    pub fn new(
        provider: Arc<dyn ChatProvider>,
        store: Arc<dyn Store>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(provider, store, None, cx);
        this.load_recent_emoji(cx);
        this.load_local_notify_prefs(cx);
        this.load_local_notify_extras(cx);
        this.load_starred(cx);
        this.load_collapsed(cx);
        this.load_drafts(cx);
        this.load_local_reminders(cx);
        this.start_bootstrap(cx);
        this
    }

    /// Opens the encrypted store for the provider's workspace, paints what it
    /// holds, then bootstraps and catches up. An unreadable store leaves the
    /// session running on memory with [`StoreHealth::Unreadable`].
    pub fn new_sqlite(
        provider: Arc<dyn ChatProvider>,
        sqlite: SqliteStore,
        codec: Arc<dyn domain::ExtCodec>,
        cx: &mut Context<Self>,
    ) -> Self {
        let store: Arc<dyn Store> = Arc::new(sqlite.clone());
        let mut this = Self::build(provider.clone(), store, Some(sqlite.clone()), cx);
        this.codec = Some(codec.clone());
        let ws = this.state.workspace.clone();
        this.run(
            cx,
            async move { sqlite.open(&ws, codec).await },
            |s, r, cx| match r {
                Ok(()) => {
                    crate::log::info("cache store open ok");
                    s.note_sync(|p| p.store_ready = true, cx);
                    s.load_cache(cx);
                    s.load_recent_emoji(cx);
                    s.load_local_notify_prefs(cx);
                    s.load_local_notify_extras(cx);
                    s.load_starred(cx);
                    s.load_collapsed(cx);
                    s.load_local_reminders(cx);
                    s.prune_at_startup(cx);
                    s.check_store_usage(cx);
                }
                Err(e) => {
                    crate::log::error(&format!("cache store unreadable: {e}"));
                    s.store = Arc::new(MemoryStore::default());
                    s.health = StoreHealth::Unreadable;
                    s.note_sync(|p| p.store_ready = true, cx);
                    s.load_local_reminders(cx);
                    cx.emit(SessionEvent::Store);
                    cx.notify();
                    s.start_bootstrap(cx);
                }
            },
        );
        this
    }

    fn build(
        provider: Arc<dyn ChatProvider>,
        store: Arc<dyn Store>,
        sqlite: Option<SqliteStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let window = message_window(provider.workspace(), cx);
        let mut state = WorkspaceState::new(provider.workspace().clone(), window);
        // A changed `performance.message_window` applies to the next load and
        // trim; no timer, only the settings store's reload notification.
        cx.observe_global::<SettingsStore>(|s, cx| {
            s.state.window_capacity = message_window(&s.state.workspace, cx);
        })
        .detach();
        // The reducer reads capabilities to gate its bootstrap loads.
        state.caps = provider.capabilities().clone();
        if let Some(n) = cx.try_global::<Notifications>() {
            n.register(state.workspace.clone(), cx.weak_entity());
        }
        let mut events = provider.events();
        cx.spawn(async move |this, cx| {
            while let Some(ev) = events.next().await {
                if this
                    .update(cx, |s, cx| s.apply(Input::Provider(ev), cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let provider_name = state.workspace.provider.0.clone();
        let memory_store = sqlite.is_none();
        Self {
            provider,
            store,
            state,
            selected: None,
            pinned_unread: None,
            opened_marker: None,
            loading_older: HashSet::new(),
            history_failed: HashSet::new(),
            replies_failed: HashSet::new(),
            history_pending: HashSet::new(),
            replies_pending: HashSet::new(),
            reopening: false,
            lean_again: false,
            content: Arc::new(PlainParser),
            encoder: Arc::new(domain::PlainEncoder),
            recent_emoji: Vec::new(),
            provider_name,
            this: cx.weak_entity(),
            sqlite,
            health: StoreHealth::Ok,
            drafts: HashMap::new(),
            collapsed: Default::default(),
            collapsed_touched: HashSet::new(),
            drafts_deleted: HashSet::new(),
            bootstrapped: false,
            // An in-memory store is ready at once; the encrypted one when it opens.
            sync_progress: components::onboarding::SyncProgress {
                store_ready: memory_store,
                ..Default::default()
            },
            bootstrap_error: None,
            bootstrap_seq: 0,
            files: Default::default(),
            live_post: None,
            typing_wake: None,
            #[cfg(feature = "fixture")]
            last_feedback: None,
            search_results: SearchResults::default(),
            search_found: Vec::new(),
            joining: HashSet::new(),
            search_seq: 0,
            search_context: std::collections::VecDeque::new(),
            search_context_pending: None,
            search_context_task: None,
            search_local: Vec::new(),
            search_next: None,
            sidebar_cache: Default::default(),
            member_patched: false,
            dms_expanded: false,
            presence_desired: Vec::new(),
            directory_people: Vec::new(),
            members_rev: 0,
            member_log: std::collections::VecDeque::new(),
            member_log_floor: 0,
            member_touch: None,
            presence_timer: false,
            settings_cache: Default::default(),
            codec: None,
            local_runner: Default::default(),
            notify_runner: Default::default(),
            mute_runner: Default::default(),
            reminder_runner: Default::default(),
            send_waiters: HashMap::new(),
            send_timing: Default::default(),
            lost_sends: Default::default(),
        }
    }

    fn start_bootstrap(&mut self, cx: &mut Context<Self>) {
        let p = self.provider.clone();
        crate::log::info("bootstrap started");
        self.bootstrap_seq += 1;
        let seq = self.bootstrap_seq;
        // One timer per attempt (not polling): a stalled bootstrap shows an
        // error with a retry instead of an empty window forever.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BOOTSTRAP_TIMEOUT).await;
            this.update(cx, |s, cx| {
                if s.bootstrap_seq == seq
                    && !s.bootstrapped
                    && s.bootstrap_error.is_none()
                    && s.state.session != Session::Expired
                {
                    crate::log::error("bootstrap timed out");
                    s.bootstrap_error =
                        Some(i18n::LocalizedText::new("parla", "shell-bootstrap-timeout"));
                    s.note_sync(|p| p.failed = true, cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        self.run(cx, async move { p.bootstrap().await }, |s, r, cx| match r {
            Ok(b) => {
                s.bootstrapped = true;
                s.bootstrap_error = None;
                let found = (b.conversations.len(), b.members.len());
                s.note_sync(
                    |p| {
                        p.directory = Some(found);
                        p.failed = false;
                    },
                    cx,
                );
                crate::log::info(&format!(
                    "bootstrap ok: {} conversations, {} members, {} with a picture",
                    b.conversations.len(),
                    b.members.len(),
                    b.members.iter().filter(|m| m.avatar.is_some()).count()
                ));
                s.apply(Input::Bootstrapped(b), cx);
                s.load_outbox(cx);
                s.load_local_schedules(cx);
                s.sync(cx);
            }
            Err(ProviderError::SessionExpired) => {
                s.note_sync(|p| p.failed = true, cx);
                s.apply(Input::Provider(ProviderEvent::SessionExpired), cx)
            }
            Err(e) => {
                crate::log::error(&format!("bootstrap failed: {e}"));
                s.note_sync(|p| p.failed = true, cx);
                s.bootstrap_error = Some(crate::error_text::provider_error(&e));
                // No `Disconnected` here: the socket is the realtime task's to report.
                // Faking one left a live socket marked offline, so sends waited for a
                // reconnect that never came and each queued send hurried the backoff.
                cx.notify();
            }
        });
    }

    /// The effective settings for this workspace: `settings.json` with the
    /// workspace's own file on top. Cached; rebuilt only after a reload.
    pub fn settings(&self, cx: &App) -> Arc<settings::Settings> {
        let Some(store) = cx.try_global::<SettingsStore>() else {
            return Arc::new(settings::Settings::default());
        };
        let generation = store.generation();
        let mut cache = self.settings_cache.borrow_mut();
        if let Some((g, s)) = &*cache
            && *g == generation
        {
            return s.clone();
        }
        let s = Arc::new(store.for_workspace(&self.state.workspace));
        *cache = Some((generation, s.clone()));
        s
    }

    /// Whether the workspace is in lean mode (windows released, store closed).
    pub fn is_lean(&self) -> bool {
        self.state.lean
    }

    /// Enters or leaves lean mode. Entering releases the message windows and
    /// closes the store; the connection stays up, so counts and notifications
    /// keep working. Leaving reopens the store first and then reloads the open
    /// conversation from it.
    pub fn set_lean(&mut self, lean: bool, cx: &mut Context<Self>) {
        if self.reopening {
            // A reopen is in flight: the last call wins when it lands (the
            // idle lock must not find the store open afterwards).
            self.lean_again = lean;
            return;
        }
        if self.state.lean == lean {
            return;
        }
        let (Some(sqlite), true) = (self.sqlite.clone(), self.health == StoreHealth::Ok) else {
            self.apply(Input::SetLean(lean), cx);
            cx.notify();
            return;
        };
        let ws = self.state.workspace.clone();
        if lean {
            self.apply(Input::SetLean(true), cx);
            cx.notify();
            self.run(cx, async move { sqlite.close(&ws).await }, |_, _, _| {});
        } else {
            if self.reopening {
                return;
            }
            self.reopening = true;
            self.lean_again = false;
            let codec = self
                .codec
                .clone()
                .unwrap_or_else(|| Arc::new(domain::NoExtCodec));
            let (again_db, again_ws) = (sqlite.clone(), ws.clone());
            self.run(
                cx,
                async move { sqlite.open(&ws, codec).await },
                move |s, r, cx| {
                    s.reopening = false;
                    if std::mem::take(&mut s.lean_again) {
                        // Asked to stay lean meanwhile: close what just opened.
                        if r.is_ok() {
                            s.run(
                                cx,
                                async move { again_db.close(&again_ws).await },
                                |_, _, _| {},
                            );
                        }
                        cx.notify();
                        return;
                    }
                    if let Err(e) = r {
                        crate::log::error(&format!("cache store reopen failed: {e}"));
                        s.store = Arc::new(MemoryStore::default());
                        s.health = StoreHealth::Unreadable;
                        cx.emit(SessionEvent::Store);
                    }
                    s.apply(Input::SetLean(false), cx);
                    cx.notify();
                },
            );
        }
    }

    /// Whether a store reopen is in flight.
    pub fn store_busy(&self) -> bool {
        self.reopening
    }

    /// How far the first sync got (store, bootstrap, catch-up).
    pub fn sync_progress(&self) -> &components::onboarding::SyncProgress {
        &self.sync_progress
    }

    /// Changes the first-sync progress and tells the first-run screen when it moved.
    fn note_sync(
        &mut self,
        f: impl FnOnce(&mut components::onboarding::SyncProgress),
        cx: &mut Context<Self>,
    ) {
        let before = self.sync_progress.clone();
        f(&mut self.sync_progress);
        if self.sync_progress != before {
            cx.emit(SessionEvent::SyncProgress);
        }
    }

    /// True until the first bootstrap answered (and while no error is shown).
    pub fn connecting(&self) -> bool {
        !self.bootstrapped && self.bootstrap_error.is_none()
    }

    /// Starts the bootstrap again after a failure or a timeout.
    pub fn retry_bootstrap(&mut self, cx: &mut Context<Self>) {
        self.bootstrap_error = None;
        cx.notify();
        self.start_bootstrap(cx);
    }

    /// Asks the provider to skip its wait and reconnect now ("Retry now").
    pub fn retry_connection(&self) {
        self.provider.reconnect_now();
    }

    /// Why the last bootstrap failed, if it did.
    pub fn bootstrap_error(&self) -> Option<&i18n::LocalizedText> {
        self.bootstrap_error.as_ref()
    }

    /// Paints the sidebar, the newest window of the most recently active
    /// conversation and the drafts from the store, then bootstraps.
    fn load_cache(&self, cx: &mut Context<Self>) {
        let (st, ws) = (self.store.clone(), self.state.workspace.clone());
        let window = self.state.window_capacity;
        self.run(
            cx,
            async move {
                let conversations = st.conversations(&ws).await.unwrap_or_default();
                let members = st.members(&ws).await.unwrap_or_default();
                let latest = st.latest_all(&ws).await.unwrap_or_default();
                let conversation = latest
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1))
                    .map(|(c, _)| c);
                let mut messages = match &conversation {
                    Some(c) => st
                        .messages_before(&ws, c, None, window)
                        .await
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                messages.reverse();
                let drafts = st.drafts(&ws).await.unwrap_or_default();
                let groups = st.user_groups(&ws).await.unwrap_or_default();
                (
                    conversations,
                    members,
                    conversation,
                    messages,
                    drafts,
                    groups,
                )
            },
            |s, (conversations, members, conversation, messages, drafts, groups), cx| {
                crate::log::info(&format!(
                    "cache painted: {} conversations, {} members",
                    conversations.len(),
                    members.len()
                ));
                s.merge_loaded_drafts(drafts);
                s.sidebar_cache.borrow_mut().rows = None;
                s.apply(
                    Input::CacheLoaded {
                        conversations,
                        members,
                        conversation,
                        messages,
                    },
                    cx,
                );
                if !groups.is_empty() {
                    s.apply(Input::UserGroupsCached(groups), cx);
                }
                s.start_bootstrap(cx);
            },
        );
    }

    /// Catch-up: asks the provider what moved, then pages each conversation
    /// that did, one at a time, within `domain::sync`'s bound. Never notifies.
    fn sync(&self, cx: &mut Context<Self>) {
        let (p, st, ws) = (
            self.provider.clone(),
            self.store.clone(),
            self.state.workspace.clone(),
        );
        self.run(
            cx,
            async move {
                let counts = p.counts().await?;
                let stored = st.latest_all(&ws).await.unwrap_or_default();
                Ok::<_, ProviderError>((counts, stored))
            },
            |s, r, cx| {
                let Ok((counts, stored)) = r else {
                    // No counts, nothing to catch up: the first sync ends here.
                    s.note_sync(|p| p.catch_up = Some((0, 0)), cx);
                    return;
                };
                let jobs = sync::plan(&stored, &counts, |c| {
                    s.state.conversations.get(c).is_some_and(|c| c.is_member)
                });
                let planned = jobs.len();
                s.note_sync(|p| p.catch_up = Some((0, planned)), cx);
                s.apply(Input::CountsLoaded(counts), cx);
                let p = s.provider.clone();
                let handle = crate::runtime::handle(cx);
                cx.spawn(async move |this, cx| {
                    for job in jobs {
                        let p = p.clone();
                        let fut = async move { sync::run(&*p, job).await };
                        let out = match &handle {
                            Some(h) => h.spawn(fut).await.ok(),
                            None => Some(fut.await),
                        };
                        // Every job counts towards the first sync, a failed one too.
                        let alive = this.update(cx, |s, cx| {
                            if let Some(Ok(r)) = out {
                                s.apply(Input::CaughtUp(r), cx);
                            }
                            s.note_sync(
                                |p| {
                                    if let Some((done, _)) = &mut p.catch_up {
                                        *done += 1;
                                    }
                                },
                                cx,
                            );
                        });
                        if alive.is_err() {
                            break;
                        }
                    }
                })
                .detach();
            },
        );
    }

    /// The provider's display name, for banners.
    pub fn display_name(&self) -> &str {
        &self.provider_name
    }

    /// The provider's link to conversation `c`, when it has one.
    pub fn conversation_web_url(&self, c: &ConversationId) -> Option<String> {
        self.provider.conversation_web_url(c)
    }

    /// The workspace's web origin, when the provider knows one.
    pub fn web_origin(&self) -> Option<String> {
        self.provider.web_origin()
    }

    pub fn store_health(&self) -> StoreHealth {
        self.health
    }

    /// `store::RebuildCache`: deletes the unreadable file, replaces the key and
    /// starts over from the provider. Only ever runs on the user's request.
    pub fn rebuild_cache(&mut self, cx: &mut Context<Self>) {
        let Some(sqlite) = self.sqlite.clone() else {
            return;
        };
        let ws = self.state.workspace.clone();
        let reopened = sqlite.clone();
        self.run(
            cx,
            async move { sqlite.rebuild(&ws).await },
            move |s, r, cx| {
                if let Err(e) = &r {
                    crate::log::error(&format!("cache rebuild failed: {e}"));
                }
                if r.is_ok() {
                    s.store = Arc::new(reopened);
                    s.health = StoreHealth::Ok;
                    s.drafts.clear();
                    s.sidebar_cache.borrow_mut().rows = None;
                    cx.emit(SessionEvent::Store);
                    cx.notify();
                    s.start_bootstrap(cx);
                }
            },
        );
    }

    /// The saved draft for a conversation or thread composer.
    pub fn draft(&self, key: &DraftKey) -> Option<domain::ComposedText> {
        self.drafts.get(key).map(|(b, _)| b.clone())
    }

    /// Saves a draft (blank deletes it). Called on conversation switch, thread
    /// close and quit, never on a timer.
    pub fn set_draft(
        &mut self,
        key: DraftKey,
        body: impl Into<domain::ComposedText>,
        cx: &mut Context<Self>,
    ) {
        self.save_draft(key, body, cx).detach();
    }

    /// Like [`set_draft`](Self::set_draft), and the task resolves once the
    /// store has the draft (awaited on quit).
    pub fn save_draft(
        &mut self,
        key: DraftKey,
        body: impl Into<domain::ComposedText>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let body: domain::ComposedText = body.into();
        let body = (!body.text.trim().is_empty()).then_some(body);
        if body.is_some() {
            self.drafts_deleted.remove(&key);
        } else {
            self.drafts_deleted.insert(key.clone());
        }
        // An unchanged body keeps its edit time.
        let edited = match (&body, self.drafts.get(&key)) {
            (Some(b), Some((old, at))) if old == b => *at,
            _ => components::now_secs(cx),
        };
        let changed = match &body {
            Some(b) => {
                self.drafts
                    .insert(key.clone(), (b.clone(), edited))
                    .map(|(old, _)| old)
                    .as_ref()
                    != Some(b)
            }
            None => self.drafts.remove(&key).is_some(),
        };
        if changed {
            self.sidebar_cache.borrow_mut().rows = None;
            cx.notify();
        }
        let (st, ws) = (self.store.clone(), self.state.workspace.clone());
        let handle = crate::runtime::handle(cx);
        cx.background_executor().spawn(async move {
            let fut = async move {
                if st.draft_put(&ws, key, body, edited).await.is_err() {
                    crate::log::warn("draft save failed");
                }
            };
            match handle {
                Some(h) => {
                    h.spawn(fut).await.ok();
                }
                None => fut.await,
            }
        })
    }

    fn patch_sidebar_member(&mut self, m: &Member) {
        self.member_patched = true;
        let is_self = self.state.self_user.as_ref() == Some(&m.id);
        let old = self.state.members.get(&m.id);
        let mut cache = self.sidebar_cache.borrow_mut();
        let changed = match cache.index.as_mut() {
            Some(index) => Arc::make_mut(index).update(old, m, is_self),
            // No index yet: nothing cached to patch.
            None => false,
        };
        if changed {
            cache.rows = None;
        }
    }

    /// Drops the cached sidebar rows (and the member index) when `c` can change them.
    fn invalidate_sidebar(&self, c: &Changed) {
        let mut cache = self.sidebar_cache.borrow_mut();
        match c {
            Changed::Members if self.member_patched => {}
            Changed::Members | Changed::Session => {
                cache.index = None;
                cache.rows = None;
            }
            Changed::Conversations
            | Changed::Lean
            | Changed::NotifyPrefs
            | Changed::Presence
            | Changed::Huddles => cache.rows = None,
            _ => {}
        }
    }

    /// The member index the sidebar rows use, built once and kept until
    /// members change.
    pub(crate) fn member_index(&self) -> Arc<MemberIndex> {
        Self::index_of(&mut self.sidebar_cache.borrow_mut(), &self.state)
    }

    fn index_of(cache: &mut SidebarCache, state: &domain::WorkspaceState) -> Arc<MemberIndex> {
        cache
            .index
            .get_or_insert_with(|| Arc::new(MemberIndex::build(state)))
            .clone()
    }

    /// Drops the cached sidebar rows so the next read rebuilds them.
    pub(crate) fn invalidate_rows(&self) {
        self.sidebar_cache.borrow_mut().rows = None;
    }

    /// Conversations with a saved composer draft (thread drafts do not count).
    fn draft_conversations(&self) -> HashSet<ConversationId> {
        self.drafts
            .keys()
            .filter(|k| k.thread.is_none())
            .map(|k| k.conversation.clone())
            .collect()
    }

    /// Marks `id` and everything after it unread again.
    pub fn mark_unread(&mut self, id: MessageId, cx: &mut Context<Self>) {
        self.apply(Input::MarkUnread(id), cx);
    }

    /// Reports where the user's attention is, so read markers follow it.
    pub fn focus(
        &mut self,
        conversation: ConversationId,
        at_bottom: bool,
        window_active: bool,
        cx: &mut Context<Self>,
    ) {
        self.apply(
            Input::Focus {
                conversation: Some(conversation),
                at_bottom,
                window_active,
            },
            cx,
        );
    }

    /// Reads the free space of the store's volume, off the main thread, for
    /// the "Disk nearly full" banner. Called at startup, after writes and on
    /// window activation; the store lets one read through per five minutes
    /// over all workspaces. Never on a timer.
    pub(crate) fn check_store_usage(&self, cx: &mut Context<Self>) {
        let Some(sqlite) = self.sqlite.clone() else {
            return;
        };
        if !sqlite.claim_free_space_check(components::now(cx).as_millisecond()) {
            return;
        }
        let free = cx
            .background_executor()
            .spawn(async move { sqlite.read_free_space() });
        self.run(cx, free, |_, free, cx| {
            if let Some(free) = free {
                crate::store_usage::StoreUsage::record(free, cx);
            }
        });
    }

    /// Once per launch: drops messages past `storage.retention_days` in
    /// conversations not opened lately, off the main thread.
    fn prune_at_startup(&self, cx: &mut Context<Self>) {
        let Some(sqlite) = self.sqlite.clone() else {
            return;
        };
        let days = self.settings(cx).storage.retention_days;
        let ws = self.state.workspace.clone();
        let now_ms = components::now(cx).as_millisecond();
        self.run(
            cx,
            async move { sqlite.prune(&ws, now_ms, days).await },
            |_, _, _| {},
        );
    }

    /// Loads unsent messages once the self user is known (pending authors).
    fn load_outbox(&self, cx: &mut Context<Self>) {
        let (st, ws) = (self.store.clone(), self.state.workspace.clone());
        self.run(cx, async move { st.outbox(&ws).await }, |s, r, cx| {
            if let Ok(items) = r {
                s.apply(Input::OutboxLoaded(items), cx);
            }
        });
    }

    /// Fills an empty window of `c` from the store, so a conversation can be
    /// shown (and scrolled to a search hit) without reaching the provider.
    /// Does nothing when the window already holds messages.
    pub fn seed_from_store(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        if self.state.windows.get(&c).is_some_and(|w| !w.is_empty()) {
            return;
        }
        let (st, ws) = (self.store.clone(), self.state.workspace.clone());
        let window = self.state.window_capacity;
        let conv = c.clone();
        self.run(
            cx,
            async move {
                let mut v = st
                    .messages_before(&ws, &conv, None, window)
                    .await
                    .unwrap_or_default();
                v.reverse();
                v
            },
            move |s, messages, cx| {
                if !messages.is_empty() {
                    s.apply(
                        Input::CacheLoaded {
                            conversations: Vec::new(),
                            members: Vec::new(),
                            conversation: Some(c),
                            messages,
                        },
                        cx,
                    );
                }
            },
        );
    }

    /// One result row for a message: conversation name, author and time.
    fn hit_row(&self, m: &Message, snippet: &str, cx: &App) -> SearchRow {
        let clock = self.settings(cx).messages.clock;
        let tz = jiff::tz::TimeZone::system();
        let l = i18n::gpui::localizer(cx);
        let author = author_name(
            m.author.as_ref().and_then(|u| self.state.members.get(u)),
            &l,
        );
        SearchRow::message(
            m.id.clone(),
            self.conversation_label(&m.id.conversation),
            author,
            format_date_time(&m.id.ts, clock, &tz, &l),
            // Mentions come as ids; show the names, like a message does.
            components::resolve_user_ids(snippet, |id| {
                self.state
                    .members
                    .get(&UserId(Arc::from(id)))
                    .and_then(components::sidebar::best_name)
                    .map(|n| n.to_string())
            }),
        )
    }

    /// `#name` for channels, the plain name otherwise.
    fn conversation_label(&self, c: &ConversationId) -> String {
        match self.state.conversations.get(c) {
            Some(conv)
                if matches!(
                    conv.kind,
                    domain::ConversationKind::Channel | domain::ConversationKind::PrivateChannel
                ) =>
            {
                format!("#{}", conv.name)
            }
            Some(conv) => conv.name.to_string(),
            None => c.0.to_string(),
        }
    }

    fn person_row(m: &Member) -> SearchRow {
        let name = if m.display_name.is_empty() {
            &m.real_name
        } else {
            &m.display_name
        };
        SearchRow {
            target: SearchTarget::Person(m.id.clone()),
            conversation: "".into(),
            author: "".into(),
            time: "".into(),
            snippet: name.to_string().into(),
        }
    }

    fn conversation_row(&self, c: &domain::Conversation) -> SearchRow {
        // A conversation the list lacks is named from the provider's record.
        let snippet = if self.state.conversations.contains_key(&c.id) {
            self.conversation_label(&c.id)
        } else if matches!(
            c.kind,
            domain::ConversationKind::Channel | domain::ConversationKind::PrivateChannel
        ) {
            format!("#{}", c.name)
        } else {
            c.name.to_string()
        };
        SearchRow {
            target: SearchTarget::Conversation(c.id.clone()),
            conversation: "".into(),
            author: "".into(),
            time: "".into(),
            snippet: snippet.into(),
        }
    }

    /// Opens a conversation from a search result: one the list lacks (a
    /// channel the user is not in) is shown from the provider's record
    /// first, with Join in the composer.
    pub fn open_search_result(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        if let Some(found) = self.search_found.iter().find(|f| f.id == c).cloned() {
            self.apply(Input::Preview(found), cx);
        }
        self.open(c, cx);
    }

    /// Holds `c`, found by a browse, as a non-member when the list lacks
    /// it, so it can be shown before the user joins it.
    pub fn preview(&mut self, c: domain::Conversation, cx: &mut Context<Self>) {
        self.apply(Input::Preview(c), cx);
    }

    /// A public channel the user is not in, which the provider can join.
    pub fn must_join(&self, c: &ConversationId) -> bool {
        self.state.caps.channel_admin
            && self.state.conversations.get(c).is_some_and(|c| {
                !c.is_member && !c.is_archived && c.kind == domain::ConversationKind::Channel
            })
    }

    /// Searches the workspace. Names on the device answer at once (and are
    /// all there is offline or without server search); the provider's server
    /// answers below them. A blank query clears the results and never
    /// reaches the server. Answers that arrive after a newer search started
    /// are dropped.
    pub fn search(&mut self, scope: SearchScope, input: String, cx: &mut Context<Self>) {
        const LIMIT: usize = 50;
        self.search_seq += 1;
        let seq = self.search_seq;
        self.search_local.clear();
        self.search_context.clear();
        self.search_next = None;
        let input = input.trim().to_string();
        if input.is_empty() {
            self.search_results = SearchResults::default();
            self.search_found.clear();
            cx.notify();
            return;
        }
        let parsed = domain::parse_query(&input);
        let (q, warnings) = domain::resolve_query(&parsed, &self.state);
        let text = if parsed.text.is_empty() {
            input.clone()
        } else {
            parsed.text
        };
        let can_remote = self.provider.capabilities().search;
        let offline = can_remote && self.state.connection == domain::Connection::Offline;
        let ask_remote = can_remote && !offline;
        let query: Arc<str> = Arc::from(input.as_str());
        let (st, ws) = (self.store.clone(), self.state.workspace.clone());
        let mut base = SearchResults {
            query,
            scope,
            warnings,
            offline,
            pending: ask_remote,
            ..Default::default()
        };
        match scope {
            SearchScope::Messages => {
                let local_ok = domain::local_supported(&q);
                let lq = q.clone();
                self.run(
                    cx,
                    async move {
                        if !local_ok {
                            return Vec::new();
                        }
                        let hits = st
                            .search_filtered(&ws, &lq, LIMIT)
                            .await
                            .unwrap_or_default();
                        let mut found = Vec::with_capacity(hits.len());
                        for h in hits {
                            if let Ok(Some(m)) = st.message(&ws, &h.id.conversation, &h.id.ts).await
                            {
                                found.push(MessageHit {
                                    message: m,
                                    snippet: h.snippet,
                                    source: HitSource::Local,
                                });
                            }
                        }
                        found
                    },
                    move |s, found, cx| {
                        if s.search_seq != seq {
                            return;
                        }
                        base.rows = found
                            .iter()
                            .map(|h| s.hit_row(&h.message, &h.snippet, cx))
                            .collect();
                        s.search_local = found;
                        s.search_results = base;
                        if ask_remote {
                            s.search_remote_messages(seq, q, None, cx);
                        }
                        cx.notify();
                    },
                );
            }
            SearchScope::Files => {
                self.search_results = base;
                if ask_remote {
                    let p = self.provider.clone();
                    self.run(
                        cx,
                        async move { p.search_files(&q, None, LIMIT as u32).await },
                        move |s, r, cx| {
                            if s.search_seq != seq {
                                return;
                            }
                            s.search_results.pending = false;
                            match r {
                                Ok(page) => {
                                    s.search_results.remote = page
                                        .items
                                        .into_iter()
                                        .filter_map(|h| {
                                            let (c, t) = (h.conversation?, h.ts?);
                                            Some(SearchRow::message(
                                                MessageId {
                                                    conversation: c.clone(),
                                                    ts: t.clone(),
                                                },
                                                s.conversation_label(&c),
                                                h.author
                                                    .as_ref()
                                                    .and_then(|u| s.state.members.get(u))
                                                    .map(|m| m.display_name.to_string())
                                                    .unwrap_or_default(),
                                                format_date_time(
                                                    &t,
                                                    s.settings(cx).messages.clock,
                                                    &jiff::tz::TimeZone::system(),
                                                    &i18n::gpui::localizer(cx),
                                                ),
                                                h.file
                                                    .title
                                                    .as_deref()
                                                    .unwrap_or(&h.file.name)
                                                    .to_string(),
                                            ))
                                        })
                                        .collect();
                                }
                                Err(_) => s.search_results.remote_error = true,
                            }
                            cx.notify();
                        },
                    );
                }
            }
            SearchScope::People => {
                let needle = text.to_lowercase();
                let mut local: Vec<&Member> = self
                    .state
                    .members
                    .values()
                    .filter(|m| {
                        !m.deleted
                            && (m.display_name.to_lowercase().contains(&needle)
                                || m.real_name.to_lowercase().contains(&needle))
                    })
                    .collect();
                local.sort_by(|a, b| a.display_name.cmp(&b.display_name));
                let known: HashSet<UserId> = local.iter().map(|m| m.id.clone()).collect();
                base.rows = local
                    .into_iter()
                    .take(LIMIT)
                    .map(Self::person_row)
                    .collect();
                self.search_results = base;
                if ask_remote {
                    let p = self.provider.clone();
                    self.run(
                        cx,
                        async move { p.search_people(&text, LIMIT as u32).await },
                        move |s, r, cx| {
                            if s.search_seq != seq {
                                return;
                            }
                            s.search_results.pending = false;
                            match r {
                                Ok(found) => {
                                    s.search_results.remote = found
                                        .iter()
                                        .filter(|m| !known.contains(&m.id))
                                        .map(Self::person_row)
                                        .collect();
                                }
                                Err(_) => s.search_results.remote_error = true,
                            }
                            cx.notify();
                        },
                    );
                }
            }
            SearchScope::Conversations => {
                let needle = text.to_lowercase();
                let mut local: Vec<&domain::Conversation> = self
                    .state
                    .conversations
                    .values()
                    .filter(|c| c.name.to_lowercase().contains(&needle))
                    .collect();
                local.sort_by(|a, b| a.name.cmp(&b.name));
                let known: HashSet<ConversationId> = local.iter().map(|c| c.id.clone()).collect();
                base.rows = local
                    .into_iter()
                    .take(LIMIT)
                    .map(|c| self.conversation_row(c))
                    .collect();
                self.search_results = base;
                if ask_remote {
                    let p = self.provider.clone();
                    self.run(
                        cx,
                        async move { p.search_conversations(&text, LIMIT as u32).await },
                        move |s, r, cx| {
                            if s.search_seq != seq {
                                return;
                            }
                            s.search_results.pending = false;
                            match r {
                                Ok(found) => {
                                    let found: Vec<domain::Conversation> = found
                                        .into_iter()
                                        .filter(|c| !known.contains(&c.id))
                                        .collect();
                                    s.search_results.remote =
                                        found.iter().map(|c| s.conversation_row(c)).collect();
                                    s.search_found = found;
                                }
                                Err(_) => s.search_results.remote_error = true,
                            }
                            cx.notify();
                        },
                    );
                }
            }
        }
        cx.notify();
    }

    /// Asks the server for a page of message hits (`cursor` is `None` for the first).
    fn search_remote_messages(
        &mut self,
        seq: u64,
        q: SearchQuery,
        cursor: Option<Arc<str>>,
        cx: &mut Context<Self>,
    ) {
        const PAGE: u32 = 20;
        let p = self.provider.clone();
        let ask = q.clone();
        self.run(
            cx,
            async move { p.search_messages(&ask, cursor, PAGE).await },
            move |s, r, cx| {
                if s.search_seq != seq {
                    return;
                }
                s.search_results.pending = false;
                match r {
                    Ok(page) => {
                        // Hits already shown (device, earlier pages) are not repeated.
                        let before = s.search_local.len();
                        let merged =
                            domain::merge_hits(std::mem::take(&mut s.search_local), page.items);
                        let rows: Vec<SearchRow> = merged[before..]
                            .iter()
                            .map(|h| s.hit_row(&h.message, &h.snippet, cx))
                            .collect();
                        s.search_local = merged;
                        s.search_results.remote.extend(rows);
                        s.search_results.more = page.next.is_some();
                        s.search_next = page.next.map(|c| (q, c));
                    }
                    Err(_) => s.search_results.remote_error = true,
                }
                cx.notify();
            },
        );
    }

    /// Asks the server for the next page of the current message search.
    pub fn search_more(&mut self, cx: &mut Context<Self>) {
        if let Some((q, cursor)) = self.search_next.take() {
            self.search_results.pending = true;
            self.search_results.more = false;
            let seq = self.search_seq;
            self.search_remote_messages(seq, q, Some(cursor), cx);
            cx.notify();
        }
    }

    pub fn state(&self) -> &WorkspaceState {
        &self.state
    }

    pub fn selected(&self) -> Option<&ConversationId> {
        self.selected.as_ref()
    }

    /// Sets how composed text becomes the provider's markup before it is sent.
    pub fn set_encoder(&mut self, encoder: Arc<dyn domain::ContentEncoder>) {
        self.encoder = encoder;
    }

    /// Records a use of `e` (a picked completion or a reaction): it leads the Recent list
    /// and the store keeps it for the next start.
    pub fn emoji_used(&mut self, e: EmojiRef, cx: &mut Context<Self>) {
        self.recent_emoji.retain(|r| r.shortcode != e.shortcode);
        self.recent_emoji.insert(0, e.clone());
        self.recent_emoji.truncate(store::RECENT_EMOJI_CAP);
        let at_ms = components::now(cx).as_millisecond();
        let (st, ws) = (self.store.clone(), self.state.workspace.clone());
        self.run(
            cx,
            async move {
                st.emoji_used(&ws, e, at_ms).await.ok();
            },
            |_, _, _| {},
        );
    }

    /// Emoji used before in this workspace, newest first.
    pub fn recent_emoji(&self) -> Vec<EmojiRef> {
        self.recent_emoji.clone()
    }

    /// Reads the persisted Recent list; picks made before it arrives stay in front.
    fn load_recent_emoji(&self, cx: &mut Context<Self>) {
        let (st, ws) = (self.store.clone(), self.state.workspace.clone());
        self.run(
            cx,
            async move {
                st.recent_emoji(&ws, store::RECENT_EMOJI_CAP)
                    .await
                    .unwrap_or_default()
            },
            |s, stored, _| {
                let mut list = std::mem::take(&mut s.recent_emoji);
                for e in stored {
                    if !list.iter().any(|r| r.shortcode == e.shortcode) {
                        list.push(e);
                    }
                }
                list.truncate(store::RECENT_EMOJI_CAP);
                s.recent_emoji = list;
            },
        );
    }

    /// Sets the provider's markup parser and display name for message rendering.
    pub fn set_content(&mut self, parser: Arc<dyn ContentParser>, provider_name: String) {
        self.content = parser;
        self.provider_name = provider_name;
    }

    pub fn open(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        self.mark_opened(&c);
        self.persist_opened(&c, cx);
        self.apply(Input::Open(c), cx);
        cx.notify();
    }

    /// Opens `c` for loading and read state without changing the main window's
    /// selection (used by pop-out windows).
    pub fn open_unfocused(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        self.persist_opened(&c, cx);
        self.apply(Input::Open(c), cx);
        cx.notify();
    }

    /// Shows the profile of `user` in the right panel (the window opens it).
    pub fn show_profile(&mut self, user: UserId, cx: &mut Context<Self>) {
        self.apply(Input::OpenProfile(user), cx);
        cx.emit(SessionEvent::ShowProfile);
        cx.notify();
    }

    /// Closes the profile panel's subject.
    pub fn hide_profile(&mut self, cx: &mut Context<Self>) {
        self.apply(Input::CloseProfile, cx);
    }

    /// A channel admin write (create, join, leave, archive, rename, topic,
    /// description, invite); the answer comes back as
    /// `SessionEvent::AdminFinished`.
    pub fn admin(&mut self, call: domain::AdminCall, cx: &mut Context<Self>) {
        self.apply(Input::Admin(call), cx);
    }

    /// Joins channel `c` once: a second ask while the first is on its way
    /// sends nothing.
    pub fn join(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        if !self.can_admin() || !self.joining.insert(c.clone()) {
            return;
        }
        cx.notify();
        self.admin(domain::AdminCall::Join(c), cx);
    }

    /// The join of `c` answered (either way).
    pub(crate) fn join_finished(&mut self, c: &ConversationId) {
        self.joining.remove(c);
    }

    /// Whether the provider has channel admin (create, browse, join, ...).
    pub fn can_admin(&self) -> bool {
        self.state.caps.channel_admin
    }

    /// The provider's rule for a typed channel name, for the dialogs.
    pub fn name_check(&self) -> components::NameCheck {
        let p = self.provider.clone();
        std::rc::Rc::new(move |typed: &str| p.check_channel_name(typed))
    }

    /// One page of the channel browser; `then` gets the answer (or why not,
    /// in the active language). Aborting the returned handle drops the
    /// request (a newer query replaced it); `then` is not called then.
    pub fn browse_channels(
        &self,
        query: String,
        cursor: Option<std::sync::Arc<str>>,
        cx: &mut Context<Self>,
        then: impl FnOnce(
            &mut Self,
            Result<domain::Page<domain::Conversation>, String>,
            &mut Context<Self>,
        ) + 'static,
    ) -> futures::future::AbortHandle {
        let p = self.provider.clone();
        let (fut, abort) = futures::future::abortable(async move {
            p.browse_channels(&query, cursor, BROWSE_PAGE).await
        });
        self.run(cx, fut, move |s, r, cx| {
            let Ok(r) = r else { return };
            let r = r.map_err(|e| {
                i18n::gpui::localizer(cx)
                    .text(&crate::error_text::provider_error(&e))
                    .into_owned()
            });
            then(s, r, cx);
        });
        abort
    }

    /// Everyone in a channel, from the provider (all pages), for the invite
    /// picker; `then` gets them or why not.
    pub fn conversation_members(
        &self,
        c: ConversationId,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Result<Vec<UserId>, String>, &mut Context<Self>) + 'static,
    ) {
        let p = self.provider.clone();
        self.run(
            cx,
            async move { p.conversation_members(&c).await },
            move |s, r, cx| {
                let r = r.map_err(|e| {
                    i18n::gpui::localizer(cx)
                        .text(&crate::error_text::provider_error(&e))
                        .into_owned()
                });
                then(s, r, cx);
            },
        );
    }

    /// The sidebar row after `c` (or before it, at the end): where the
    /// person goes when the open conversation goes away.
    fn neighbour_of(&self, c: &ConversationId, cx: &App) -> Option<ConversationId> {
        self.ensure_rows(cx);
        let cache = self.sidebar_cache.borrow();
        let ids: &[ConversationId] = match &cache.rows {
            Some((_, _, ids)) => ids,
            None => &[],
        };
        let at = ids.iter().position(|i| i == c).unwrap_or(ids.len());
        ids[at.min(ids.len())..]
            .iter()
            .chain(ids[..at.min(ids.len())].iter().rev())
            .find(|i| *i != c)
            .cloned()
    }

    /// Opens the direct message with `user`, when there is one.
    pub fn open_direct_now(&mut self, user: UserId, cx: &mut Context<Self>) {
        let dm = self
            .state
            .conversations
            .values()
            .find(|c| c.kind == domain::ConversationKind::Direct && c.peer.as_ref() == Some(&user))
            .map(|c| c.id.clone());
        match dm {
            Some(id) => self.open(id, cx),
            // None yet: the provider opens it; the answer navigates (`feed_back`).
            None => self.apply(Input::OpenDirect(user), cx),
        }
    }

    /// Shows the conversation of `id` and brings the message into view,
    /// without loading the conversation's newest page first (that would
    /// race the detached window the jump opens).
    pub fn open_at(&mut self, id: MessageId, cx: &mut Context<Self>) {
        if self.mark_opened(&id.conversation) {
            self.persist_opened(&id.conversation, cx);
        }
        self.apply(Input::JumpTo(id), cx);
        cx.notify();
    }

    /// Makes `c` the shown conversation (with its unread marker) and returns
    /// whether the selection moved.
    fn mark_opened(&mut self, c: &ConversationId) -> bool {
        if self.selected.as_ref() == Some(c) {
            return false;
        }
        self.opened_marker = Some((
            c.clone(),
            self.state
                .conversations
                .get(c)
                .and_then(|conv| conv.unread.then(|| conv.last_read.clone()).flatten()),
        ));
        self.pinned_unread = self
            .state
            .conversations
            .get(c)
            .filter(|conv| conv.unread && !conv.is_muted)
            .map(|conv| conv.id.clone());
        self.selected = Some(c.clone());
        self.sidebar_cache.borrow_mut().rows = None;
        true
    }

    /// Records that `c` was opened: retention keeps what was opened lately (the
    /// reducer does not emit this).
    fn persist_opened(&mut self, c: &ConversationId, cx: &mut Context<Self>) {
        let at_ms = components::now(cx).as_millisecond();
        self.run_effect(
            Effect::Persist(domain::PersistOp::Opened {
                conversation: c.clone(),
                at_ms,
            }),
            cx,
        );
    }

    /// Opens a message that may be far from the live end (search, pins,
    /// saved items, notification clicks).
    pub fn jump_to(&mut self, id: MessageId, cx: &mut Context<Self>) {
        self.apply(Input::JumpTo(id), cx);
        cx.notify();
    }

    /// Leaves a detached window for the newest messages.
    pub fn jump_to_latest(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        self.apply(Input::JumpToLatest(c), cx);
        cx.notify();
    }

    pub fn send(&mut self, c: ConversationId, text: String, cx: &mut Context<Self>) {
        let (now, now_ms) = now_pair(cx);
        let input = Input::Send {
            conversation: c,
            text: Arc::from(text),
            thread: None,
            id: new_client_id(cx),
            now,
            now_ms,
        };
        self.apply(input, cx);
    }

    /// The composer path for an MCP send: an optimistic row, the outbox and
    /// echo de-duplication, resolving when the reducer finishes that
    /// `ClientMsgId` (`Ok` is the delivered message, `Err` the provider's
    /// failure). While the workspace is offline the item stays queued in the
    /// outbox and the task resolves `Err(Network("queued offline"))` at once.
    pub fn send_and_wait(
        &mut self,
        c: ConversationId,
        thread: Option<MessageTs>,
        broadcast: bool,
        text: String,
        id: Option<ClientMsgId>,
        cx: &mut Context<Self>,
    ) -> Task<Result<Message, ProviderError>> {
        let id = id.unwrap_or_else(|| new_client_id(cx));
        let (tx, rx) = futures::channel::oneshot::channel();
        let (now, now_ms) = now_pair(cx);
        let text: Arc<str> = Arc::from(text);
        let input = match thread {
            Some(ts) => Input::Reply {
                root: MessageId {
                    conversation: c,
                    ts,
                },
                text,
                broadcast,
                id: id.clone(),
                now,
                now_ms,
            },
            None => Input::Send {
                conversation: c,
                text,
                thread: None,
                id: id.clone(),
                now,
                now_ms,
            },
        };
        self.send_waiters.insert(id.clone(), tx);
        self.apply(input, cx);
        if self.state.connection != domain::Connection::Online {
            self.send_waiters.remove(&id);
            return Task::ready(Err(ProviderError::Network(QUEUED_OFFLINE.into())));
        }
        cx.background_spawn(async move {
            rx.await
                .unwrap_or_else(|_| Err(ProviderError::Other("stopped".into())))
        })
    }

    pub fn edit(&mut self, id: MessageId, text: String, cx: &mut Context<Self>) {
        self.apply(
            Input::Edit {
                id,
                text: Arc::from(text),
            },
            cx,
        );
    }

    pub fn delete(&mut self, id: MessageId, cx: &mut Context<Self>) {
        self.apply(Input::Delete(id), cx);
    }

    /// Adds or removes the current user's reaction (optimistic; the reducer rolls back on failure).
    pub fn react(&mut self, id: MessageId, emoji: EmojiRef, add: bool, cx: &mut Context<Self>) {
        if add {
            self.emoji_used(emoji.clone(), cx);
        }
        self.apply(Input::React { id, emoji, add }, cx);
    }

    /// Opens (and starts loading) the thread under `root`.
    pub fn open_thread(&mut self, root: MessageId, cx: &mut Context<Self>) {
        self.apply(Input::OpenThread(root), cx);
    }

    pub fn close_thread(&mut self, root: MessageId, cx: &mut Context<Self>) {
        self.apply(Input::CloseThread(root.clone()), cx);
        // The error goes with the thread entry, not with one of its viewers.
        if !self.state.threads.contains_key(&root) {
            self.replies_failed.remove(&root);
        }
    }

    /// Marks the thread under `root` read up to `ts` (the notification's
    /// "Mark as read" on a thread message).
    pub fn mark_thread_read(&mut self, root: MessageId, ts: MessageTs, cx: &mut Context<Self>) {
        self.run_effect(Effect::MarkThreadRead { root, ts }, cx);
    }

    /// Replies in the thread under `root`; `broadcast` also posts it to the conversation.
    pub fn reply(
        &mut self,
        root: MessageId,
        text: String,
        broadcast: bool,
        cx: &mut Context<Self>,
    ) {
        let (now, now_ms) = now_pair(cx);
        let input = Input::Reply {
            root,
            text: Arc::from(text),
            broadcast,
            id: new_client_id(cx),
            now,
            now_ms,
        };
        self.apply(input, cx);
    }

    /// Fetches the page before the oldest loaded message, once at a time.
    pub fn load_older(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        let Some(w) = self.state.windows.get(&c) else {
            return;
        };
        if !w.has_more_before() || self.loading_older.contains(&c) {
            return;
        }
        let Some(before) = w.messages().first().map(|m| m.id.ts.clone()) else {
            return;
        };
        self.loading_older.insert(c.clone());
        let (st, ws) = (self.store.clone(), self.state.workspace.clone());
        let (conv, oldest) = (c.clone(), before);
        let lost_c = c.clone();
        self.run_or(
            cx,
            async move {
                let gaps = st.gaps(&ws, &conv).await.unwrap_or_default();
                let has_older = st
                    .messages_before(&ws, &conv, Some(oldest.clone()), 1)
                    .await
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                match sync::next_older(&oldest, has_older, &gaps) {
                    Older::Store => {
                        let mut v: Vec<Message> = st
                            .messages_before(&ws, &conv, Some(oldest), HISTORY_PAGE as usize)
                            .await
                            .unwrap_or_default();
                        v.reverse();
                        (Older::Store, Some(v))
                    }
                    o => (o, None),
                }
            },
            move |s, (older, stored), cx| match (older, stored) {
                (Older::Store, Some(messages)) => {
                    s.loading_older.remove(&c);
                    s.apply(
                        Input::HistoryLoaded {
                            conversation: c,
                            page: HistoryPage {
                                messages,
                                has_more: true,
                            },
                        },
                        cx,
                    );
                }
                (
                    Older::Remote {
                        after: None,
                        before,
                    },
                    _,
                ) => s.fetch_history(c, Some(before), cx),
                (Older::Remote { after, before }, _) => {
                    s.loading_older.remove(&c);
                    s.run_effect(
                        Effect::FetchBetween {
                            conversation: c,
                            after,
                            before,
                        },
                        cx,
                    );
                }
                (Older::Store, None) => {
                    s.loading_older.remove(&c);
                }
            },
            move |s, _| {
                s.loading_older.remove(&lost_c);
            },
        );
    }

    fn fetch_history(
        &mut self,
        c: ConversationId,
        before: Option<MessageTs>,
        cx: &mut Context<Self>,
    ) {
        let p = self.provider.clone();
        let conv = c.clone();
        let first_page = before.is_none();
        if first_page {
            // One first page at a time; asking again shows the loading state.
            if !self.history_pending.insert(c.clone()) {
                return;
            }
            self.history_failed.remove(&c);
            cx.notify();
        }
        self.run(
            cx,
            async move { p.history(&conv, before, HISTORY_PAGE).await },
            move |s, r, cx| {
                s.loading_older.remove(&c);
                if first_page {
                    s.history_pending.remove(&c);
                }
                match r {
                    Ok(page) => {
                        s.history_failed.remove(&c);
                        s.apply(
                            Input::HistoryLoaded {
                                conversation: c,
                                page,
                            },
                            cx,
                        )
                    }
                    // One scrubbed line (no ids, no content) says why; a failed
                    // first page also shows the timeline's Retry state.
                    Err(e) => {
                        crate::log::warn(&format!("history load failed: {}", error_kind(&e)));
                        if first_page {
                            s.history_failed.insert(c);
                            cx.notify();
                        }
                    }
                }
            },
        );
    }

    /// The first history page of `c` failed to load and has not been retried.
    pub fn history_failed(&self, c: &ConversationId) -> bool {
        self.history_failed.contains(c)
    }

    /// Asks the provider for the first history page of `c` again.
    pub fn retry_history(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        if self.history_failed.contains(&c) {
            self.fetch_history(c, None, cx);
        }
    }

    /// The replies of the thread under `root` failed to load and have not been retried.
    pub fn replies_failed(&self, root: &MessageId) -> bool {
        self.replies_failed.contains(root)
    }

    /// Asks the provider for the replies of `root` again.
    pub fn retry_replies(&mut self, root: MessageId, cx: &mut Context<Self>) {
        if self.replies_failed.contains(&root) {
            let after = self
                .state
                .threads
                .get(&root)
                .and_then(|t| t.replies.newest_delivered_ts().cloned());
            self.run_effect(Effect::FetchReplies { root, after }, cx);
        }
    }

    pub(crate) fn apply(&mut self, input: Input, cx: &mut Context<Self>) {
        // The one member this input can change, for the People directory.
        self.member_touch = match &input {
            Input::Provider(ProviderEvent::MemberUpdated(m)) => Some(m.id.clone()),
            Input::Loaded(Ok(domain::Loaded::Profile(p))) => Some(p.user.clone()),
            Input::SetStatus(_) => self.state.self_user.clone(),
            _ => None,
        };
        // A reconnect after the first bootstrap catches up; the very first
        // connect is covered by the sync that follows bootstrap.
        if self.bootstrapped
            && matches!(
                input,
                Input::Provider(ProviderEvent::Connected { reconnect: true })
            )
        {
            self.sync(cx);
            // A reconnect is how a wake from sleep shows up: check the schedules.
            self.local_tick(cx);
        }
        match &input {
            Input::Provider(ProviderEvent::Connected { reconnect }) => {
                crate::log::info(&format!("realtime connected (reconnect: {reconnect})"));
            }
            Input::Provider(ProviderEvent::Disconnected) => {
                crate::log::info("realtime disconnected");
            }
            // Left or archived: nothing staged for it can be sent any more.
            Input::Provider(ProviderEvent::ConversationMetaChanged(meta))
                if meta.is_archived == Some(true) || meta.is_member == Some(false) =>
            {
                self.prune_conversation(&meta.id, cx);
            }
            _ => {}
        }
        if let Input::Provider(ProviderEvent::MessagePosted(m)) = &input {
            self.live_post = Some(m.clone());
        }
        // The open conversation is left or archived (here or elsewhere): the
        // next sidebar row opens instead, or none.
        let gone = match &input {
            Input::Provider(ProviderEvent::ConversationMetaChanged(m))
                if m.is_archived == Some(true) || m.is_member == Some(false) =>
            {
                Some(m.id.clone())
            }
            Input::CallFinished {
                call:
                    domain::ProviderCall::Admin(
                        domain::AdminCall::Leave(c) | domain::AdminCall::Archive(c),
                    ),
                result: Ok(_),
            } => Some(c.clone()),
            _ => None,
        }
        .filter(|c| self.selected.as_ref() == Some(c))
        .map(|c| {
            let next = self.neighbour_of(&c, cx);
            (c, next)
        });
        // One member's update patches the cached index; only a name or handle
        // change drops the rows (a profile or presence update costs nothing).
        if let Input::Provider(ProviderEvent::MemberUpdated(m)) = &input {
            self.patch_sidebar_member(m);
        }
        if matches!(input, Input::Provider(ProviderEvent::HuddleRing { .. })) {
            self.sync_ring_setting(cx);
        }
        // One wall and one monotonic read per batch, no timer.
        if matches!(input, Input::Provider(_)) {
            let unix = components::now(cx).as_second();
            let fx = reduce(
                &mut self.state,
                Input::Clock {
                    unix,
                    mono: std::time::Instant::now(),
                },
            );
            for e in fx {
                self.run_effect(e, cx);
            }
        }
        match &input {
            Input::Send { id, .. } | Input::Reply { id, .. } => self.send_timing.enqueued(id),
            Input::Provider(ProviderEvent::MessagePosted(m)) => {
                if let Some(line) = m
                    .client_msg_id
                    .as_ref()
                    .and_then(|id| self.send_timing.echo(id))
                {
                    crate::log::info(&line);
                }
            }
            Input::SendFinished { id, result, .. } => {
                if let Some(line) = self.send_timing.finished(id, result) {
                    crate::log::info(&line);
                }
            }
            _ => {}
        }
        let finished = match &input {
            Input::SendFinished { id, result, .. } if self.send_waiters.contains_key(id) => {
                Some((id.clone(), result.clone()))
            }
            _ => None,
        };
        let effects = reduce(&mut self.state, input);
        if let Some((id, result)) = finished
            && let Some(w) = self.send_waiters.remove(&id)
        {
            let _ = w.send(result);
        }
        for e in effects {
            self.run_effect(e, cx);
        }
        self.member_patched = false;
        self.member_touch = None;
        self.live_post = None;
        self.sync_presence_watch(cx);
        if let Some((c, next)) = gone
            && self.selected.as_ref() == Some(&c)
        {
            match next {
                Some(n) => self.open(n, cx),
                None => {
                    self.selected = None;
                    self.sidebar_cache.borrow_mut().rows = None;
                    cx.notify();
                }
            }
        }
    }

    /// Fills the sidebar row cache when it is empty or its key is stale.
    fn ensure_rows(&self, cx: &App) {
        let settings = self.settings(cx);
        let l = i18n::Translate::localizer(cx);
        let mut cache = self.sidebar_cache.borrow_mut();
        // Compared in place: the key is cloned only when the rows rebuild.
        if let Some((k, _, _)) = &cache.rows
            && k.0 == settings.sidebar
            && k.1 == *l.locale()
        {
            return;
        }
        let key = (settings.sidebar.clone(), l.locale().clone());
        let index = Self::index_of(&mut cache, &self.state);
        let rows = components::build_rows_sections(
            &self.state,
            &index,
            self.selected.as_ref(),
            self.pinned_unread.as_ref(),
            &key.0,
            &self.draft_conversations(),
            l,
            self.dms_expanded,
            &self.collapsed,
        );
        let ids = rows
            .iter()
            .filter_map(|r| match r {
                SidebarRow::Conversation { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        cache.rows = Some((key, rows, ids));
    }

    /// The people on screen: the peers of the direct-message rows shown, the
    /// open conversation's peer and the open profile (never ourselves).
    fn visible_people(&self, cx: &App) -> Vec<UserId> {
        let mut set = std::collections::BTreeSet::new();
        let peer = |id: &ConversationId| {
            self.state
                .conversations
                .get(id)
                .filter(|c| c.kind == domain::ConversationKind::Direct)
                .and_then(|c| c.peer.clone())
        };
        self.ensure_rows(cx);
        if let Some((_, _, ids)) = &self.sidebar_cache.borrow().rows {
            for id in ids {
                set.extend(peer(id));
            }
        }
        set.extend(self.selected.as_ref().and_then(peer));
        set.extend(self.state.open_profile.clone());
        set.extend(self.directory_people.iter().cloned());
        set.retain(|u| self.state.self_user.as_ref() != Some(u));
        set.into_iter().collect()
    }

    /// Tells the provider who is on screen, once the set settles. No timer runs
    /// unless the set changed, and none while it stays the same.
    fn sync_presence_watch(&mut self, cx: &mut Context<Self>) {
        if !self.state.caps.presence_watch || !self.bootstrapped {
            return;
        }
        let want = self.visible_people(cx);
        if want == self.presence_desired {
            return;
        }
        self.presence_desired = want;
        if self.presence_timer {
            return;
        }
        self.presence_timer = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PRESENCE_DEBOUNCE).await;
            this.update(cx, |s, cx| {
                s.presence_timer = false;
                let users = s.presence_desired.clone();
                s.apply(Input::WatchPresence(users), cx);
            })
            .ok();
        })
        .detach();
    }

    /// Asks `domain::notify::decide` about a live post and shows the result.
    fn maybe_notify(&self, conversation: &ConversationId, message: &MessageId, cx: &mut App) {
        let Some(hub) = cx.try_global::<Notifications>().cloned() else {
            return;
        };
        let Some(m) = self.live_post.as_ref().filter(|m| m.id == *message) else {
            return;
        };
        let (Some(conv), Some(me)) = (
            self.state.conversations.get(conversation),
            self.state.self_user.as_ref(),
        ) else {
            return;
        };
        let settings = self.settings(cx).notifications.clone();
        let author = m
            .author
            .as_ref()
            .and_then(|u| self.state.members.get(u))
            .map(|a| a.display_name.to_string())
            .unwrap_or_default();
        let now_ts = components::now(cx);
        let local = now_ts.to_zoned_datetime(jiff::tz::TimeZone::system());
        let ctx = NotifyContext {
            settings: &settings,
            self_user: me,
            now: local.time(),
            weekday: local.weekday(),
            now_unix: now_ts.as_second(),
            dnd: self.state.dnd.get(me),
            prefs: self.state.prefs_for(conversation),
            extras: self.state.local_notify_extras.get(conversation),
            thread_started_by_me: m.thread_root.as_ref().is_some_and(|root| {
                let id = domain::MessageId {
                    conversation: conversation.clone(),
                    ts: root.clone(),
                };
                let root_msg = self
                    .state
                    .threads
                    .get(&id)
                    .and_then(|t| t.root.as_ref())
                    .or_else(|| {
                        self.state
                            .windows
                            .get(conversation)
                            .and_then(|w| w.messages().iter().find(|x| x.id == id))
                    });
                root_msg.is_some_and(|r| r.author.as_ref() == Some(me))
            }),
            thread_followed: m.thread_root.as_ref().is_some_and(|root| {
                self.state.thread_followed(&domain::MessageId {
                    conversation: conversation.clone(),
                    ts: root.clone(),
                })
            }),
            focused: self.selected.as_ref(),
            window_active: cx.active_window().is_some(),
            user_groups: Some(&self.state.user_groups),
        };
        if let Some(n) = decide(&ctx, &self.state.workspace, conv, m, &author) {
            hub.show(n, cx);
        }
    }

    /// Posts one native notification when this workspace's session expires.
    /// Event-driven, like `MaybeNotify`: no timers and no polling. The hub
    /// drops repeats of the same expiry until the workspace signs in again.
    fn notify_session_expired(&self, cx: &mut App) {
        let Some(hub) = cx.try_global::<Notifications>().cloned() else {
            return;
        };
        let notice = Notifications::expired_notice(&self.state.workspace, &self.state.name, cx);
        hub.session_expired(notice, cx);
    }

    /// After the conversation list changed: update the badge and dismiss
    /// notifications for conversations that are read now (here or elsewhere).
    fn sync_notifications(&self, cx: &mut App) {
        let Some(hub) = cx.try_global::<Notifications>().cloned() else {
            return;
        };
        let ws = &self.state.workspace;
        hub.dismiss_read(
            ws,
            |c| {
                // A reminder alert is no conversation: reading never clears it.
                !c.0.starts_with(crate::reminders::REMINDER_PREFIX)
                    && self
                        .state
                        .conversations
                        .get(c)
                        .is_none_or(|c| !c.unread && c.mentions == 0)
            },
            cx,
        );
        hub.set_mentions(ws, self.state.totals().mentions, cx);
    }

    /// Runs one effect as the reducer's output would be. For tests of the
    /// effects the feature tasks have not wired into a reducer rule yet.
    #[doc(hidden)]
    pub fn run_effect_now(&mut self, e: Effect, cx: &mut Context<Self>) {
        self.run_effect(e, cx);
    }

    /// The latest answer fed back for an `Effect::Call` or `Effect::Load`.
    #[doc(hidden)]
    #[cfg(feature = "fixture")]
    pub fn last_feedback(&self) -> Option<&Input> {
        self.last_feedback.as_ref()
    }

    fn run_effect(&mut self, e: Effect, cx: &mut Context<Self>) {
        match e {
            Effect::Call(call) => self.run_call(call, cx),
            Effect::Load(load) => self.run_load(load, cx),
            Effect::Persist(op) => {
                let store = self.store.clone();
                let ws = self.state.workspace.clone();
                self.run(cx, async move { store.apply(&ws, op).await }, |_, r, _| {
                    if r.is_err() {
                        crate::log::warn("store persist failed");
                    }
                });
                self.check_store_usage(cx);
            }
            Effect::FetchHistory {
                conversation,
                before,
            } => self.fetch_history(conversation, before, cx),
            Effect::FetchAround { target } => {
                let p = self.provider.clone();
                let (st, ws) = (self.store.clone(), self.state.workspace.clone());
                let t = target.clone();
                self.run(
                    cx,
                    async move {
                        // The store first: when it holds a page's worth on both
                        // sides of the target the provider is not asked. A
                        // partial page still serves when the provider cannot
                        // answer (offline).
                        let half = (HISTORY_PAGE / 2) as usize;
                        let at = st.message(&ws, &t.conversation, &t.ts).await.ok().flatten();
                        let mut held = None;
                        if let Some(at) = at {
                            let mut before = st
                                .messages_before(&ws, &t.conversation, Some(t.ts.clone()), half)
                                .await
                                .unwrap_or_default();
                            let after = st
                                .messages_after(&ws, &t.conversation, &t.ts, half)
                                .await
                                .unwrap_or_default();
                            let page_flags = (before.len() == half, after.len() == half);
                            before.reverse();
                            before.push(at);
                            before.extend(after);
                            let page = domain::AroundPage {
                                messages: before,
                                has_more_before: page_flags.0,
                                has_more_after: page_flags.1,
                            };
                            if page_flags == (true, true) {
                                return Ok(page);
                            }
                            held = Some(page);
                        }
                        match p.history_around(&t.conversation, &t.ts, HISTORY_PAGE).await {
                            Ok(page)
                                if held.is_none()
                                    || page.messages.iter().any(|m| m.id.ts == t.ts) =>
                            {
                                Ok(page)
                            }
                            Ok(_) => held.ok_or(ProviderError::Other(String::new())),
                            Err(e) => held.ok_or(e),
                        }
                    },
                    move |s, r, cx| {
                        if let Ok(page) = r {
                            s.apply(Input::AroundLoaded { target, page }, cx)
                        }
                    },
                );
            }
            Effect::FetchNewer {
                conversation,
                after,
            } => {
                let p = self.provider.clone();
                let (st, ws) = (self.store.clone(), self.state.workspace.clone());
                let (c, a) = (conversation.clone(), after);
                self.run(
                    cx,
                    async move {
                        // The store first: a full page after `a` needs no request.
                        let n = HISTORY_PAGE as usize;
                        let mut stored = st
                            .messages_after(&ws, &c, &a, n + 1)
                            .await
                            .unwrap_or_default();
                        if stored.len() > n {
                            stored.truncate(n);
                            return Ok(HistoryPage {
                                messages: stored,
                                has_more: true,
                            });
                        }
                        // `history_between` answers newest first, so the
                        // messages right after `a` are reached by paging
                        // down (bounded); keep the oldest page of them.
                        let mut all: Vec<Message> = Vec::new();
                        let mut bound: Option<MessageTs> = None;
                        for _ in 0..domain::AROUND_MAX_PAGES {
                            let page = p
                                .history_between(&c, a.clone(), bound.clone(), HISTORY_PAGE)
                                .await?;
                            bound = page.messages.iter().map(|m| m.id.ts.clone()).min();
                            let more = page.has_more && bound.is_some();
                            all.extend(page.messages);
                            if !more {
                                break;
                            }
                        }
                        all.sort_by(|x, y| x.id.ts.cmp(&y.id.ts));
                        let n = HISTORY_PAGE as usize;
                        let has_more = all.len() > n;
                        all.truncate(n);
                        Ok::<_, ProviderError>(HistoryPage {
                            messages: all,
                            has_more,
                        })
                    },
                    move |s, r, cx| {
                        if let Ok(page) = r {
                            s.apply(Input::NewerLoaded { conversation, page }, cx)
                        }
                    },
                );
            }
            Effect::Reload(conversation) => {
                self.seed_from_store(conversation.clone(), cx);
                self.fetch_history(conversation, None, cx);
            }
            Effect::FetchBetween {
                conversation,
                after,
                before,
            } => {
                let p = self.provider.clone();
                let (c, a, b) = (
                    conversation.clone(),
                    after
                        .clone()
                        .unwrap_or_else(|| MessageTs("0.000000".into())),
                    before.clone(),
                );
                self.run(
                    cx,
                    async move {
                        p.history_between(&c, a, Some(b), domain::sync::CATCH_UP_PAGE)
                            .await
                    },
                    move |s, r, cx| {
                        if let Ok(page) = r {
                            s.apply(
                                Input::GapPageLoaded {
                                    conversation,
                                    after,
                                    before,
                                    page,
                                },
                                cx,
                            )
                        }
                    },
                );
            }
            Effect::MarkRead { conversation, ts } => {
                let p = self.provider.clone();
                self.run(
                    cx,
                    async move { p.mark_read(&conversation, &ts).await },
                    |_, r, _| {
                        if let Err(e) = r {
                            crate::log::warn(&format!("mark read failed: {}", error_kind(&e)));
                        }
                    },
                );
            }
            Effect::CallSend {
                conversation,
                text,
                id,
            } => {
                let p = self.provider.clone();
                let (c, i) = (conversation.clone(), id.clone());
                self.send_timing.started(&id);
                let (lost_c, lost_id) = (conversation.clone(), id.clone());
                self.run_or(
                    cx,
                    async move { p.send(&c, &text, None, i).await },
                    move |s, r, cx| {
                        s.apply(
                            Input::SendFinished {
                                conversation,
                                id,
                                result: r,
                            },
                            cx,
                        )
                    },
                    move |s, cx| s.send_lost(lost_c, lost_id, cx),
                );
            }
            Effect::CallReply {
                root,
                text,
                broadcast,
                id,
            } => {
                let p = self.provider.clone();
                let (r, i) = (root.clone(), id.clone());
                self.send_timing.started(&id);
                let (lost_c, lost_id) = (root.conversation.clone(), id.clone());
                self.run_or(
                    cx,
                    async move { p.reply(&r.conversation, &r.ts, &text, broadcast, i).await },
                    move |s, r, cx| {
                        s.apply(
                            Input::SendFinished {
                                conversation: root.conversation,
                                id,
                                result: r,
                            },
                            cx,
                        )
                    },
                    move |s, cx| s.send_lost(lost_c, lost_id, cx),
                );
            }
            Effect::FetchReplies { root, after } => {
                // One fetch per thread at a time; a new one clears the error.
                if !self.replies_pending.insert(root.clone()) {
                    return;
                }
                self.replies_failed.remove(&root);
                cx.notify();
                let p = self.provider.clone();
                let r = root.clone();
                self.run(
                    cx,
                    async move { p.replies(&r.conversation, &r.ts, after, HISTORY_PAGE).await },
                    move |s, r, cx| match r {
                        Ok(page) => {
                            s.replies_pending.remove(&root);
                            s.replies_failed.remove(&root);
                            s.apply(Input::RepliesLoaded { root, page }, cx)
                        }
                        Err(e) => {
                            crate::log::warn(&format!(
                                "thread replies load failed: {}",
                                error_kind(&e)
                            ));
                            s.replies_pending.remove(&root);
                            // A thread closed meanwhile has no pane to show it.
                            if s.state.threads.contains_key(&root) {
                                s.replies_failed.insert(root);
                            }
                            cx.notify();
                        }
                    },
                );
            }
            Effect::CallReact { id, emoji, add } => {
                let p = self.provider.clone();
                let (i, e) = (id.clone(), emoji.clone());
                self.run(
                    cx,
                    async move { p.react(&i, &e, add).await },
                    move |s, r, cx| {
                        s.apply(
                            Input::ReactFinished {
                                id,
                                emoji,
                                add,
                                result: r,
                            },
                            cx,
                        )
                    },
                );
            }
            Effect::MarkThreadRead { root, ts } => {
                let p = self.provider.clone();
                self.run(
                    cx,
                    async move { p.mark_thread_read(&root.conversation, &root.ts, &ts).await },
                    |_, r, _| {
                        if let Err(e) = r {
                            crate::log::warn(&format!(
                                "mark thread read failed: {}",
                                error_kind(&e)
                            ));
                        }
                    },
                );
            }
            Effect::FetchCustomEmoji => {
                let p = self.provider.clone();
                self.run(cx, async move { p.custom_emoji().await }, |s, r, cx| {
                    if let Ok(all) = r {
                        s.apply(Input::CustomEmojiLoaded(all), cx)
                    }
                });
            }
            Effect::FetchUserGroups => {
                let p = self.provider.clone();
                self.run(
                    cx,
                    async move { p.user_groups().await },
                    |s, r, cx| match r {
                        Ok(all) => s.apply(Input::UserGroupsLoaded(all), cx),
                        Err(e) => crate::log::warn(&format!(
                            "user groups load failed: {}",
                            error_kind(&e)
                        )),
                    },
                );
            }
            Effect::Outbox(op) => {
                let store = self.store.clone();
                let ws = self.state.workspace.clone();
                self.run(
                    cx,
                    async move {
                        match op {
                            OutboxOp::Put(i) => store.outbox_put(&ws, i).await,
                            OutboxOp::Remove(id) => store.outbox_remove(&ws, &id).await,
                        }
                    },
                    |_, r, _| {
                        if r.is_err() {
                            crate::log::warn("outbox write failed");
                        }
                    },
                );
            }
            Effect::CallEdit { id, text } => {
                let p = self.provider.clone();
                self.run(cx, async move { p.edit(&id, &text).await }, |s, r, cx| {
                    if let Err(e) = r {
                        crate::log::warn(&format!("message edit failed: {}", error_kind(&e)));
                        // Nothing was changed locally: the message stays as it was.
                        s.call_notice(i18n::LocalizedText::new("parla", "message-edit-failed"), cx);
                    }
                });
            }
            Effect::CallDelete { id } => {
                let p = self.provider.clone();
                self.run(cx, async move { p.delete(&id).await }, |s, r, cx| {
                    if let Err(e) = r {
                        crate::log::warn(&format!("message delete failed: {}", error_kind(&e)));
                        s.call_notice(
                            i18n::LocalizedText::new("parla", "message-delete-failed"),
                            cx,
                        );
                    }
                });
            }
            Effect::MaybeNotify {
                conversation,
                message,
            } => self.maybe_notify(&conversation, &message, cx),
            Effect::SendLocal(l) => self.send_local(l, cx),
            Effect::AskLate(ids) => self.ask_late(ids, cx),
            Effect::WakeAt(at) => self.wake_at(at, cx),
            Effect::RetryOutbox { after_secs } => {
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(u64::from(after_secs)))
                        .await;
                    this.update(cx, |s, cx| s.apply(Input::OutboxRetry, cx))
                        .ok();
                })
                .detach();
            }
            Effect::ReminderDue(r) => self.alert_reminder(&r, cx),
            Effect::OpenHuddle { conversation } => {
                self.open_huddle(&conversation, cx);
            }
            Effect::StartRing(r) => self.start_ring(r, cx),
            Effect::SendTyping {
                conversation,
                thread,
            } => self.provider.typing(&conversation, thread.as_ref()),
            Effect::TypingWakeAt(at) => self.typing_wake_at(at, cx),
            Effect::StopRing {
                conversation,
                reason,
            } => self.stop_ring(&conversation, reason, cx),
            Effect::Changed(c) => {
                self.invalidate_sidebar(&c);
                if matches!(c, Changed::Members | Changed::Session) {
                    self.log_member_change(c == Changed::Members);
                }
                if matches!(c, Changed::Conversations | Changed::Dnd | Changed::Session) {
                    crate::tray::Tray::request(cx);
                }
                if c == Changed::Session && self.state.session == Session::Expired {
                    self.notify_session_expired(cx);
                }
                if c == Changed::Conversations {
                    self.sync_notifications(cx);
                }
                if c == Changed::Huddles {
                    self.sync_call(cx);
                }
                if c == Changed::NotifyPrefs {
                    self.arm_mute_wake(cx);
                }
                if c == Changed::Dnd {
                    self.arm_pause_wake(cx);
                    if !self.state.caps.reminders {
                        self.arm_reminder_wake(cx);
                    }
                }
                if c == Changed::Reminders && !self.state.caps.reminders {
                    self.arm_reminder_wake(cx);
                }
                cx.emit(SessionEvent::from(c));
                cx.notify();
            }
        }
    }

    /// The task of a send panicked or was cancelled: settle it as a network
    /// failure so the outbox frees its slot and sends the item again, instead
    /// of holding every later send behind an answer that never comes. After
    /// `MAX_LOST` losses the item fails (Retry) instead of looping.
    fn send_lost(&mut self, conversation: ConversationId, id: ClientMsgId, cx: &mut Context<Self>) {
        let result = Err(self.lost_sends.lost(&id));
        self.apply(
            Input::SendFinished {
                conversation,
                id,
                result,
            },
            cx,
        );
    }

    /// Runs `fut` on the Tokio runtime (or GPUI's executor when none is
    /// registered, as in tests) and hands the result back on the main thread.
    pub(crate) fn run<T, F>(
        &self,
        cx: &mut Context<Self>,
        fut: F,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        self.run_or(cx, fut, then, |_, _| {});
    }

    /// Like [`run`](Self::run), and `lost` runs instead of `then` when the
    /// task panicked or was cancelled, so in-flight bookkeeping can be cleared.
    pub(crate) fn run_or<T, F>(
        &self,
        cx: &mut Context<Self>,
        fut: F,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        lost: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let handle = crate::runtime::handle(cx);
        cx.spawn(async move |this, cx| {
            let out = match handle {
                Some(h) => h.spawn(fut).await.ok(),
                None => Some(fut.await),
            };
            match out {
                Some(out) => {
                    this.update(cx, |s, cx| then(s, out, cx)).ok();
                }
                None => {
                    crate::log::warn("background task lost");
                    this.update(cx, |s, cx| lost(s, cx)).ok();
                }
            }
        })
        .detach();
    }

    /// The store this session reads and writes.
    pub(crate) fn store_handle(&self) -> Arc<dyn Store> {
        self.store.clone()
    }

    /// A conversation's display name, as the sidebar shows it.
    pub(crate) fn label_of(&self, c: &ConversationId, cx: &App) -> String {
        let Some(conv) = self.state.conversations.get(c) else {
            return c.0.to_string();
        };
        let index = self
            .sidebar_cache
            .borrow_mut()
            .index
            .get_or_insert_with(|| Arc::new(MemberIndex::build(&self.state)))
            .clone();
        components::sidebar::conversation_label(
            &self.state,
            &index,
            conv,
            i18n::Translate::localizer(cx),
        )
        .to_string()
    }

    /// Source-trait methods only get `&mut App`; hop back into the entity.
    /// Arms the one typing expiry timer for `at`, replacing any earlier one;
    /// `None` cancels it. No tick: it fires once, at the earliest expiry.
    fn typing_wake_at(&mut self, at: Option<std::time::Instant>, cx: &mut Context<Self>) {
        self.typing_wake = None;
        let Some(at) = at else { return };
        let wait = at.saturating_duration_since(std::time::Instant::now());
        self.typing_wake = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |s, cx| {
                s.typing_wake = None;
                s.apply(
                    Input::TypingExpired {
                        now: std::time::Instant::now(),
                    },
                    cx,
                );
            })
            .ok();
        }));
    }

    pub(crate) fn defer_to(
        &self,
        cx: &mut App,
        f: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let this = self.this.clone();
        cx.defer(move |cx| {
            this.update(cx, f).ok();
        });
    }
}

/// The local timestamp of an optimistic row and the same instant in Unix
/// milliseconds (the outbox order and reload rows use the latter).
fn now_pair(cx: &gpui::App) -> (MessageTs, i64) {
    let us = components::now(cx).as_microsecond();
    let ts = MessageTs(Arc::from(format!(
        "{}.{:06}",
        us.div_euclid(1_000_000),
        us.rem_euclid(1_000_000)
    )));
    (ts, us.div_euclid(1000))
}

/// A fresh client message id: a lowercase UUID v4 like the web client's,
/// which the server keeps and hands back on the echo.
pub(crate) fn new_client_id(_cx: &gpui::App) -> ClientMsgId {
    ClientMsgId(Arc::from(new_uuid()))
}

fn new_uuid() -> String {
    uuid::Uuid::new_v4().hyphenated().to_string()
}

impl SidebarSource for WorkspaceSession {
    fn workspace_name(&self, _cx: &App) -> Option<gpui::SharedString> {
        let name = self.state.name.as_ref();
        (!name.is_empty()).then(|| gpui::SharedString::from(name.to_string()))
    }

    fn rows(&self, cx: &App) -> Vec<SidebarRow> {
        self.ensure_rows(cx);
        match &self.sidebar_cache.borrow().rows {
            Some((_, rows, _)) => rows.clone(),
            None => Vec::new(),
        }
    }

    fn open(&mut self, id: ConversationId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::open(s, id.clone(), cx);
            cx.emit(SessionEvent::Picked(id));
        });
    }

    fn scheduled_entry(&self) -> Option<usize> {
        let n = self.scheduled_total();
        (self.provider.capabilities().scheduled_send || n > 0).then_some(n)
    }

    fn open_scheduled(&mut self, cx: &mut App) {
        self.defer_to(cx, |_, cx| cx.emit(SessionEvent::ShowScheduled));
    }

    fn reminders_entry(&self) -> Option<usize> {
        Some(self.reminders_open())
    }

    fn open_reminders(&mut self, cx: &mut App) {
        self.defer_to(cx, |s, cx| {
            s.apply(Input::OpenReminders, cx);
            cx.emit(SessionEvent::ShowReminders);
        });
    }

    fn threads_entry(&self) -> Option<usize> {
        self.state.caps.thread_inbox.then(|| {
            self.state.thread_inbox.as_ref().map_or(0, |p| {
                p.items.iter().map(|t| t.unread_replies as usize).sum()
            })
        })
    }

    fn open_threads(&mut self, cx: &mut App) {
        self.defer_to(cx, |_, cx| cx.emit(SessionEvent::ShowThreads));
    }

    fn later_entry(&self) -> Option<usize> {
        self.state.caps.saved.then(|| {
            self.state
                .saved
                .as_ref()
                .map_or(0, |p| p.items.iter().filter(|i| !i.done).count())
        })
    }

    fn open_later(&mut self, cx: &mut App) {
        self.defer_to(cx, |_, cx| cx.emit(SessionEvent::ShowLater));
    }

    fn paused_label(&self, cx: &App) -> Option<String> {
        use components::NotifyControls as _;
        let cfg = self.send_later_settings(cx);
        let ends = self
            .can_pause()
            .then(|| self.paused(components::now_secs(cx), &cfg.zone))??;
        Some(components::paused_text(
            &ends,
            &cfg,
            i18n::Translate::localizer(cx),
        ))
    }

    fn resume_notifications(&mut self, cx: &mut App) {
        components::NotifyControls::resume(self, cx);
    }

    fn drafts_entry(&self) -> Option<usize> {
        let n = self.drafts.len();
        (n > 0).then_some(n)
    }

    fn people_entry(&self) -> bool {
        true
    }

    fn open_people(&mut self, cx: &mut App) {
        self.defer_to(cx, |_, cx| cx.emit(SessionEvent::ShowPeople));
    }

    fn open_drafts(&mut self, cx: &mut App) {
        self.defer_to(cx, |_, cx| cx.emit(SessionEvent::ShowDrafts));
    }

    fn toggle_section(&mut self, section: components::SidebarSection, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.toggle_section_now(section, cx));
    }

    fn section_op(&mut self, op: components::SectionOp, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.section_op_now(op, cx));
    }

    fn set_sidebar_option(&mut self, option: components::SidebarOption, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.sidebar_option_now(option, cx));
    }

    fn sidebar_options(&self, cx: &App) -> (bool, settings::SidebarSort) {
        WorkspaceSession::sidebar_options(self, cx)
    }

    fn toggle_direct_messages(&mut self, cx: &mut App) {
        self.defer_to(cx, |s, cx| {
            s.dms_expanded = !s.dms_expanded;
            s.sidebar_cache.borrow_mut().rows = None;
            cx.notify();
        });
    }
}

/// Search hits whose context answers are kept (moving back and forth
/// through the hits asks nothing again).
const SEARCH_CONTEXT_KEEP: usize = 16;

/// Most messages of a thread read to find a reply hit's neighbours.
const SEARCH_THREAD_LIMIT: usize = 200;

/// Aborts a runtime task when the GPUI task awaiting it is dropped.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl WorkspaceSession {
    /// The messages around search hit `id`: its conversation's window, or
    /// an answer read for it (empty when there is none to show).
    fn search_context_messages(&self, id: &MessageId) -> Option<&[Message]> {
        if let Some(w) = self.state.windows.get(&id.conversation)
            && let Some(m) = components::context_around(w.messages(), id)
        {
            return Some(m);
        }
        self.search_context
            .iter()
            .find(|(at, _)| at == id)
            .map(|(_, m)| &**m)
    }

    /// Like [`run`](Self::run), but returns the task: dropping it cancels
    /// the work, on the runtime too.
    fn run_cancellable<T, F>(
        &self,
        cx: &mut Context<Self>,
        fut: F,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) -> Task<()>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let handle = crate::runtime::handle(cx);
        cx.spawn(async move |this, cx| {
            let out = match handle {
                Some(h) => {
                    let mut job = AbortOnDrop(h.spawn(fut));
                    (&mut job.0).await.ok()
                }
                None => Some(fut.await),
            };
            if let Some(out) = out {
                this.update(cx, |s, cx| then(s, out, cx)).ok();
            }
        })
    }

    /// The selected search hit is now `id`: drops the read for the hit
    /// before and reads the messages around `id` unless known. A thread
    /// reply reads its thread (never the channel around it); otherwise the
    /// store, then the provider when the store lacks the hit.
    fn load_search_context(&mut self, id: Option<MessageId>, cx: &mut Context<Self>) {
        self.search_context_task = None;
        self.search_context_pending = None;
        let Some(id) = id else { return };
        if self.search_context_messages(&id).is_some() {
            return;
        }
        // The hit as found, when it says it is a reply.
        let thread = self
            .search_local
            .iter()
            .find(|h| h.message.id == id)
            .and_then(|h| h.message.thread_root.clone())
            .filter(|r| *r != id.ts);
        self.search_context_pending = Some(id.clone());
        let p = self.provider.clone();
        let (st, ws) = (self.store.clone(), self.state.workspace.clone());
        let t = id.clone();
        let n = components::CONTEXT_AROUND;
        let around = |msgs: &[Message], t: &MessageId| {
            components::context_around(msgs, t)
                .map(<[Message]>::to_vec)
                .unwrap_or_default()
        };
        let task = self.run_cancellable(
            cx,
            async move {
                let c = &t.conversation;
                let at = st.message(&ws, c, &t.ts).await.ok().flatten();
                let root = thread.or_else(|| {
                    at.as_ref()
                        .and_then(|m| m.thread_root.clone())
                        .filter(|r| *r != t.ts)
                });
                if let Some(root) = root {
                    let held = st
                        .thread(&ws, c, &root, SEARCH_THREAD_LIMIT)
                        .await
                        .unwrap_or_default();
                    if held.iter().any(|m| m.id == t) {
                        return around(&held, &t);
                    }
                    return match p.replies(c, &root, None, SEARCH_THREAD_LIMIT as u32).await {
                        Ok(page) => around(&page.messages, &t),
                        Err(_) => Vec::new(),
                    };
                }
                if let Some(at) = at {
                    let mut out = st
                        .messages_before(&ws, c, Some(t.ts.clone()), n)
                        .await
                        .unwrap_or_default();
                    out.reverse();
                    out.push(at);
                    out.extend(
                        st.messages_after(&ws, c, &t.ts, n)
                            .await
                            .unwrap_or_default(),
                    );
                    return out;
                }
                match p.history_around(c, &t.ts, (2 * n + 1) as u32).await {
                    Ok(page) => around(&page.messages, &t),
                    Err(_) => Vec::new(),
                }
            },
            move |s, msgs, cx| {
                // Only the answer for the hit still selected lands.
                if s.search_context_pending.as_ref() != Some(&id) {
                    return;
                }
                s.search_context_pending = None;
                if s.search_context.len() >= SEARCH_CONTEXT_KEEP {
                    s.search_context.pop_front();
                }
                // Kept even when empty, so the hit is not asked again.
                s.search_context.push_back((id, msgs.into()));
                cx.notify();
            },
        );
        self.search_context_task = Some(task);
    }
}

impl SearchSource for WorkspaceSession {
    fn context(&self, id: &MessageId, cx: &App) -> Vec<components::PreviewLine> {
        use components::TimelineSource as _;
        let Some(msgs) = self.search_context_messages(id) else {
            return Vec::new();
        };
        let cfg = self.send_later_settings(cx);
        let l = i18n::gpui::localizer(cx);
        let parser = self.content();
        msgs.iter()
            .map(|m| {
                let author = m.author.as_ref().and_then(|u| self.state.members.get(u));
                components::preview_line(
                    m,
                    author,
                    &*parser,
                    cfg.clock,
                    &cfg.zone,
                    &l,
                    &self.state.user_groups,
                )
            })
            .collect()
    }

    fn load_context(&mut self, id: Option<MessageId>, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.load_search_context(id, cx));
    }

    fn search(&mut self, scope: SearchScope, query: String, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::search(s, scope, query, cx)
        });
    }

    fn search_results(&self) -> &SearchResults {
        &self.search_results
    }

    fn more(&mut self, cx: &mut App) {
        self.defer_to(cx, WorkspaceSession::search_more);
    }

    fn remote_search(&self) -> bool {
        self.provider.capabilities().search
    }

    fn provider_name(&self) -> &str {
        &self.provider_name
    }
}

impl TimelineSource for WorkspaceSession {
    fn offline(&self) -> bool {
        self.state.connection == domain::Connection::Offline
    }

    fn session_expired(&self) -> bool {
        self.state.session == Session::Expired
    }

    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }

    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }

    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }

    fn capabilities(&self) -> &Capabilities {
        self.provider.capabilities()
    }

    fn load_older(&mut self, c: ConversationId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::load_older(s, c, cx));
    }

    fn history_failed(&self, c: &ConversationId) -> bool {
        WorkspaceSession::history_failed(self, c)
    }

    fn retry_history(&mut self, c: ConversationId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::retry_history(s, c, cx));
    }

    fn jump(&self) -> Option<&MessageId> {
        self.state.jump.as_ref()
    }

    fn jump_to_latest(&mut self, c: ConversationId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::jump_to_latest(s, c, cx));
    }

    fn edit(&mut self, id: MessageId, text: String, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::edit(s, id, text, cx));
    }

    fn delete(&mut self, id: MessageId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::delete(s, id, cx));
    }

    fn retry_send(&mut self, id: domain::ClientMsgId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.apply(Input::Retry(id), cx));
    }

    fn discard_send(&mut self, id: domain::ClientMsgId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.apply(Input::Discard(id), cx));
    }

    fn react(&mut self, id: MessageId, emoji: EmojiRef, add: bool, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::react(s, id, emoji, add, cx)
        });
    }

    fn custom_emoji(&self) -> Vec<CustomEmoji> {
        self.state.custom_emoji.values().cloned().collect()
    }

    fn custom_emoji_image(&self, shortcode: &str) -> Option<domain::ImageRef> {
        self.state
            .custom_emoji
            .get(shortcode)
            .map(|e| e.image.clone())
    }

    fn user_group(&self, id: &str) -> Option<&domain::UserGroup> {
        self.state.user_groups.get(id)
    }

    fn recent_emoji(&self) -> Vec<EmojiRef> {
        WorkspaceSession::recent_emoji(self)
    }

    fn download_clip(
        &mut self,
        url: domain::RemoteUrl,
        dest: std::path::PathBuf,
        done: components::timeline::ClipDone,
        cx: &mut App,
    ) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::fetch_clip(s, url, dest, done, cx)
        });
    }

    fn fetch_captions(
        &mut self,
        url: domain::RemoteUrl,
        max_bytes: u64,
        done: components::timeline::CaptionsDone,
        cx: &mut App,
    ) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::fetch_captions(s, url, max_bytes, done, cx)
        });
    }

    fn content(&self) -> Arc<dyn ContentParser> {
        self.content.clone()
    }

    fn opened_marker(&self, id: &ConversationId) -> Option<Option<domain::MessageTs>> {
        self.opened_marker
            .as_ref()
            .filter(|(c, _)| c == id)
            .map(|(_, m)| m.clone())
    }

    fn conversation(&self, id: &ConversationId) -> Option<&Conversation> {
        self.state.conversations.get(id)
    }
    fn workspace_state(&self) -> Option<&domain::WorkspaceState> {
        Some(&self.state)
    }

    fn provider_name(&self) -> &str {
        &self.provider_name
    }

    fn web_url(&self, id: &MessageId) -> Option<String> {
        self.provider.web_url(id)
    }

    fn open_conversation(&mut self, id: ConversationId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::open(s, id, cx));
    }

    fn huddle_live(&self, c: &ConversationId) -> bool {
        self.state.huddles.contains_key(c)
    }

    fn join_huddle(&mut self, c: ConversationId, cx: &mut App) {
        components::HuddleSource::join_huddle(self, c, cx)
    }

    fn open_direct(&mut self, user: UserId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.open_direct_now(user, cx));
    }

    fn open_profile(&mut self, user: UserId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.show_profile(user, cx));
    }

    fn download(&mut self, file: &domain::FileRef, cx: &mut App) {
        let file = file.clone();
        self.defer_to(cx, move |s, cx| WorkspaceSession::download(s, &file, cx));
    }

    fn open_media(&mut self, gallery: components::media_viewer::Gallery, cx: &mut App) {
        let host: std::rc::Rc<dyn crate::media_viewer::ViewerHost> =
            std::rc::Rc::new(self.this.clone());
        cx.defer(move |cx| crate::media_viewer::open(gallery, host, cx));
    }

    fn open_code(&mut self, gallery: components::media_viewer::Gallery, cx: &mut App) {
        let host: std::rc::Rc<dyn crate::code_viewer::CodeHost> =
            std::rc::Rc::new(self.this.clone());
        cx.defer(move |cx| crate::code_viewer::open(gallery, host, cx));
    }

    fn stage(
        &mut self,
        c: ConversationId,
        thread: Option<MessageTs>,
        paths: Vec<std::path::PathBuf>,
        cx: &mut App,
    ) {
        self.defer_to(cx, move |s, cx| s.stage_files(c, thread, paths, cx));
    }

    fn file_notice(&self) -> Option<FileNotice> {
        WorkspaceSession::file_notice(self)
    }

    fn dismiss_file_notice(&mut self, cx: &mut App) {
        self.defer_to(cx, |s, cx| {
            s.files.notice = None;
            cx.notify();
        });
    }

    fn mark_unread(&mut self, id: MessageId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::mark_unread(s, id, cx));
    }

    fn can_remind(&self) -> bool {
        !self.state.caps.reminders && self.state.self_user.is_some()
    }

    fn remind_about(&mut self, id: MessageId, at: i64, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.remind_about_message(id, at, cx));
    }

    fn reminder_zone(&self, cx: &App) -> Option<jiff::tz::TimeZone> {
        Some(self.send_later_settings(cx).zone)
    }

    fn set_pinned(&mut self, id: MessageId, pinned: bool, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            s.apply(Input::SetPinned { id, pinned }, cx)
        });
    }

    fn set_saved(&mut self, id: MessageId, saved: bool, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.apply(Input::SetSaved { id, saved }, cx));
    }

    fn focus(&mut self, c: ConversationId, at_bottom: bool, window_active: bool, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::focus(s, c, at_bottom, window_active, cx)
        });
    }
}

impl components::CollectionsSource for WorkspaceSession {
    fn pins(&self, c: &ConversationId) -> Option<&[domain::Pin]> {
        self.state.pins.get(c).map(Vec::as_slice)
    }

    fn bookmarks(&self, c: &ConversationId) -> Option<&[domain::Bookmark]> {
        self.state.bookmarks.get(c).map(Vec::as_slice)
    }

    fn saved(&self) -> Option<&domain::Page<domain::SavedItem>> {
        self.state.saved.as_ref()
    }

    fn thread_inbox(&self) -> Option<&domain::Page<domain::InboxThread>> {
        self.state.thread_inbox.as_ref()
    }

    fn referenced(&self, id: &MessageId) -> Option<&Message> {
        self.state.referenced.get(id).or_else(|| {
            self.state
                .windows
                .get(&id.conversation)
                .and_then(|w| w.messages().iter().find(|m| m.id == *id))
        })
    }

    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }

    fn conversation_label(&self, c: &ConversationId, _cx: &App) -> String {
        // An unknown conversation shows nothing, never its raw id.
        if self.state.conversations.contains_key(c) {
            WorkspaceSession::conversation_label(self, c)
        } else {
            String::new()
        }
    }

    fn content(&self) -> Arc<dyn ContentParser> {
        self.content.clone()
    }

    fn time_config(&self, cx: &App) -> components::SendLaterConfig {
        components::ScheduledSource::send_later_config(self, cx)
    }

    fn dispatch(&mut self, input: Input, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.apply(input, cx));
    }
}

impl ComposerSource for WorkspaceSession {
    fn typed(&mut self, c: ConversationId, thread: Option<domain::MessageTs>, cx: &mut App) {
        // Most keystrokes fall inside the throttle: skip the input for them.
        let now = std::time::Instant::now();
        if !self.state.typing_due(&c, now) {
            return;
        }
        let input = Input::Typed {
            conversation: c,
            thread,
            now,
        };
        self.defer_to(cx, move |s, cx| s.apply(input, cx));
    }

    fn typing(
        &self,
        c: &ConversationId,
        thread: Option<&domain::MessageTs>,
    ) -> Vec<Option<&domain::Member>> {
        let typers = self.state.typers(c, thread);
        if typers.is_empty() {
            return Vec::new();
        }
        let now = std::time::Instant::now();
        typers
            .iter()
            .filter(|t| t.active(now))
            .map(|t| self.state.members.get(&t.user))
            .collect()
    }

    fn read_only(&self) -> bool {
        self.state.session == Session::Expired
    }

    fn must_join(&self, c: &ConversationId) -> bool {
        WorkspaceSession::must_join(self, c)
    }

    fn join_conversation(&mut self, c: ConversationId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.join(c, cx));
    }

    fn joining(&self, c: &ConversationId) -> bool {
        self.joining.contains(c)
    }

    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }

    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }

    fn capabilities(&self) -> &Capabilities {
        self.provider.capabilities()
    }

    fn staged(
        &self,
        c: &ConversationId,
        thread: Option<&MessageTs>,
    ) -> Vec<components::StagedFile> {
        self.staged_files(c, thread)
    }

    fn stage(
        &mut self,
        c: ConversationId,
        thread: Option<MessageTs>,
        paths: Vec<std::path::PathBuf>,
        cx: &mut App,
    ) {
        self.defer_to(cx, move |s, cx| s.stage_files(c, thread, paths, cx));
    }

    fn supports_snippets(&self) -> bool {
        true
    }

    fn stage_snippet(
        &mut self,
        c: ConversationId,
        thread: Option<MessageTs>,
        snippet: components::NewSnippet,
        cx: &mut App,
    ) {
        self.defer_to(cx, move |s, cx| s.stage_snippet(c, thread, snippet, cx));
    }

    fn unstage(&mut self, _c: ConversationId, _thread: Option<MessageTs>, id: u64, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.unstage_file(id, cx));
    }

    fn cancel_staged(
        &mut self,
        _c: ConversationId,
        _thread: Option<MessageTs>,
        id: u64,
        cx: &mut App,
    ) {
        self.defer_to(cx, move |s, cx| s.cancel_staged_file(id, cx));
    }

    fn retry_staged(
        &mut self,
        _c: ConversationId,
        _thread: Option<MessageTs>,
        id: u64,
        cx: &mut App,
    ) {
        self.defer_to(cx, move |s, cx| s.retry_staged_file(id, cx));
    }

    fn send_staged(
        &mut self,
        c: ConversationId,
        thread: Option<MessageTs>,
        text: Option<String>,
        cx: &mut App,
    ) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::send_staged(s, c, thread, text, cx)
        });
    }

    fn completions(&self, kind: CompletionKind, query: &str) -> Vec<Completion> {
        match kind {
            CompletionKind::Person => {
                components::completion::people(&self.state, self.selected.as_ref(), query)
            }
            CompletionKind::Conversation => {
                components::completion::conversations(&self.state, query)
            }
            CompletionKind::Emoji => {
                components::completion::emoji(self.state.custom_emoji.values(), query)
            }
        }
    }

    fn recent_emoji(&self) -> Vec<EmojiRef> {
        WorkspaceSession::recent_emoji(self)
    }

    fn emoji_used(&mut self, e: EmojiRef, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::emoji_used(s, e, cx));
    }

    fn encode(&self, t: &domain::ComposedText) -> String {
        self.encoder.encode(t)
    }

    fn decode(&self, markup: &str) -> domain::ComposedText {
        // The markup names people, channels and groups by id; show them by name,
        // as the completion does.
        components::completion::relabel_decoded(&self.state, &self.encoder.decode(markup))
    }

    fn picker_custom_emoji(&self) -> Vec<CustomEmoji> {
        self.state.custom_emoji.values().cloned().collect()
    }

    fn custom_emoji_image(&self, shortcode: &str) -> Option<domain::ImageRef> {
        self.state
            .custom_emoji
            .get(shortcode)
            .map(|e| e.image.clone())
    }

    fn provider_name(&self) -> &str {
        &self.provider_name
    }

    fn run_slash(
        &mut self,
        cmd: domain::SlashCommand,
        cx: &mut App,
    ) -> Result<(), domain::SlashError> {
        let mut input = domain::slash_to_input(cmd, &self.state, components::now(cx).as_second())?;
        if let Input::AddReminder { text, when, user } = &input
            && !self.state.caps.reminders
        {
            // No provider reminders: Parla keeps it, for the user alone.
            input = self
                .local_reminder(text, when, user.is_none(), cx)
                .ok_or(domain::SlashError::Usage(domain::REMIND_USAGE))?;
        }
        self.defer_to(cx, move |s, cx| s.apply(input, cx));
        Ok(())
    }

    fn send_later(&self, cx: &App) -> components::SendLaterConfig {
        self.send_later_settings(cx)
    }

    fn open_in_provider(&mut self, c: &ConversationId, cx: &mut App) {
        // The provider links messages, not conversations: the newest one opens the conversation.
        // The newest confirmed one: unconfirmed rows carry a local ts.
        let last = self
            .state
            .windows
            .get(c)
            .and_then(|w| w.newest_delivered_ts())
            .map(|ts| MessageId {
                conversation: c.clone(),
                ts: ts.clone(),
            });
        if let Some(url) = last.and_then(|id| self.provider.web_url(&id)) {
            cx.open_url(&url);
        }
    }
}

impl components::ChannelDetailsSource for WorkspaceSession {
    fn details_state(&self) -> &domain::WorkspaceState {
        &self.state
    }

    fn details_conversation(&self) -> Option<&ConversationId> {
        self.selected()
    }

    fn details_content(&self) -> std::sync::Arc<dyn domain::ContentParser> {
        components::TimelineSource::content(self)
    }

    fn details_can_admin(&self) -> bool {
        self.can_admin()
            && self
                .selected()
                .and_then(|c| self.state.conversations.get(c))
                .is_some_and(|c| {
                    matches!(
                        c.kind,
                        domain::ConversationKind::Channel
                            | domain::ConversationKind::PrivateChannel
                    ) && c.is_member
                        && !c.is_archived
                })
    }
}

/// Single-member changes the People directory can patch from.
const MEMBER_LOG: usize = 256;

impl WorkspaceSession {
    /// Bumps the members revision; a change to one known member is logged so
    /// the People directory patches that entry instead of rebuilding.
    fn log_member_change(&mut self, single: bool) {
        self.members_rev += 1;
        match self.member_touch.clone().filter(|_| single) {
            Some(u) => {
                self.member_log.push_back((self.members_rev, u));
                if self.member_log.len() > MEMBER_LOG
                    && let Some((rev, _)) = self.member_log.pop_front()
                {
                    self.member_log_floor = rev;
                }
            }
            None => {
                self.member_log.clear();
                self.member_log_floor = self.members_rev;
            }
        }
    }

    /// The people the open People directory asked to watch (a test hook).
    pub fn directory_watch(&self) -> &[UserId] {
        &self.directory_people
    }

    /// The People directory closed: its cards are no longer watched.
    pub fn clear_directory_watch(&mut self, cx: &mut Context<Self>) {
        if !self.directory_people.is_empty() {
            self.directory_people.clear();
            self.sync_presence_watch(cx);
        }
    }
}

impl components::PeopleSource for WorkspaceSession {
    fn people_state(&self) -> &domain::WorkspaceState {
        &self.state
    }

    fn members_revision(&self) -> u64 {
        self.members_rev
    }

    fn members_changed_since(&self, since: u64) -> Option<Vec<UserId>> {
        (since >= self.member_log_floor).then(|| {
            self.member_log
                .iter()
                .filter(|(rev, _)| *rev > since)
                .map(|(_, u)| u.clone())
                .collect()
        })
    }

    fn clock(&self, cx: &App) -> settings::Clock {
        self.settings(cx).messages.clock
    }

    fn watch_people(&mut self, people: Vec<UserId>, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            if s.directory_people != people {
                s.directory_people = people;
                s.sync_presence_watch(cx);
            }
        });
    }
}

impl components::ProfileSource for WorkspaceSession {
    fn profile(&self, u: &UserId) -> Option<&domain::Profile> {
        self.state.profiles.get(u)
    }

    fn dnd(&self, u: &UserId) -> Option<&domain::DndStatus> {
        self.state.dnd.get(u)
    }

    fn in_huddle(&self, u: &UserId) -> bool {
        self.state.in_huddle.contains(u)
    }

    fn shown_profile(&self) -> Option<&UserId> {
        self.state.open_profile.as_ref()
    }

    fn is_own(&self, u: &UserId) -> bool {
        self.state.self_user.as_ref() == Some(u)
    }

    fn own_presence(&self) -> Option<domain::PresenceSetting> {
        self.state.presence_setting
    }

    fn has_dm(&self, u: &UserId) -> bool {
        self.state
            .conversations
            .values()
            .any(|c| c.kind == domain::ConversationKind::Direct && c.peer.as_ref() == Some(u))
    }

    fn open_dm(&mut self, u: &UserId, cx: &mut App) {
        let u = u.clone();
        self.defer_to(cx, move |s, cx| s.open_direct_now(u, cx));
    }

    fn close_profile(&mut self, cx: &mut App) {
        self.defer_to(cx, WorkspaceSession::hide_profile);
    }

    fn set_status(&mut self, st: Option<domain::Status>, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.apply(Input::SetStatus(st), cx));
    }

    fn set_presence(&mut self, p: domain::PresenceSetting, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.apply(Input::SetPresence(p), cx));
    }

    fn presence(&self, u: &UserId) -> Option<bool> {
        self.state.presence.get(u).copied()
    }

    fn can_open_dm(&self) -> bool {
        self.provider.capabilities().open_direct
    }

    fn can_set_status(&self) -> bool {
        self.provider.capabilities().status
    }

    fn can_set_presence(&self) -> bool {
        self.provider.capabilities().presence
    }

    fn clock(&self, cx: &App) -> settings::Clock {
        self.settings(cx).messages.clock
    }
}

impl ThreadSource for WorkspaceSession {
    fn thread(&self, root: &MessageId) -> Option<&domain::ThreadWindow> {
        self.state.threads.get(root)
    }

    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }

    fn capabilities(&self) -> &Capabilities {
        self.provider.capabilities()
    }

    fn open_thread(&mut self, root: MessageId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::open_thread(s, root, cx));
    }

    fn close_thread(&mut self, root: MessageId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| WorkspaceSession::close_thread(s, root, cx));
    }

    fn replies_failed(&self, root: &MessageId) -> bool {
        WorkspaceSession::replies_failed(self, root)
    }

    fn retry_replies(&mut self, root: MessageId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::retry_replies(s, root, cx)
        });
    }

    fn reply(&mut self, root: MessageId, text: String, broadcast: bool, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::reply(s, root, text, broadcast, cx)
        });
    }

    fn react(&mut self, id: MessageId, emoji: EmojiRef, add: bool, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::react(s, id, emoji, add, cx)
        });
    }

    fn thread_follow(&self, root: &MessageId) -> Option<bool> {
        self.state
            .caps
            .thread_follow
            .then(|| self.state.thread_followed(root))
    }

    fn set_thread_followed(&mut self, root: MessageId, follow: bool, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            s.apply(Input::SetThreadFollowed { root, follow }, cx)
        });
    }
    fn custom_emoji_image(&self, shortcode: &str) -> Option<domain::ImageRef> {
        self.state
            .custom_emoji
            .get(shortcode)
            .map(|e| e.image.clone())
    }
}

/// A short, content-free name for a failed provider call (no ids, no text).
fn error_kind(e: &ProviderError) -> &'static str {
    match e {
        ProviderError::SessionExpired => "session_expired",
        ProviderError::NotAllowed(_) => "not_allowed",
        ProviderError::RateLimited => "rate_limited",
        ProviderError::Network(_) => "network",
        ProviderError::Unsupported => "unsupported",
        ProviderError::TooLarge => "too_large",
        ProviderError::Other(_) | ProviderError::Localized(_) => "provider_error",
    }
}

/// Channels per page of the channel browser.
const BROWSE_PAGE: u32 = 50;

#[cfg(test)]
mod client_id_tests {
    #[test]
    fn client_ids_are_lowercase_uuid_v4_and_distinct() {
        let (a, b) = (super::new_uuid(), super::new_uuid());
        assert_ne!(a, b);
        let p = uuid::Uuid::parse_str(&a).expect("a uuid");
        assert_eq!(p.get_version_num(), 4);
        assert_eq!(a, a.to_lowercase());
        assert_eq!(a.len(), 36);
    }
}
