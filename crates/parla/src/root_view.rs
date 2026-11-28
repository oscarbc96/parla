//! Main window: rail, sidebar, timeline + composer and a panel placeholder,
//! wired to the active workspace's session.

mod channel_admin;
pub(crate) mod chat_pane;

use std::rc::Rc;
use std::sync::Arc;

use components::onboarding::{
    Onboarding, OnboardingEvent, ProviderCard, SyncProgress, WorkspaceSync,
};
use components::sign_in::{SignInEvent, SignInScreen};
use components::tour::{Tour, TourEvent, TourKeys, TourLayout};
use components::{
    AggregateEvent, AggregateKind, AggregateTarget, AggregateView, AllMentions, AllScheduled,
    AllUnreads, BookmarksBar, ChannelDetailsEvent, ChannelDetailsPanel, CollectionEvent,
    CommandDelegate, CommandItem, CommandPalette, Composer, GoBack, GoForward, PickerEvent,
    PinsPopover, ProfilePanel, ProfilePanelEvent, QuickSwitch, RemindersEvent, RemindersView,
    SavedView, ScheduledEvent, ScheduledSource, ScheduledView, Search, SearchPanel,
    SearchPanelEvent, Sidebar, SidebarSource, StatusDialog, StatusDialogEvent, SwitchDelegate,
    SwitchItem, SwitchTarget, ThreadPanel, ThreadPanelEvent, ThreadsView, Timeline,
};
use domain::{ConversationId, DraftKey, MessageId, ProviderError, Session};
use gpui::{
    AnyElement, App, Context, Entity, Focusable as _, IntoElement, Render, Subscription, Task,
    Window, div, prelude::*, px,
};
use settings::{SettingsStore, WorkspaceKey};
use store::Store;
use themes::ActiveTheme;
use themes::parla_tokens;

use crate::actions::{
    DeleteAllData, PauseNotifications, PopOutConversation, PopOutThread, RebuildCache,
    RemoveWorkspace, SetStatus, SignInAgain, SignOut,
};
use crate::expiry_banner::expiry_banner;
use crate::idle_lock::IdleInput as _;
use crate::nav_history::{NavEntry, NavHistory};
use crate::providers::ProviderRegistry;
use crate::session::WorkspaceSession;
use crate::session::{SessionEvent, StoreHealth};
use crate::store_banner::store_banner;
use crate::titlebar;
use crate::toast::ConfigBanner;
use crate::workspaces::{Workspaces, WorkspacesEvent, WorkspacesHandle};
use components::NextUnread;
use components::rail::{
    ActivateWorkspace1, ActivateWorkspace2, ActivateWorkspace3, ActivateWorkspace4,
    ActivateWorkspace5, ActivateWorkspace6, ActivateWorkspace7, ActivateWorkspace8,
    ActivateWorkspace9, NextWorkspace, PreviousWorkspace, RailActions, RailItem,
};

/// Puts the saved draft for conversation `c` into `composer`.
fn restore_draft(
    session: &Entity<WorkspaceSession>,
    composer: &Entity<Composer<WorkspaceSession>>,
    c: &ConversationId,
    cx: &mut App,
) {
    let key = DraftKey {
        conversation: c.clone(),
        thread: None,
    };
    restore_draft_for(session, composer, &key, cx);
}

pub(crate) fn restore_draft_for(
    session: &Entity<WorkspaceSession>,
    composer: &Entity<Composer<WorkspaceSession>>,
    key: &DraftKey,
    cx: &mut App,
) {
    if let Some(d) = session.read(cx).draft(key) {
        composer.update(cx, |c, cx| c.set_composed(&d, cx));
    }
}

/// "Could not load the workspace: <why>" in the active language.
fn load_failed_text(cx: &App, why: &i18n::LocalizedText) -> String {
    let l = i18n::gpui::localizer(cx);
    t!(l, "shell-load-failed", reason = l.text(why).into_owned()).into_owned()
}

/// Where startup is.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// No account is configured: show the sign-in state.
    NoAccounts,
    Starting,
    Ready,
    /// Stored credentials no longer work.
    Expired,
    /// Why the account could not start, as catalog text rendered per frame.
    Failed(i18n::LocalizedText),
}

/// A visible one-line error strip: danger-soft ground, a bold title then plain
/// text (split at the first colon), room for an action after it.
pub fn error_strip(
    colors: &themes::ThemeColors,
    soft: gpui::Hsla,
    text: &str,
) -> gpui::Stateful<gpui::Div> {
    let (title, rest) = match text.split_once(": ") {
        Some((t, r)) => (format!("{t}:"), Some(r.to_string())),
        None => (text.to_string(), None),
    };
    div()
        .id("error-banner")
        .debug_selector(|| "error-banner".into())
        .flex()
        .items_center()
        .gap(themes::metrics::SPACE_5)
        .px(themes::metrics::SPACE_6)
        .py(themes::metrics::SPACE_5)
        .bg(soft)
        .text_color(colors.text)
        .text_size(themes::metrics::FONT_SIZE_MD)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .gap_x(themes::metrics::SPACE_2)
                .child(
                    div()
                        .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                        .child(title),
                )
                .children(rest),
        )
}

/// An overlay's full-window scrim: centres its child at the top.
fn scrim(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .flex()
        .justify_center()
        .items_start()
        .occlude()
}

pub(crate) fn thread_key(root: &MessageId) -> DraftKey {
    DraftKey {
        conversation: root.conversation.clone(),
        thread: Some(root.ts.clone()),
    }
}

/// The call bar and what it listens to.
struct CallBarHost {
    bar: Entity<components::CallBar>,
    _sub: Subscription,
}

/// A major pane focus can move to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Rail,
    Sidebar,
    Timeline,
    Composer,
    RightPanel,
}

struct Chat {
    conversation: ConversationId,
    /// The bookmarks bar under the header, when the provider has bookmarks.
    bookmarks: Option<Entity<BookmarksBar<WorkspaceSession>>>,
    /// The huddle bar above the timeline, when the provider has huddles.
    huddle: Option<Entity<components::HuddleBar<WorkspaceSession>>>,
    timeline: Entity<Timeline<WorkspaceSession>>,
    composer: Entity<Composer<WorkspaceSession>>,
    /// The Files tab, while it is the one shown (built on each visit, so it
    /// lists what the store holds then).
    files: Option<Entity<components::FilesTab<WorkspaceSession>>>,
    _subs: Vec<Subscription>,
}

struct Ready {
    session: Entity<WorkspaceSession>,
    sidebar: Entity<Sidebar<WorkspaceSession>>,
    chat: Option<Chat>,
    /// The one open right-column panel (search, scheduled, reminders, profile
    /// or thread): opening another replaces it.
    right: Option<RightPanel>,
    /// Threads or Later, filling the centre column.
    collection: Option<CollectionPane>,
    /// The pins popover under the header.
    pins: Option<PinsPane>,
    /// The People directory a profile was opened from: closing that profile
    /// shows it again.
    people_back: Option<PeoplePane>,
    _sub: Subscription,
    _events: Subscription,
    _sidebar_events: Subscription,
}

/// What fills the right column. Only one at a time.
enum RightPanel {
    Search(SearchPane),
    Scheduled(ScheduledPane),
    Reminders(RemindersPane),
    Drafts(DraftsPane),
    People(PeoplePane),
    Profile(ProfilePane),
    Details(DetailsPane),
    Thread(ThreadPane),
}

impl RightPanel {
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        match self {
            RightPanel::Search(p) => p.panel.focus_handle(cx),
            RightPanel::Scheduled(p) => p.view.focus_handle(cx),
            RightPanel::Reminders(p) => p.view.focus_handle(cx),
            RightPanel::Drafts(p) => p.view.focus_handle(cx),
            RightPanel::People(p) => p.view.focus_handle(cx),
            RightPanel::Profile(p) => p.panel.focus_handle(cx),
            RightPanel::Details(p) => p.panel.focus_handle(cx),
            RightPanel::Thread(p) => p.panel.focus_handle(cx),
        }
    }
}

impl Ready {
    /// Empties the right column. A thread there keeps its composer text as a
    /// draft and is closed on the session.
    fn close_right(&mut self, cx: &mut App) {
        // Whatever replaces the panel, the directory a profile came from is gone.
        self.people_back = None;
        if matches!(self.right, Some(RightPanel::People(_))) {
            self.session.update(cx, |s, cx| s.clear_directory_watch(cx));
        }
        if let Some(RightPanel::Thread(t)) = self.right.take() {
            let panel = t.panel.read(cx);
            let root = panel.root().clone();
            let text = panel.composer().read(cx).composed(cx);
            self.session.update(cx, |s, cx| {
                s.set_draft(thread_key(&root), text, cx);
                s.close_thread(root, cx)
            });
        }
    }
}

struct ScheduledPane {
    view: Entity<ScheduledView<WorkspaceSession>>,
    _sub: Subscription,
}

struct RemindersPane {
    view: Entity<RemindersView<WorkspaceSession>>,
    _sub: Subscription,
}

struct DraftsPane {
    view: Entity<components::DraftsView<WorkspaceSession>>,
    _sub: Subscription,
    /// Follows the open composer, so its text shows in the list as typed.
    _live: Option<Subscription>,
}

/// The workspace views that fill the centre column.
enum CollectionKind {
    Threads(Entity<ThreadsView<WorkspaceSession>>),
    Later(Entity<SavedView<WorkspaceSession>>),
}

struct CollectionPane {
    kind: CollectionKind,
    _sub: Subscription,
}

struct PinsPane {
    popover: Entity<PinsPopover<WorkspaceSession>>,
    _sub: Subscription,
}

struct DetailsPane {
    panel: Entity<ChannelDetailsPanel<WorkspaceSession>>,
    _sub: Subscription,
}

struct PeoplePane {
    view: Entity<components::PeopleDirectory<WorkspaceSession>>,
    _sub: Subscription,
}

struct ProfilePane {
    panel: Entity<ProfilePanel<WorkspaceSession>>,
    _sub: Subscription,
}

/// The status dialog, over the window.
struct StatusPane {
    dialog: Entity<StatusDialog>,
    _sub: Subscription,
}

struct SearchPane {
    panel: Entity<SearchPanel<WorkspaceSession>>,
    _sub: Subscription,
}

struct AggregatePane {
    kind: AggregateKind,
    view: Entity<AggregateView<Workspaces>>,
    _sub: Subscription,
}

/// The open notification menu (rail, sidebar row or header), at a point.
struct MenuPane {
    menu: Entity<components::NotifyMenu<WorkspaceSession>>,
    at: gpui::Point<gpui::Pixels>,
    _sub: Subscription,
}

struct SwitcherPane {
    picker: Entity<components::QuickSwitcher>,
    /// Channels the provider's browse found for the typed text, the user
    /// not in them (one page, replaced by each search).
    found: Vec<domain::Conversation>,
    /// The text the last browse was for.
    query: String,
    /// The wait for the typing to settle; a new keystroke drops it.
    pending: Option<Task<()>>,
    /// The browse on its way; a newer query aborts it.
    loading: Option<futures::future::AbortHandle>,
    _sub: Subscription,
    _observe: Option<Subscription>,
}

struct ShortcutsSheetPane {
    sheet: Entity<components::shortcuts_sheet::ShortcutsSheet>,
    previous_focus: Option<gpui::FocusHandle>,
    _sub: Subscription,
}

struct CommandPalettePane {
    picker: Entity<CommandPalette>,
    previous_focus: Option<gpui::FocusHandle>,
    _sub: Subscription,
}

struct ThreadPane {
    panel: Entity<ThreadPanel<WorkspaceSession>>,
    _sub: Subscription,
}

/// The encrypted store, when the app runs with one (`main`); sessions open it
/// per workspace. Tests that pass a plain `Store` leave it unset.
pub struct SqliteHandle(pub store_sqlite::SqliteStore);

impl gpui::Global for SqliteHandle {}

struct SignInWindowState {
    handle: gpui::WindowHandle<crate::sign_in_window::SignInWindow>,
    screen: Entity<SignInScreen>,
    /// The Welcome and provider screens ahead of the sign-in, when it has them.
    onboarding: Option<Entity<Onboarding>>,
    /// What the shown sign-in screen reports; replaced with the screen when
    /// the provider picker moves to another provider.
    events: Subscription,
    _subs: Vec<Subscription>,
}

impl crate::delete_confirm::DeleteConfirmHost for RootView {
    fn delete_confirm_slot(&mut self) -> &mut Option<crate::delete_confirm::DeleteConfirm> {
        &mut self.delete_confirm
    }
}

pub struct RootView {
    phase: Phase,
    ready: Option<Ready>,
    /// Every workspace session; the view shows the active one.
    workspaces: Entity<Workspaces>,
    _workspaces_sub: Subscription,
    /// Back and forward, and the recent list of the quick switcher.
    nav: NavHistory,
    /// The entry back or forward is moving to; visits on the way are not recorded.
    nav_target: Option<NavEntry>,
    switcher: Option<SwitcherPane>,
    status: Option<StatusPane>,
    /// The open delete-message confirmation, if any.
    delete_confirm: Option<crate::delete_confirm::DeleteConfirm>,
    /// The open channel admin dialog (create, browse, edit, invite, confirm).
    admin: Option<channel_admin::AdminPane>,
    /// The open cross-workspace view (All unreads, mentions or scheduled).
    aggregate: Option<AggregatePane>,
    menu: Option<MenuPane>,
    command_palette: Option<CommandPalettePane>,
    shortcuts_sheet: Option<ShortcutsSheetPane>,
    /// Focus of the window itself, so window-level keys (the quick switcher,
    /// back and forward) work before any conversation is open.
    focus: gpui::FocusHandle,
    /// Focus handle for the workspace rail pane.
    rail_focus: gpui::FocusHandle,
    registry: ProviderRegistry,
    /// A removal that failed, shown above the columns until the next change.
    notice: Option<String>,
    /// New message picked a person without a direct message: waiting for the
    /// provider to open one (workspace, person, name for a failure notice).
    pending_dm: Option<(WorkspaceKey, domain::UserId, String)>,
    /// The account being shown (or being signed in again).
    account: Option<WorkspaceKey>,
    /// Sign-in shown in the main window: only when there is no account yet.
    sign_in: Option<(Entity<SignInScreen>, Subscription)>,
    /// Sign-in in its own window (add a workspace, sign in again).
    sign_in_window: Option<SignInWindowState>,
    /// First-run screens around the main window's sign-in: Welcome, the
    /// provider picker and the first-sync progress.
    onboarding: Option<(Entity<Onboarding>, Subscription)>,
    /// Workspaces the first-sync screen waits for, those that could not
    /// start, and the observers of their sessions' progress.
    first_sync: Vec<WorkspaceKey>,
    first_sync_failed: Vec<WorkspaceKey>,
    first_sync_subs: Vec<Subscription>,
    /// The first-run tour over the main window.
    tour: Option<(Entity<Tour>, Subscription)>,
    /// Painted bounds of the rail and sidebar, which the tour spotlights.
    tour_targets: components::tour::TourTargets,
    /// Hit chosen in the search panel, revealed once its conversation is shown.
    pending_reveal: Option<MessageId>,
    /// The rail's workspace icon, from the provider's persisted account row.
    rail_icon: Option<domain::ImageRef>,
    /// The rail's own avatar, kept while its session, name and presence stay
    /// the same, so a redraw builds nothing for it.
    rail_self: Option<RailSelfCache>,
    /// The rail's callbacks, built once.
    rail_actions: Option<RailActions>,
    /// The header's callbacks for the shown session, built once per session.
    header_fns: std::cell::RefCell<Option<(gpui::EntityId, components::HeaderActions)>>,
    /// The workspace name and whether it was online when the icon was last read:
    /// the icon is read again when either changes (the provider refreshes the
    /// row in the background after it connects), not on every frame.
    rail_probe: Option<(Arc<str>, bool)>,
    /// Observers of the background workspaces: they redraw the rail only when
    /// a badge (mentions, unread, expired, lean) changes.
    rail_subs: Vec<Subscription>,
    rail_marks: std::collections::HashMap<WorkspaceKey, (u32, bool, bool, bool, bool)>,
    /// Per-workspace `accent` colours, read again when the settings change.
    rail_accents: Option<(
        u64,
        std::collections::HashMap<WorkspaceKey, Option<gpui::Hsla>>,
    )>,
    /// The call bar while a call runs.
    call_bar: Option<CallBarHost>,
    /// Focus for the lock screen's Unlock while the idle lock is on.
    lock_focus: Option<gpui::FocusHandle>,
}

impl RootView {
    /// Starts every registered account; the last active one is shown. Test
    /// helper: the app creates its workspaces first and calls
    /// [`RootView::with_workspaces`].
    #[doc(hidden)]
    pub fn new(
        registry: &ProviderRegistry,
        store: Arc<dyn Store>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let workspaces = cx.new(|cx| Workspaces::new(registry, store, cx));
        Self::with_workspaces(registry, workspaces, window, cx)
    }

    /// Like [`RootView::new`] over a manager that already exists (headless mode
    /// creates it before any window). Starts it unless it was started.
    pub fn with_workspaces(
        registry: &ProviderRegistry,
        workspaces: Entity<Workspaces>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sub = cx.subscribe_in(&workspaces, window, |this, _, ev, window, cx| {
            this.on_workspaces(ev, window, cx)
        });
        cx.set_global(WorkspacesHandle(workspaces.clone()));
        // Re-check notification permission when the window comes back to the
        // front (the user may have changed it in System Settings).
        crate::notifications::watch_activation(window, cx);
        // A click on an expired-session notification takes the same route as
        // the banner's "Sign in again" button, for the workspace that expired.
        if let Some(hub) = cx.try_global::<crate::notifications::Notifications>() {
            let weak = cx.weak_entity();
            hub.set_sign_in_opener(Rc::new(move |ws, cx| {
                if let Some(root) = weak.upgrade() {
                    root.update(cx, |this, cx| this.sign_in_again_for(ws, cx));
                }
            }));
        }
        let mut this = Self {
            phase: Phase::NoAccounts,
            ready: None,
            workspaces: workspaces.clone(),
            _workspaces_sub: sub,
            nav: NavHistory::new(),
            nav_target: None,
            switcher: None,
            status: None,
            delete_confirm: None,
            admin: None,
            aggregate: None,
            menu: None,
            command_palette: None,
            shortcuts_sheet: None,
            focus: cx.focus_handle(),
            rail_focus: cx.focus_handle(),
            registry: registry.clone(),
            notice: None,
            pending_dm: None,
            account: None,
            sign_in: None,
            sign_in_window: None,
            onboarding: None,
            first_sync: Vec::new(),
            first_sync_failed: Vec::new(),
            first_sync_subs: Vec::new(),
            tour: None,
            tour_targets: Default::default(),
            pending_reveal: None,
            rail_icon: None,
            rail_self: None,
            rail_actions: None,
            header_fns: Default::default(),
            rail_probe: None,
            rail_subs: Vec::new(),
            rail_marks: Default::default(),
            call_bar: None,
            rail_accents: None,
            lock_focus: None,
        };
        // The idle lock covers the window while it is on.
        if let Some(h) = cx.try_global::<crate::idle_lock::IdleLockHandle>() {
            cx.observe(&h.0.clone(), |_, _, cx| cx.notify()).detach();
        }
        // The ring banner appears and goes with the active ring.
        if let Some(r) = cx.try_global::<crate::ringer::Ringer>() {
            cx.observe(&r.state(), |_, _, cx| cx.notify()).detach();
        }
        // The call bar appears and goes with the call (no timer: the manager
        // notifies on each change). A change inside a running call (speaking,
        // mute, people) repaints the bar alone, not this whole view.
        if let Some(m) = cx.try_global::<crate::call_manager::CallManagerHandle>() {
            cx.observe(&m.0.clone(), |this, _, cx| this.call_changed(cx))
                .detach();
        }
        // Provider text (errors, the unofficial notice) renders from the
        // provider's own catalog.
        registry.register_catalogs(cx);
        // Closing the window keeps the app in the menu bar while the icon is on.
        crate::tray::Tray::guard_window(window, cx);
        // Quit saves the open drafts and waits for the store to have them.
        cx.on_app_quit(|this, cx| this.save_drafts(cx)).detach();
        // Nobody looks at a workspace while the window is not presented
        // (covered, minimised, hidden, display asleep): lean timers run. Focus
        // is not looking: a visible window behind another app stays awake.
        // The window may already be hidden at this point (a start with the
        // icon on opens it later), so sync before observing changes.
        let visible = window.is_visible();
        this.workspaces
            .update(cx, |w, cx| w.set_window_visible(visible, cx));
        cx.observe_window_visibility(window, |this, visibility, _, cx| {
            let visible = visibility.is_visible();
            this.workspaces
                .update(cx, |w, cx| w.set_window_visible(visible, cx));
            if visible {
                // Back from sleep or a hidden window: due local schedules go out.
                let sessions: Vec<_> = this.workspaces.read(cx).sessions().cloned().collect();
                for s in &sessions {
                    s.update(cx, |s, cx| s.local_tick(cx));
                }
                // The disk may have filled or emptied while away; the store
                // lets one read through per five minutes.
                if let Some(s) = sessions.first() {
                    s.update(cx, |s, cx| s.check_store_usage(cx));
                }
                // The header's local time of a direct message peer is read at
                // paint: show the current one when the window comes back.
                cx.notify();
            }
        })
        .detach();
        if registry.accounts().is_empty() {
            this.show_sign_in(true, window, cx);
            return this;
        }
        this.phase = Phase::Starting;
        if !workspaces.read(cx).is_started() {
            workspaces.update(cx, |w, cx| w.start_all(cx));
        }
        this.account = workspaces.read(cx).active().cloned();
        // Sessions that started before this window existed are shown now.
        if workspaces.read(cx).sessions().next().is_some() {
            this.sync_active(window, cx);
        }
        this
    }

    /// Reacts to the manager: shows the active session, reports a failed start
    /// of the active workspace.
    fn on_workspaces(&mut self, ev: &WorkspacesEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            WorkspacesEvent::Changed | WorkspacesEvent::Activated(_) => {
                // Picking a workspace (rail, keys) leaves the cross-workspace view.
                if matches!(ev, WorkspacesEvent::Activated(_)) {
                    self.aggregate = None;
                }
                self.sync_active(window, cx);
                if !self.first_sync.is_empty() {
                    self.watch_first_sync(cx);
                    // Showing the session moved the focus; the first-sync
                    // screen keeps it for Enter.
                    if let Some((ob, _)) = &self.onboarding {
                        window.focus(&ob.read(cx).focus_handle_for(cx), cx);
                    }
                }
            }
            WorkspacesEvent::LatePrompt(key) => self.ask_late(key.clone(), window, cx),
            WorkspacesEvent::UnknownProvider(key) => {
                crate::log::error(&format!("unknown provider {}", key.provider.0));
                self.phase = Phase::Failed(
                    i18n::LocalizedText::new("parla", "shell-unknown-provider")
                        .with("provider", key.provider.0.clone()),
                );
                // Keep sign-in reachable so a new account can replace it.
                if self.sign_in.is_none() {
                    self.show_sign_in(false, window, cx);
                }
                cx.notify();
            }
            WorkspacesEvent::StartFailed(key, e) => {
                if self.first_sync.contains(key) && !self.first_sync_failed.contains(key) {
                    self.first_sync_failed.push(key.clone());
                    self.refresh_first_sync(cx);
                }
                // Expired while Parla was closed: no session ever ran, so tell
                // the user here (the hub dedupes per workspace).
                if matches!(e, ProviderError::SessionExpired)
                    && let Some(hub) = cx
                        .try_global::<crate::notifications::Notifications>()
                        .cloned()
                {
                    let notice = crate::notifications::Notifications::expired_notice(key, "", cx);
                    hub.session_expired(notice, cx);
                }
                if self.account.as_ref() != Some(key) {
                    crate::log::error(&format!("start failed for a background workspace: {e}"));
                    return;
                }
                match e {
                    ProviderError::SessionExpired => {
                        crate::log::info("session expired at start");
                        self.phase = Phase::Expired;
                    }
                    e => {
                        crate::log::error(&format!("start failed: {e}"));
                        self.phase = Phase::Failed(crate::error_text::provider_error(e));
                    }
                }
                cx.notify();
            }
            WorkspacesEvent::OpenLink(url) => {
                self.open_link_url(url.clone(), window, cx);
            }
        }
    }

    /// Makes the view show the manager's active session.
    fn sync_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.observe_background(cx);
        let (active, session) = {
            let w = self.workspaces.read(cx);
            (w.active().cloned(), w.active_session())
        };
        match (active, session) {
            (None, _) => {
                if self.ready.is_some() || self.phase != Phase::NoAccounts {
                    self.ready = None;
                    self.account = None;
                    self.phase = Phase::NoAccounts;
                    if self.sign_in.is_none() {
                        self.show_sign_in(true, window, cx);
                    }
                }
            }
            (Some(key), Some(session)) => {
                self.account = Some(key);
                if self.ready.as_ref().is_none_or(|r| r.session != session) {
                    self.show_session(session, window, cx);
                }
            }
            (Some(key), None) => {
                self.account = Some(key);
                if self.ready.take().is_some() {
                    self.phase = Phase::Starting;
                }
            }
        }
        cx.notify();
    }

    /// Watches every workspace but the active one (which the view observes
    /// already) for badge changes.
    fn observe_background(&mut self, cx: &mut Context<Self>) {
        self.rail_subs.clear();
        let (active, sessions): (Option<WorkspaceKey>, Vec<_>) = {
            let w = self.workspaces.read(cx);
            (
                w.active().cloned(),
                w.accounts()
                    .iter()
                    .filter_map(|a| w.session(&a.key).map(|s| (a.key.clone(), s)))
                    .collect(),
            )
        };
        for (key, session) in sessions {
            if active.as_ref() == Some(&key) {
                continue;
            }
            let sub = cx.observe(&session, move |this, session, cx| {
                let s = session.read(cx).state();
                let t = s.totals();
                let marks = (
                    t.mentions,
                    t.conversations > 0,
                    s.session == Session::Expired,
                    s.lean,
                    s.self_user
                        .as_ref()
                        .and_then(|me| s.dnd.get(me))
                        .is_some_and(|d| d.active(components::now_secs(cx))),
                );
                if this.rail_marks.insert(key.clone(), marks) != Some(marks) {
                    cx.notify();
                }
            });
            self.rail_subs.push(sub);
        }
    }

    /// The signed-in user of the shown workspace, for the bottom of the rail:
    /// a click opens the own profile (status and presence). Rebuilt only when
    /// the session, the name or the presence changes.
    fn rail_self(&mut self, cx: &App) -> Option<components::rail::RailSelf> {
        let session = self.ready.as_ref()?.session.clone();
        let state = session.read(cx).state();
        let me = state.self_user.as_ref()?;
        let name = state
            .members
            .get(me)
            .and_then(components::sidebar::best_name)
            .or_else(|| {
                state
                    .profiles
                    .get(me)
                    .map(|p| p.display_name.clone())
                    .filter(|n| !n.is_empty())
            })?;
        let active = state.presence.get(me).copied().or(state
            .presence_setting
            .map(|p| p == domain::PresenceSetting::Auto));
        let avatar = state.members.get(me).and_then(|m| {
            m.avatar_for(themes::metrics::LAYOUT_MSG_AVATAR.as_f32())
                .cloned()
        });
        let id = session.entity_id();
        if let Some(c) = self.rail_self.as_ref().filter(|c| {
            c.session == id && c.name == name && c.active == active && c.avatar == avatar
        }) {
            return Some(c.item.clone());
        }
        let me = me.clone();
        let item = components::rail::RailSelf::new(
            name.clone().into(),
            active,
            std::rc::Rc::new(move |_, cx| {
                let me = me.clone();
                session.update(cx, |s, cx| {
                    components::TimelineSource::open_profile(s, me, cx)
                });
            }),
        )
        .with_avatar(avatar.clone());
        self.rail_self = Some(RailSelfCache {
            session: id,
            name,
            active,
            avatar,
            item: item.clone(),
        });
        Some(item)
    }

    /// The rail's entries from the manager's accounts and sessions.
    fn rail_items(&mut self, cx: &App) -> Vec<RailItem> {
        let generation = cx.try_global::<SettingsStore>().map(|s| s.generation());
        let w = self.workspaces.read(cx);
        let mut accounts = w.accounts().to_vec();
        let active = w.active().cloned();
        // The active workspace's icon is the freshly read account row's.
        if let Some(icon) = &self.rail_icon {
            for a in accounts
                .iter_mut()
                .filter(|a| Some(&a.key) == active.as_ref())
            {
                a.icon = Some(icon.clone());
            }
        }
        let stale = self.rail_accents.as_ref().is_none_or(|(g, m)| {
            Some(*g) != generation || accounts.iter().any(|a| !m.contains_key(&a.key))
        });
        if stale {
            let map = cx
                .try_global::<SettingsStore>()
                .map(|store| {
                    accounts
                        .iter()
                        .map(|a| {
                            let accent = store
                                .for_workspace(&a.key)
                                .accent
                                .as_deref()
                                .and_then(components::rail::parse_accent);
                            (a.key.clone(), accent)
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.rail_accents = Some((generation.unwrap_or(0), map));
        }
        let accents = self.rail_accents.as_ref().map(|(_, m)| m);
        let sessions: Vec<_> = accounts
            .iter()
            .filter_map(|a| w.session(&a.key).map(|s| (a.key.clone(), s)))
            .collect();
        let states: Vec<_> = sessions
            .iter()
            .map(|(k, s)| (k.clone(), s.read(cx).state()))
            .collect();
        components::rail::rail_items(
            &accounts,
            &states,
            active.as_ref(),
            &|k| accents.and_then(|m| m.get(k).copied().flatten()),
            components::now_secs(cx),
        )
    }

    /// `workspace::ActivateWorkspace1` .. `9`, `NextWorkspace`, `PreviousWorkspace`.
    fn switch_workspace(&mut self, target: Option<WorkspaceKey>, cx: &mut Context<Self>) {
        if let Some(key) = target {
            self.workspaces.update(cx, |w, cx| w.activate(&key, cx));
        }
    }

    /// Starts `ws` (a new sign-in or a re-sign-in) and makes it the active one.
    fn start_account(&mut self, ws: WorkspaceKey, cx: &mut Context<Self>) {
        // A successful sign-in releases the expiry latch, so a later expiry of
        // this workspace may notify again.
        if let Some(hub) = cx
            .try_global::<crate::notifications::Notifications>()
            .cloned()
        {
            hub.signed_in(&ws, cx);
        }
        self.account = Some(ws.clone());
        self.phase = Phase::Starting;
        self.workspaces.update(cx, |w, cx| w.adopt(ws, cx));
        cx.notify();
    }

    /// The error shown in the window, if any.
    pub fn error_banner(&self, cx: &App) -> Option<String> {
        match (&self.phase, &self.ready) {
            (Phase::Failed(msg), _) => Some(i18n::gpui::localizer(cx).text(msg).into_owned()),
            (Phase::Ready, Some(r)) => r
                .session
                .read(cx)
                .bootstrap_error()
                .map(|e| load_failed_text(cx, e)),
            _ => None,
        }
    }

    /// Whether the window shows "Still connecting" (the first bootstrap has
    /// not answered yet and no error is shown).
    pub fn connecting(&self, cx: &App) -> bool {
        match (&self.phase, &self.ready) {
            (Phase::Ready, Some(r)) => r.session.read(cx).connecting(),
            _ => false,
        }
    }

    /// The open sign-in screen, if any.
    pub fn sign_in_screen(&self) -> Option<&Entity<SignInScreen>> {
        self.sign_in
            .as_ref()
            .map(|(s, _)| s)
            .or(self.sign_in_window.as_ref().map(|w| &w.screen))
    }

    /// The name the expiry banner uses for the active workspace.
    fn expired_workspace_name(&self, cx: &App) -> String {
        match &self.account {
            Some(ws) => crate::notifications::Notifications::expired_notice(ws, "", cx)
                .workspace_name
                .to_string(),
            None => String::new(),
        }
    }

    /// Whether the "offline" banner is shown.
    pub fn offline_banner_visible(&self, cx: &App) -> bool {
        match (&self.phase, &self.ready) {
            (Phase::Ready, Some(r)) => {
                let s = r.session.read(cx).state();
                // A failed bootstrap has its own strip with Retry; "Retry now"
                // only reconnects an established session.
                s.session != Session::Expired
                    && s.connection == domain::Connection::Offline
                    && r.session.read(cx).bootstrap_error().is_none()
            }
            _ => false,
        }
    }

    /// Whether the "session expired" banner is shown.
    pub fn expiry_banner_visible(&self, cx: &App) -> bool {
        match (&self.phase, &self.ready) {
            (Phase::Expired, _) => true,
            (Phase::Ready, Some(r)) => r.session.read(cx).state().session == Session::Expired,
            _ => false,
        }
    }

    /// Shows the sign-in in the main window; with `welcome`, behind the
    /// first-run Welcome and provider screens.
    fn show_sign_in(&mut self, welcome: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signin) = self.registry.sign_in() else {
            return;
        };
        let (screen, sub) = self.main_sign_in(signin, cx);
        let focus = screen.read(cx).focus_handle(cx);
        self.sign_in = Some((screen.clone(), sub));
        self.end_onboarding();
        if welcome {
            let providers = self
                .registry
                .sign_ins()
                .iter()
                .map(|s| ProviderCard {
                    name: s.provider_name(),
                })
                .collect();
            let ob = cx.new(|cx| Onboarding::new(providers, screen, cx));
            if let Some(store) = cx.try_global::<SettingsStore>() {
                let locale = store.current().locale.clone();
                ob.update(cx, |o, cx| o.set_language(locale, cx));
            }
            let sub =
                cx.subscribe_in(
                    &ob,
                    window,
                    |this, _, ev: &OnboardingEvent, window, cx| match ev {
                        OnboardingEvent::ProviderChosen(i) => this.choose_provider(*i, cx),
                        OnboardingEvent::Finished => this.finish_onboarding(window, cx),
                        OnboardingEvent::LanguageChosen(l) => Self::write_locale(l.clone(), cx),
                        OnboardingEvent::Dismissed => {}
                    },
                );
            window.focus(&ob.read(cx).focus_handle_for(cx), cx);
            self.onboarding = Some((ob, sub));
        } else {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// Writes the `locale` setting; the settings store applies it live.
    fn write_locale(locale: settings::LocaleSetting, cx: &mut Context<Self>) {
        let path = crate::app_state::settings_path(cx);
        if let Err(e) = crate::settings_window::GeneralChange::Locale(locale).write(&path) {
            crate::log::error(&format!("settings: cannot write the language: {e}"));
        }
    }

    /// The provider picker moved on: with more than one provider, the sign-in
    /// step shows the chosen provider's screen.
    fn choose_provider(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.registry.sign_ins().len() < 2 {
            return;
        }
        let Some(signin) = self.registry.sign_ins().get(index).cloned() else {
            return;
        };
        let (screen, sub) = self.main_sign_in(signin, cx);
        if let Some((ob, _)) = &self.onboarding {
            ob.update(cx, |o, cx| o.set_sign_in(screen.clone(), cx));
        }
        self.sign_in = Some((screen, sub));
    }

    /// A sign-in screen for the main window and what its events do.
    fn main_sign_in(
        &mut self,
        signin: Rc<dyn components::sign_in::SignIn>,
        cx: &mut Context<Self>,
    ) -> (Entity<SignInScreen>, Subscription) {
        signin.set_top_inset(f32::from(titlebar::top_inset()));
        let screen = cx.new(|cx| SignInScreen::new(signin, cx));
        let sub = cx.subscribe(&screen, |this, _, ev: &SignInEvent, cx| {
            match ev {
                SignInEvent::SignedIn(ws) => {
                    this.sign_in = None;
                    this.start_account(ws.clone(), cx);
                    this.begin_first_sync(vec![ws.clone()], cx);
                }
                SignInEvent::Added(keys) => {
                    this.sign_in = None;
                    for ws in keys {
                        this.start_account(ws.clone(), cx);
                    }
                    this.begin_first_sync(keys.clone(), cx);
                }
                SignInEvent::Cancelled => {
                    // Keep the signed-out screen; close it over an existing account.
                    if this.account.is_some() {
                        this.sign_in = None;
                    }
                }
            }
            cx.notify();
        });
        (screen, sub)
    }

    /// The first-run screens, while they show (Welcome to first sync).
    pub fn onboarding(&self) -> Option<&Entity<Onboarding>> {
        self.onboarding.as_ref().map(|(o, _)| o)
    }

    /// The Welcome and provider screens of the add-workspace window, while it
    /// shows them.
    pub fn sign_in_window_onboarding(&self) -> Option<&Entity<Onboarding>> {
        self.sign_in_window
            .as_ref()
            .and_then(|w| w.onboarding.as_ref())
    }

    /// The first-run tour, while it shows.
    pub fn tour(&self) -> Option<&Entity<Tour>> {
        self.tour.as_ref().map(|(t, _)| t)
    }

    /// The first-run sign-in added `keys`: the first-sync screen follows them.
    fn begin_first_sync(&mut self, keys: Vec<WorkspaceKey>, cx: &mut Context<Self>) {
        let Some((ob, _)) = &self.onboarding else {
            return;
        };
        let names = keys.iter().map(|k| self.workspace_name(k, cx)).collect();
        ob.update(cx, |o, cx| o.start_first_sync(names, cx));
        self.first_sync = keys;
        self.first_sync_failed.clear();
        self.watch_first_sync(cx);
    }

    /// The name of `key` from its session, its account row or its id.
    fn workspace_name(&self, key: &WorkspaceKey, cx: &App) -> String {
        let w = self.workspaces.read(cx);
        w.session(key)
            .map(|s| s.read(cx).state().name.to_string())
            .filter(|n| !n.is_empty())
            .or_else(|| {
                w.accounts()
                    .iter()
                    .find(|a| &a.key == key)
                    .map(|a| a.name.to_string())
                    .filter(|n| !n.is_empty())
            })
            .unwrap_or_else(|| key.id.0.clone())
    }

    /// Follows the progress events of the first-sync workspaces' sessions
    /// (no timer: each session says when its sync moved).
    fn watch_first_sync(&mut self, cx: &mut Context<Self>) {
        let sessions: Vec<_> = {
            let w = self.workspaces.read(cx);
            self.first_sync
                .iter()
                .filter_map(|k| w.session(k))
                .collect()
        };
        self.first_sync_subs = sessions
            .iter()
            .map(|s| {
                cx.subscribe(s, |this, _, ev: &SessionEvent, cx| {
                    if matches!(ev, SessionEvent::SyncProgress | SessionEvent::Conversations) {
                        this.refresh_first_sync(cx);
                    }
                })
            })
            .collect();
        self.refresh_first_sync(cx);
    }

    /// Hands the first-sync screen every workspace's latest progress.
    fn refresh_first_sync(&mut self, cx: &mut Context<Self>) {
        let Some((ob, _)) = &self.onboarding else {
            return;
        };
        let list: Vec<WorkspaceSync> = self
            .first_sync
            .iter()
            .map(|k| {
                let progress = match self.workspaces.read(cx).session(k) {
                    Some(s) => s.read(cx).sync_progress().clone(),
                    None => SyncProgress {
                        failed: self.first_sync_failed.contains(k),
                        ..Default::default()
                    },
                };
                WorkspaceSync {
                    name: self.workspace_name(k, cx),
                    progress,
                }
            })
            .collect();
        ob.update(cx, |o, cx| o.set_sync(list, cx));
    }

    /// Drops the first-run screens and everything the first sync followed, so
    /// a later workspace event neither moves the focus nor watches old sessions.
    fn end_onboarding(&mut self) {
        self.onboarding = None;
        self.first_sync.clear();
        self.first_sync_failed.clear();
        self.first_sync_subs.clear();
    }

    /// Workspaces the first-sync screen still follows (tests).
    pub fn first_sync_pending(&self) -> usize {
        self.first_sync.len()
    }

    /// "Open Parla" after the first sync: the main window, then the tour the
    /// first time.
    fn finish_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_onboarding();
        if crate::tour_state::dismissed(cx) {
            self.focus_chat_or_root(window, cx);
        } else {
            self.show_tour(window, cx);
        }
        cx.notify();
    }

    fn show_tour(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mac = cfg!(target_os = "macos");
        let rows = crate::shortcuts::current(cx);
        let keys_for = |action: &str| {
            rows.iter()
                .find(|r| r.action == action)
                .and_then(|r| r.keystrokes.clone())
                .map(|k| crate::shortcuts::key_caps(&k, mac).join(if mac { "" } else { "+" }))
                .unwrap_or_default()
        };
        let keys = TourKeys {
            workspace_first: keys_for("workspace::ActivateWorkspace1"),
            workspace_last: keys_for("workspace::ActivateWorkspace9"),
            switcher: keys_for("workspace::QuickSwitch"),
            shortcuts: keys_for("app::OpenKeyboardShortcuts"),
        };
        let layout = TourLayout {
            top: if mac {
                themes::metrics::LAYOUT_TITLEBAR
            } else {
                themes::metrics::SPACE_0
            },
            rail: themes::metrics::LAYOUT_RAIL,
            sidebar: themes::metrics::LAYOUT_SIDEBAR,
        };
        let targets = self.tour_targets.clone();
        let tour = cx.new(|cx| {
            Tour::new(layout, cx)
                .with_keys(keys)
                .with_targets(targets, cx)
        });
        let sub = cx.subscribe_in(&tour, window, |this, _, ev: &TourEvent, window, cx| {
            match ev {
                TourEvent::Dismissed => {
                    this.tour = None;
                    this.tour_targets.clear();
                    crate::tour_state::dismiss(cx);
                    this.focus_chat_or_root(window, cx);
                }
            }
            cx.notify();
        });
        window.focus(&tour.read(cx).focus_handle_for(), cx);
        self.tour = Some((tour, sub));
    }

    /// Rail "add workspace": its own window starts at the first-run Welcome
    /// and provider screens; an open one is brought to the front. With no
    /// account the main window shows them.
    pub fn add_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sign_in.is_some() {
            return;
        }
        if self.ready.is_none() && self.account.is_none() {
            self.show_sign_in(true, window, cx);
            return;
        }
        self.open_sign_in_window(true, cx);
    }

    /// `auth::SignInAgain`: opens the sign-in screen for the expired account,
    /// without the Welcome (the provider is known).
    pub fn sign_in_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sign_in.is_some() {
            return;
        }
        if self.ready.is_none() && self.account.is_none() {
            self.show_sign_in(true, window, cx);
            return;
        }
        self.open_sign_in_window(false, cx);
    }

    /// A click on an expired-session notification: bring the workspace whose
    /// session ended forward, then open the same sign-in window the banner's
    /// "Sign in again" button uses.
    pub fn sign_in_again_for(&mut self, ws: &WorkspaceKey, cx: &mut Context<Self>) {
        self.workspaces.update(cx, |w, cx| w.activate(ws, cx));
        self.open_sign_in_window(false, cx);
    }

    /// What a sign-in screen of the add-workspace window does when it ends.
    fn watch_window_screen(
        &mut self,
        screen: &Entity<SignInScreen>,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe(screen, |this, _, ev: &SignInEvent, cx| {
            match ev {
                SignInEvent::SignedIn(ws) => this.start_account(ws.clone(), cx),
                SignInEvent::Added(keys) => {
                    for ws in keys {
                        this.start_account(ws.clone(), cx);
                    }
                }
                SignInEvent::Cancelled => {}
            }
            this.close_sign_in_window(cx);
        })
    }

    /// The window's provider picker chose a provider: its sign-in screen
    /// replaces the shown one.
    fn choose_window_provider(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.registry.sign_ins().len() < 2 {
            return;
        }
        let Some(signin) = self.registry.sign_ins().get(index).cloned() else {
            return;
        };
        signin.set_top_inset(f32::from(titlebar::top_inset()));
        let screen = cx.new(|cx| SignInScreen::new(signin, cx));
        let events = self.watch_window_screen(&screen, cx);
        let Some(open) = &mut self.sign_in_window else {
            return;
        };
        let handle = open.handle;
        if let Some(ob) = &open.onboarding {
            ob.update(cx, |o, cx| o.set_sign_in(screen.clone(), cx));
        }
        open.screen = screen.clone();
        open.events = events;
        // The window's own copy follows, so the dropped screen is freed, and
        // the new screen takes the focus.
        let _ = handle.update(cx, |w, window, cx| {
            w.set_screen(screen.clone());
            let focus = screen.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        });
    }

    /// Opens the add-workspace window, behind the Welcome and provider screens
    /// with `welcome`; an open one is brought to the front. No first sync or
    /// tour follows: the workspace joins the main window.
    fn open_sign_in_window(&mut self, welcome: bool, cx: &mut Context<Self>) {
        if let Some(open) = &self.sign_in_window {
            let _ = open
                .handle
                .update(cx, |_, window, _| window.activate_window());
            return;
        }
        let Some(signin) = self.registry.sign_in() else {
            return;
        };
        signin.set_top_inset(f32::from(titlebar::top_inset()));
        let providers: Vec<ProviderCard> = self
            .registry
            .sign_ins()
            .iter()
            .map(|s| ProviderCard {
                name: s.provider_name(),
            })
            .collect();
        let Some(handle) = crate::sign_in_window::open(cx, welcome, |_window, cx| {
            let screen = cx.new(|cx| SignInScreen::new(signin, cx));
            let ob = welcome.then(|| {
                let ob = cx
                    .new(|cx| Onboarding::new(providers, screen.clone(), cx).for_added_workspace());
                if let Some(store) = cx.try_global::<SettingsStore>() {
                    let locale = store.current().locale.clone();
                    ob.update(cx, |o, cx| o.set_language(locale, cx));
                }
                ob
            });
            (screen, ob)
        }) else {
            return;
        };
        let Ok(view) = handle.entity(cx) else {
            return;
        };
        let screen = view.read(cx).screen().clone();
        let onboarding = view.read(cx).onboarding().cloned();
        let events = self.watch_window_screen(&screen, cx);
        let mut subs = Vec::new();
        if let Some(ob) = &onboarding {
            subs.push(
                cx.subscribe(ob, |this, _, ev: &OnboardingEvent, cx| match ev {
                    OnboardingEvent::ProviderChosen(i) => this.choose_window_provider(*i, cx),
                    OnboardingEvent::LanguageChosen(l) => Self::write_locale(l.clone(), cx),
                    OnboardingEvent::Dismissed => this.close_sign_in_window(cx),
                    OnboardingEvent::Finished => {}
                }),
            );
        }
        // Closing the window by hand ends the flow too.
        subs.push(cx.observe_release(&view, |this, _, cx| {
            if let Some(open) = this.sign_in_window.take() {
                let reported = open.screen.update(cx, |s, _| s.take_reported_refresh());
                for ws in reported {
                    this.start_account(ws, cx);
                }
            }
            cx.notify();
        }));
        self.sign_in_window = Some(SignInWindowState {
            handle,
            screen,
            onboarding,
            events,
            _subs: subs,
        });
        cx.activate(true);
    }

    fn close_sign_in_window(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.sign_in_window.take() {
            let _ = open
                .handle
                .update(cx, |_, window, _| window.remove_window());
        }
        cx.notify();
    }

    /// `workspace::RemoveWorkspace`: after a confirmation naming the workspace,
    /// signs it out and deletes its local data (database, key, media cache).
    pub fn remove_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.account.clone() else {
            return;
        };
        self.remove_workspace_key(key, window, cx);
    }

    /// The same for a chosen workspace (the rail's context menu).
    fn remove_workspace_key(
        &mut self,
        key: WorkspaceKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::media_viewer::close(cx);
        crate::code_viewer::close(cx);
        let name = {
            let w = self.workspaces.read(cx);
            w.accounts()
                .iter()
                .find(|a| a.key == key)
                .map(|a| a.name.to_string())
                .unwrap_or_else(|| key.id.0.clone())
        };
        let l = i18n::gpui::localizer(cx);
        let (title, detail, confirm) = (
            t!(l, "shell-remove-title", name = name).into_owned(),
            t!(l, "shell-remove-detail").into_owned(),
            t!(l, "shell-remove-confirm").into_owned(),
        );
        self.confirm(title, detail, confirm, window, cx, move |this, cx| {
            let task = this.workspaces.update(cx, |w, cx| w.remove(&key, cx));
            this.finish_removal(task, cx);
        });
    }

    /// `app::DeleteAllData`: after a confirmation, removes every workspace and
    /// deletes the data and cache dirs (never the config dir), then the window
    /// shows the sign-in screen.
    pub fn delete_all_data(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let l = i18n::gpui::localizer(cx);
        let (title, detail, confirm) = (
            t!(l, "shell-delete-all-title").into_owned(),
            t!(l, "shell-delete-all-detail").into_owned(),
            t!(l, "shell-delete-all-confirm").into_owned(),
        );
        self.confirm(title, detail, confirm, window, cx, |this, cx| {
            this.delete_all_now(cx)
        });
    }

    /// The "Delete all data" removal itself, for a caller that already asked
    /// (the Settings window's danger zone).
    pub fn delete_all_now(&mut self, cx: &mut Context<Self>) {
        let task = self.workspaces.update(cx, |w, cx| w.delete_all(cx));
        self.finish_removal(task, cx);
    }

    /// Asks `title` with Cancel (the default) and `confirm`; runs `then` only
    /// on `confirm`.
    fn confirm(
        &mut self,
        title: String,
        detail: String,
        confirm: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let cancel = t!(cx, "shell-cancel").into_owned();
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &title,
            Some(&detail),
            &[cancel.as_str(), confirm.as_str()],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(1) {
                this.update(cx, |this, cx| then(this, cx)).ok();
            }
        })
        .detach();
    }

    /// Shows a removal that failed part way (it is safe to run again).
    fn finish_removal(
        &mut self,
        task: Task<Result<(), crate::providers::RemoveError>>,
        cx: &mut Context<Self>,
    ) {
        self.notice = None;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if let Err(e) = result {
                    let text = match &e {
                        crate::providers::RemoveError::Running => {
                            i18n::LocalizedText::new("parla", "error-removal-running")
                        }
                        crate::providers::RemoveError::Provider(pe) => {
                            crate::error_text::provider_error(pe)
                        }
                    };
                    crate::log::error("removal failed");
                    let l = i18n::gpui::localizer(cx);
                    let why = l.text(&text).into_owned();
                    this.notice = Some(t!(l, "shell-removal-failed", reason = why).into_owned());
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// `auth::SignOut`: deletes the stored secrets and account, then shows sign-in.
    pub fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.account.clone() else {
            return;
        };
        self.sign_out_key(ws, window, cx);
    }

    /// Signs `ws` out, the shown workspace or not (the Settings window's
    /// workspace list, after its own confirmation).
    pub fn sign_out_key(&mut self, ws: WorkspaceKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(signin) = self.registry.sign_in() else {
            return;
        };
        crate::media_viewer::close(cx);
        crate::code_viewer::close(cx);
        let task = signin.sign_out(&ws, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, _window, cx| match result {
                Ok(()) => {
                    // The manager drops the session and shows the next account,
                    // or the sign-in screen when none is left.
                    this.workspaces.update(cx, |w, cx| w.forget(&ws, cx));
                }
                Err(e) => {
                    this.phase = Phase::Failed(
                        i18n::LocalizedText::new("parla", "error-detail").with("detail", e),
                    );
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The workspace icon the rail shows (the persisted account row's).
    pub fn rail_icon(&self) -> Option<&domain::ImageRef> {
        self.rail_icon.as_ref()
    }

    pub fn sidebar(&self) -> Option<&Entity<Sidebar<WorkspaceSession>>> {
        self.ready.as_ref().map(|r| &r.sidebar)
    }

    pub fn session(&self) -> Option<&Entity<WorkspaceSession>> {
        self.ready.as_ref().map(|r| &r.session)
    }

    /// The conversation the timeline currently shows.
    pub fn shown_conversation(&self) -> Option<&ConversationId> {
        self.ready
            .as_ref()
            .and_then(|r| r.chat.as_ref())
            .map(|c| &c.conversation)
    }

    /// Shows `session` (the manager's active one): sidebar, timeline and composer.
    fn show_session(
        &mut self,
        session: Entity<WorkspaceSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // An admin dialog acts on the workspace it opened in: it goes with it.
        self.admin = None;
        let sidebar = cx.new(|cx| Sidebar::new(session.clone(), cx));
        let sub = cx.observe_in(&session, window, |this, _, window, cx| {
            this.refresh_rail_icon(cx);
            this.sync_chat(window, cx);
            this.record_visit(cx);
            this.apply_pending_reveal(cx);
            cx.notify();
        });
        let events = cx.subscribe_in(
            &session,
            window,
            |this, _, ev: &SessionEvent, window, cx| match ev {
                SessionEvent::ShowScheduled => this.open_scheduled(window, cx),
                SessionEvent::ShowReminders => this.open_reminders(window, cx),
                SessionEvent::Picked(_) => {
                    // Picking a conversation leaves Threads, Later and the pins.
                    if let Some(r) = this.ready.as_mut() {
                        r.collection = None;
                        r.pins = None;
                    }
                    this.focus_composer(window, cx);
                    cx.notify();
                }
                SessionEvent::ShowThreads => this.open_collection(true, window, cx),
                SessionEvent::ShowLater => this.open_collection(false, window, cx),
                SessionEvent::ShowDrafts => this.open_drafts(window, cx),
                SessionEvent::ShowPeople => this.open_people(window, cx),
                SessionEvent::DraftDeleted(key) => this.clear_deleted_draft(key, cx),
                SessionEvent::DirectOpened(u) => this.direct_opened(u, window, cx),
                SessionEvent::DirectFailed(u, why) => this.direct_failed(u, why, cx),
                SessionEvent::ShowProfile => this.open_profile_panel(window, cx),
                SessionEvent::AdminFinished { call, result } => {
                    this.admin_finished(call, result, window, cx)
                }
                _ => {}
            },
        );
        let sidebar_events = cx.subscribe_in(
            &sidebar,
            window,
            |this, _, ev: &components::SidebarEvent, window, cx| match ev {
                components::SidebarEvent::ContextMenu {
                    conversation,
                    position,
                } => this.open_conversation_menu(conversation.clone(), *position, window, cx),
                components::SidebarEvent::QuickSwitch => this.open_quick_switcher(window, cx),
                components::SidebarEvent::NewMessage => {
                    this.open_switcher(components::SwitchScope::Compose, window, cx)
                }
                components::SidebarEvent::BrowseChannels => this.browse_channels(window, cx),
                components::SidebarEvent::BrowsePeople => {
                    this.open_switcher(components::SwitchScope::People, window, cx)
                }
                components::SidebarEvent::WorkspaceMenu { position } => {
                    this.open_sidebar_workspace_menu(*position, window, cx)
                }
            },
        );
        // Unsent text and the old thread viewer go with the old workspace.
        if self.ready.is_some() {
            self.save_drafts(cx).detach();
            if let Some(old) = self.ready.as_mut() {
                old.close_right(cx);
            }
        }
        self.ready = Some(Ready {
            session,
            sidebar,
            chat: None,
            right: None,
            collection: None,
            pins: None,
            people_back: None,
            _sub: sub,
            _events: events,
            _sidebar_events: sidebar_events,
        });
        self.phase = Phase::Ready;
        self.rail_probe = None;
        self.refresh_rail_icon(cx);
        // A restored session may already have a conversation open.
        self.sync_chat(window, cx);
        self.record_visit(cx);
        self.apply_pending_reveal(cx);
        if self.ready.as_ref().is_some_and(|r| r.chat.is_none()) {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// Reads the workspace icon again from the account row when the workspace
    /// name or connection changed since the last read (a few times per start).
    fn refresh_rail_icon(&mut self, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_ref() else {
            return;
        };
        let probe = {
            let state = ready.session.read(cx).state();
            (
                state.name.clone(),
                matches!(state.connection, domain::Connection::Online),
            )
        };
        if self.rail_probe.as_ref() == Some(&probe) {
            return;
        }
        self.rail_probe = Some(probe);
        let Some(ws) = self.account.clone() else {
            return;
        };
        self.rail_icon = self
            .registry
            .factory_for(&ws)
            .and_then(|f| f.account_info().into_iter().find(|a| a.key == ws))
            .and_then(|a| a.icon);
    }

    /// Perf driver: fills the composer with `text` and focuses it.
    #[cfg(feature = "fixture")]
    pub fn perf_prepare_composer(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.ready.as_ref().and_then(|r| r.chat.as_ref()) else {
            return;
        };
        let composer = chat.composer.clone();
        composer.update(cx, |c, cx| c.set_text(text.to_string(), cx));
        let focus = composer.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Shows the timeline and composer for the session's selected conversation.
    fn sync_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        let Some(selected) = ready.session.read(cx).selected().cloned() else {
            // The open conversation went away (left, archived) with nothing
            // to show next: no composer stays on it.
            if ready.chat.take().is_some() {
                window.focus(&self.focus, cx);
            }
            return;
        };
        if let Some(chat) = &mut ready.chat {
            if chat.conversation != selected {
                // Save the draft of the conversation being left, restore the new one.
                let left = chat.composer.read(cx);
                if left.editing().is_none() {
                    let key = DraftKey {
                        conversation: chat.conversation.clone(),
                        thread: None,
                    };
                    let text = left.composed(cx);
                    ready.session.update(cx, |s, cx| s.set_draft(key, text, cx));
                }
                chat.conversation = selected.clone();
                // Another conversation leaves Threads, Later and the pins.
                ready.collection = None;
                ready.pins = None;
                if let Some(b) = &chat.bookmarks {
                    b.update(cx, |b, cx| b.set_conversation(selected.clone(), cx));
                }
                if let Some(h) = &chat.huddle {
                    h.update(cx, |h, cx| h.set_conversation(selected.clone(), cx));
                }
                chat.timeline
                    .update(cx, |t, cx| t.set_conversation(selected.clone(), cx));
                chat.composer
                    .update(cx, |c, cx| c.set_conversation(selected.clone(), cx));
                restore_draft(&ready.session, &chat.composer, &selected, cx);
            }
        } else {
            let session = ready.session.clone();
            let open_url = {
                let root = cx.entity().downgrade();
                Rc::new(move |url: String, window: &mut Window, cx: &mut App| {
                    root.update(cx, |root, cx| root.open_link_url(url, window, cx))
                        .ok();
                })
            };
            let pane = chat_pane::ChatPane::new(
                session,
                selected,
                window,
                cx,
                Some(open_url),
                |this, root, window, cx| this.open_thread_panel(root, window, cx),
            );
            let focus = pane.composer.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            ready.chat = Some(Chat {
                conversation: pane.conversation,
                bookmarks: pane.bookmarks,
                huddle: pane.huddle,
                timeline: pane.timeline,
                composer: pane.composer,
                files: None,
                _subs: pane.subs,
            });
        }
    }

    /// Shows the Messages or Files tab of the open conversation and moves the
    /// focus into it (the composer, or the file list).
    pub fn show_tab(
        &mut self,
        tab: components::ConversationTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        let session = ready.session.clone();
        let Some(chat) = ready.chat.as_mut() else {
            return;
        };
        match tab {
            components::ConversationTab::Files => {
                let c = chat.conversation.clone();
                let files = cx.new(|cx| components::FilesTab::new(session, c, cx));
                let focus = files.read(cx).focus_handle();
                chat.files = Some(files);
                window.focus(&focus, cx);
            }
            components::ConversationTab::Messages => {
                chat.files = None;
                let focus = chat.composer.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
        }
        cx.notify();
    }

    /// The tab the open conversation shows.
    pub fn conversation_tab(&self) -> components::ConversationTab {
        match self.ready.as_ref().and_then(|r| r.chat.as_ref()) {
            Some(c) if c.files.is_some() => components::ConversationTab::Files,
            _ => components::ConversationTab::Messages,
        }
    }

    /// The composer under the timeline, once a conversation is shown.
    pub fn composer(&self) -> Option<&Entity<Composer<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| r.chat.as_ref())
            .map(|c| &c.composer)
    }

    /// Saves the conversation and thread composer texts as drafts (blank
    /// deletes). The task resolves once the store has them; quit awaits it.
    pub fn save_drafts(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let Some(ready) = &self.ready else {
            return Task::ready(());
        };
        let session = ready.session.clone();
        let mut pending = Vec::new();
        if let Some(chat) = &ready.chat {
            let c = chat.composer.read(cx);
            if c.editing().is_none() {
                let key = DraftKey {
                    conversation: chat.conversation.clone(),
                    thread: None,
                };
                let text = c.composed(cx);
                pending.push(session.update(cx, |s, cx| s.save_draft(key, text, cx)));
            }
        }
        if let Some(RightPanel::Thread(t)) = &ready.right {
            let panel = t.panel.read(cx);
            let (key, text) = (
                thread_key(panel.root()),
                panel.composer().read(cx).composed(cx),
            );
            pending.push(session.update(cx, |s, cx| s.save_draft(key, text, cx)));
        }
        cx.background_executor().spawn(async move {
            futures::future::join_all(pending).await;
        })
    }

    /// `sidebar::NextUnread` from anywhere in the window: opens the next unread conversation.
    fn next_unread(&mut self, cx: &mut Context<Self>) {
        let Some(r) = &self.ready else { return };
        let next = {
            let s = r.session.read(cx);
            components::next_unread(&SidebarSource::rows(s, cx), s.selected())
        };
        if let Some(id) = next {
            r.session.update(cx, |s, cx| s.open(id, cx));
        }
    }

    /// `store::RebuildCache`: only offered (and only acts) while the store is unreadable.
    fn rebuild_cache(&mut self, cx: &mut Context<Self>) {
        if let Some(r) = &self.ready {
            r.session.update(cx, |s, cx| {
                if s.store_health() == StoreHealth::Unreadable {
                    s.rebuild_cache(cx);
                }
            });
        }
    }

    /// The open search panel, if any.
    pub fn search_panel(&self) -> Option<&Entity<SearchPanel<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| match &r.right {
                Some(RightPanel::Search(s)) => Some(s),
                _ => None,
            })
            .map(|s| &s.panel)
    }

    /// The open delete confirmation, if any.
    pub fn delete_confirm(&self) -> Option<&crate::delete_confirm::DeleteConfirm> {
        self.delete_confirm.as_ref()
    }

    /// The timeline of the shown conversation, once there is one.
    pub fn timeline(&self) -> Option<&Entity<Timeline<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| r.chat.as_ref())
            .map(|c| &c.timeline)
    }

    /// The open cross-workspace view, if any.
    pub fn aggregate_view(&self) -> Option<&Entity<AggregateView<Workspaces>>> {
        self.aggregate.as_ref().map(|a| &a.view)
    }

    /// The open cross-workspace view's kind, if any.
    pub fn aggregate_kind(&self) -> Option<AggregateKind> {
        self.aggregate.as_ref().map(|a| a.kind)
    }

    /// Opens a cross-workspace view over the window; asking for the open one
    /// closes it again. Nothing is marked read by looking.
    pub fn open_aggregate(
        &mut self,
        kind: AggregateKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ready.is_none() {
            return;
        }
        if self.aggregate_kind() == Some(kind) {
            self.close_aggregate(window, cx);
            return;
        }
        let w = self.workspaces.clone();
        let view = cx.new(|cx| AggregateView::new(kind, w, cx));
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, _, ev: &AggregateEvent, window, cx| match ev {
                AggregateEvent::Close => this.close_aggregate(window, cx),
                AggregateEvent::Open { workspace, target } => {
                    this.open_aggregate_row(workspace.clone(), target.clone(), window, cx)
                }
            },
        );
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.aggregate = Some(AggregatePane {
            kind,
            view,
            _sub: sub,
        });
        cx.notify();
    }

    fn close_aggregate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.aggregate.take().is_none() {
            return;
        }
        match self.ready.as_ref().and_then(|r| r.chat.as_ref()) {
            Some(chat) => {
                let focus = chat.composer.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// A row of a cross-workspace view opens in its own workspace.
    fn open_aggregate_row(
        &mut self,
        ws: WorkspaceKey,
        target: AggregateTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.aggregate = None;
        match target {
            AggregateTarget::Conversation(c) => self.navigate(ws, c, None, window, cx),
            AggregateTarget::Message(id) => {
                self.navigate(ws, id.conversation.clone(), Some(id), window, cx)
            }
            AggregateTarget::Schedule(_) => {
                self.workspaces.update(cx, |w, cx| w.activate(&ws, cx));
                // The window shows the new workspace once the activation has settled.
                cx.defer_in(window, |this, window, cx| this.open_scheduled(window, cx));
            }
        }
        cx.notify();
    }

    /// The open Scheduled view, if any.
    pub fn scheduled_view(&self) -> Option<&Entity<ScheduledView<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| match &r.right {
                Some(RightPanel::Scheduled(s)) => Some(s),
                _ => None,
            })
            .map(|s| &s.view)
    }

    /// Opens the workspace's Scheduled view in the right column.
    pub fn open_scheduled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if !matches!(ready.right, Some(RightPanel::Scheduled(_))) {
            ready.close_right(cx);
            let view = cx.new(|cx| ScheduledView::new(ready.session.clone(), cx));
            let sub =
                cx.subscribe_in(
                    &view,
                    window,
                    |this, _, ev: &ScheduledEvent, window, cx| match ev {
                        ScheduledEvent::Close => this.close_scheduled(window, cx),
                    },
                );
            ready.right = Some(RightPanel::Scheduled(ScheduledPane { view, _sub: sub }));
        }
        if let Some(RightPanel::Scheduled(p)) = &ready.right {
            let focus = p.view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// The open Drafts view, if any.
    pub fn drafts_view(&self) -> Option<&Entity<components::DraftsView<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| match &r.right {
                Some(RightPanel::Drafts(d)) => Some(d),
                _ => None,
            })
            .map(|d| &d.view)
    }

    /// Opens the workspace's Drafts view in the right column. The text in the
    /// composers on screen is saved first, so the list is current.
    pub fn open_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ready.is_none() {
            return;
        }
        self.save_drafts(cx).detach();
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if !matches!(ready.right, Some(RightPanel::Drafts(_))) {
            ready.close_right(cx);
            let view = cx.new(|cx| components::DraftsView::new(ready.session.clone(), cx));
            let sub = cx.subscribe_in(
                &view,
                window,
                |this, _, ev: &components::DraftsEvent, window, cx| match ev {
                    components::DraftsEvent::Close => this.close_drafts(window, cx),
                    components::DraftsEvent::Open(key) => this.open_draft(key.clone(), window, cx),
                },
            );
            let live = ready.chat.as_ref().map(|chat| {
                cx.observe(&chat.composer, |this, composer, cx| {
                    let Some(ready) = this.ready.as_ref() else {
                        return;
                    };
                    if !matches!(ready.right, Some(RightPanel::Drafts(_))) {
                        return;
                    }
                    let Some(chat) = ready.chat.as_ref() else {
                        return;
                    };
                    let c = composer.read(cx);
                    if c.editing().is_some() {
                        return;
                    }
                    let key = DraftKey {
                        conversation: chat.conversation.clone(),
                        thread: None,
                    };
                    let text = c.composed(cx);
                    ready
                        .session
                        .clone()
                        .update(cx, |s, cx| s.set_live_draft(key, text, cx));
                })
            });
            ready.right = Some(RightPanel::Drafts(DraftsPane {
                view,
                _sub: sub,
                _live: live,
            }));
        }
        if let Some(RightPanel::Drafts(p)) = &ready.right {
            let focus = p.view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// The open People directory, if any.
    pub fn people_directory(
        &self,
    ) -> Option<&Entity<components::PeopleDirectory<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| match &r.right {
                Some(RightPanel::People(p)) => Some(p),
                _ => None,
            })
            .map(|p| &p.view)
    }

    /// Opens the workspace's People directory in the right column, with the
    /// keyboard in its search field.
    pub fn open_people(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if !matches!(ready.right, Some(RightPanel::People(_))) {
            ready.close_right(cx);
            let view = cx.new(|cx| components::PeopleDirectory::new(ready.session.clone(), cx));
            let sub = cx.subscribe_in(
                &view,
                window,
                |this, _, ev: &components::PeopleEvent, window, cx| match ev {
                    components::PeopleEvent::Close => this.close_people(window, cx),
                    components::PeopleEvent::Message { user, name } => {
                        this.message_person(user.clone(), name.to_string(), window, cx)
                    }
                    components::PeopleEvent::ViewProfile(u) => {
                        this.profile_from_people(u.clone(), window, cx)
                    }
                },
            );
            ready.right = Some(RightPanel::People(PeoplePane { view, _sub: sub }));
        }
        if let Some(RightPanel::People(p)) = &ready.right {
            let focus = p.view.read(cx).search_focus(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// Shows `u`'s profile in place of the directory, keeping the directory
    /// (query, filters, scroll) to come back to when the profile closes.
    fn profile_from_people(
        &mut self,
        u: domain::UserId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        let Some(RightPanel::People(pane)) = ready.right.take() else {
            return;
        };
        let session = ready.session.clone();
        session.update(cx, |s, cx| {
            s.clear_directory_watch(cx);
            s.show_profile(u, cx);
        });
        self.open_profile_panel(window, cx);
        if let Some(ready) = self.ready.as_mut() {
            ready.people_back = Some(pane);
        }
    }

    fn close_people(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if matches!(ready.right, Some(RightPanel::People(_))) {
            ready.close_right(cx);
            ready.right = None;
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Opens the direct message with `u` in the shown workspace, or starts
    /// one: the window waits for the provider (`direct_opened`) and says so
    /// when it cannot (`direct_failed`, or no support at all).
    fn message_person(
        &mut self,
        u: domain::UserId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ws) = self.account.clone() else {
            return;
        };
        let Some(session) = self.workspaces.read(cx).session(&ws) else {
            return;
        };
        let state = session.read(cx).state();
        let has_dm = state
            .conversations
            .values()
            .any(|c| c.kind == domain::ConversationKind::Direct && c.peer.as_ref() == Some(&u));
        if !has_dm {
            self.start_direct(ws, &session, u, name, cx);
            return;
        }
        session.update(cx, |s, cx| s.open_direct_now(u, cx));
        cx.defer_in(window, |this, window, cx| this.focus_composer(window, cx));
    }

    /// Asks the provider for a new direct message with `u`; the window waits
    /// for it (`direct_opened`, `direct_failed`), or says the provider cannot.
    fn start_direct(
        &mut self,
        ws: WorkspaceKey,
        session: &Entity<WorkspaceSession>,
        u: domain::UserId,
        name: String,
        cx: &mut Context<Self>,
    ) {
        if !session.read(cx).state().caps.open_direct {
            let l = i18n::gpui::localizer(cx);
            self.notice = Some(t!(l, "shell-dm-unsupported", person = name).into_owned());
            cx.notify();
            return;
        }
        self.notice = None;
        self.pending_dm = Some((ws, u.clone(), name));
        session.update(cx, |s, cx| s.open_direct_now(u, cx));
    }

    fn close_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if matches!(ready.right, Some(RightPanel::Drafts(_))) {
            ready.right = None;
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Opens the conversation or thread of a draft, with the draft in its
    /// composer and the keyboard there.
    fn open_draft(&mut self, key: DraftKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.account.clone() else {
            return;
        };
        if let Some(ready) = self.ready.as_mut()
            && matches!(ready.right, Some(RightPanel::Drafts(_)))
        {
            ready.right = None;
        }
        match key.thread {
            Some(ts) => {
                let root = MessageId {
                    conversation: key.conversation,
                    ts,
                };
                self.navigate_thread(ws, root, None, window, cx);
            }
            None => {
                self.navigate(ws, key.conversation, None, window, cx);
                cx.defer_in(window, |this, window, cx| this.focus_composer(window, cx));
            }
        }
        cx.notify();
    }

    /// A draft deleted from the Drafts view empties every composer showing it
    /// (the main window's and pop-outs'), so leaving one does not save it back.
    /// The Drafts view and a thread panel share the right column, so a thread
    /// draft can only be on screen in a pop-out.
    fn clear_deleted_draft(&mut self, key: &DraftKey, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_ref() else {
            return;
        };
        let mut composers: Vec<_> = ready
            .chat
            .as_ref()
            .filter(|c| key.thread.is_none() && c.conversation == key.conversation)
            .map(|c| c.composer.clone())
            .into_iter()
            .collect();
        if let Some(ws) = self.account.clone() {
            composers.extend(crate::popout::composers_for(&ws, key, cx));
        }
        for c in composers {
            c.update(cx, |c, cx| {
                c.set_composed(&domain::ComposedText::plain(""), cx)
            });
        }
    }

    /// The direct message New message asked for is open: its composer gets
    /// the keyboard once the window shows it.
    fn direct_opened(&mut self, u: &domain::UserId, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_dm.as_ref().is_some_and(|(_, p, _)| p == u) {
            self.pending_dm = None;
            cx.defer_in(window, |this, window, cx| this.focus_composer(window, cx));
        }
    }

    /// The direct message New message asked for could not be opened: say so.
    fn direct_failed(&mut self, u: &domain::UserId, why: &str, cx: &mut Context<Self>) {
        let Some((_, _, name)) = self.pending_dm.take_if(|(_, p, _)| p == u) else {
            return;
        };
        let l = i18n::gpui::localizer(cx);
        let reason = why.to_string();
        self.notice = Some(t!(l, "shell-dm-failed", person = name, reason = reason).into_owned());
        cx.notify();
    }

    /// The window's notice line (an action that failed), if any.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// The open Threads or Later view, if any.
    pub fn collection_open(&self) -> bool {
        self.ready.as_ref().is_some_and(|r| r.collection.is_some())
    }

    /// The open pins popover, if any.
    pub fn pins_popover(&self) -> Option<&Entity<PinsPopover<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| r.pins.as_ref())
            .map(|p| &p.popover)
    }

    /// Opens the Threads view (`threads`) or the Later view in the centre column.
    pub fn open_collection(&mut self, threads: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        // Threads and Later keep an open thread beside them, nothing else.
        if !matches!(ready.right, Some(RightPanel::Thread(_))) {
            ready.close_right(cx);
        }
        ready.pins = None;
        let session = ready.session.clone();
        let (kind, focus) = if threads {
            let view = cx.new(|cx| ThreadsView::new(session, cx));
            let focus = view.read(cx).focus_handle(cx);
            (CollectionKind::Threads(view), focus)
        } else {
            let view = cx.new(|cx| SavedView::new(session, cx));
            let focus = view.read(cx).focus_handle(cx);
            (CollectionKind::Later(view), focus)
        };
        let sub = match &kind {
            CollectionKind::Threads(v) => cx.subscribe_in(v, window, Self::on_collection_event),
            CollectionKind::Later(v) => cx.subscribe_in(v, window, Self::on_collection_event),
        };
        window.focus(&focus, cx);
        ready.collection = Some(CollectionPane { kind, _sub: sub });
        cx.notify();
    }

    fn on_collection_event<V: 'static>(
        &mut self,
        _: &Entity<V>,
        ev: &CollectionEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            CollectionEvent::Close => {
                if let Some(r) = self.ready.as_mut() {
                    r.collection = None;
                }
                self.focus_composer(window, cx);
                cx.notify();
            }
            CollectionEvent::OpenMessage(id) => {
                if let Some(r) = self.ready.as_mut() {
                    r.collection = None;
                }
                self.open_hit(id.clone(), cx);
                self.focus_composer(window, cx);
            }
            CollectionEvent::OpenThread(root) => self.open_thread_panel(root.clone(), window, cx),
        }
    }

    /// What a Later or pins row does: leaves the view and shows the message.
    /// Test helper: the rows themselves emit the same event.
    #[doc(hidden)]
    pub fn open_collection_message(
        &mut self,
        id: MessageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_collection_event(&cx.entity(), &CollectionEvent::OpenMessage(id), window, cx);
    }

    /// The composer when a conversation is open, else the window root: where
    /// the keyboard goes once an overlay (the tour) lets go of focus.
    fn focus_chat_or_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ready.as_ref().is_some_and(|r| r.chat.is_some()) {
            self.focus_composer(window, cx);
        } else {
            window.focus(&self.focus, cx);
        }
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chat) = self.ready.as_ref().and_then(|r| r.chat.as_ref()) {
            let focus = chat.composer.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
    }

    /// Opens the pins popover of `c` under the header; asking again closes it.
    pub fn toggle_pins(&mut self, c: ConversationId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if ready.pins.take().is_some() {
            self.focus_composer(window, cx);
            cx.notify();
            return;
        }
        let session = ready.session.clone();
        let popover = cx.new(|cx| PinsPopover::new(session, c, cx));
        let sub = cx.subscribe_in(
            &popover,
            window,
            |this, _, ev: &CollectionEvent, window, cx| match ev {
                CollectionEvent::Close => {
                    if let Some(r) = this.ready.as_mut() {
                        r.pins = None;
                    }
                    this.focus_composer(window, cx);
                    cx.notify();
                }
                CollectionEvent::OpenMessage(id) => this.open_hit(id.clone(), cx),
                CollectionEvent::OpenThread(root) => {
                    this.open_thread_panel(root.clone(), window, cx)
                }
            },
        );
        let focus = popover.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        ready.pins = Some(PinsPane { popover, _sub: sub });
        cx.notify();
    }

    /// The open Reminders view, if any.
    pub fn reminders_view(&self) -> Option<&Entity<RemindersView<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| match &r.right {
                Some(RightPanel::Reminders(s)) => Some(s),
                _ => None,
            })
            .map(|s| &s.view)
    }

    /// Opens the workspace's Reminders view in the right column.
    pub fn open_reminders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if !matches!(ready.right, Some(RightPanel::Reminders(_))) {
            ready.close_right(cx);
            let view = cx.new(|cx| RemindersView::new(ready.session.clone(), cx));
            let sub =
                cx.subscribe_in(
                    &view,
                    window,
                    |this, _, ev: &RemindersEvent, window, cx| match ev {
                        RemindersEvent::Close => this.close_reminders(window, cx),
                        RemindersEvent::OpenMessage(id) => this.open_hit(id.clone(), cx),
                    },
                );
            ready.right = Some(RightPanel::Reminders(RemindersPane { view, _sub: sub }));
        }
        if let Some(RightPanel::Reminders(p)) = &ready.right {
            let focus = p.view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn close_reminders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if matches!(ready.right, Some(RightPanel::Reminders(_))) {
            ready.right = None;
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn close_scheduled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if matches!(ready.right, Some(RightPanel::Scheduled(_))) {
            ready.right = None;
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Asks once about the overdue local schedules of workspace `key`: send
    /// them now, or keep them scheduled (the next launch asks again).
    fn ask_late(&mut self, key: WorkspaceKey, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.workspaces.read(cx).session(&key) else {
            return;
        };
        let l = i18n::gpui::localizer(cx);
        let lines: Vec<String> = {
            let s = session.read(cx);
            let ids = s.late_prompt().unwrap_or_default();
            s.state()
                .local_schedules
                .iter()
                .filter(|item| ids.contains(&item.id))
                .map(|item| {
                    let label = ScheduledSource::conversation_label(s, &item.conversation, cx);
                    let text: String = item.text.chars().take(60).collect();
                    t!(l, "schedule-late-line", label = label, text = text).into_owned()
                })
                .collect()
        };
        if lines.is_empty() {
            return;
        }
        let title = t!(l, "schedule-late-title", count = lines.len()).into_owned();
        let detail = format!(
            "{}\n\n{}",
            t!(l, "schedule-late-detail", count = lines.len()),
            lines.join("\n")
        );
        let (keep, send) = (
            t!(l, "schedule-late-keep").into_owned(),
            t!(l, "schedule-late-send").into_owned(),
        );
        let answer = window.prompt(
            gpui::PromptLevel::Info,
            &title,
            Some(&detail),
            &[keep.as_str(), send.as_str()],
            cx,
        );
        cx.spawn(async move |_, cx| {
            let send = answer.await == Ok(1);
            session.update(cx, |s, cx| s.answer_late(send, cx));
        })
        .detach();
    }

    /// `workspace::Search`: opens the search panel in the right column (or
    /// focuses it when already open).
    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if !matches!(ready.right, Some(RightPanel::Search(_))) {
            ready.close_right(cx);
            let session = ready.session.clone();
            let shown = ready
                .chat
                .as_ref()
                .and_then(|c| session.read(cx).state().conversations.get(&c.conversation))
                .map(|c| match c.kind {
                    domain::ConversationKind::Channel
                    | domain::ConversationKind::PrivateChannel => format!("#{}", c.name),
                    _ => format!("@{}", c.name),
                });
            let panel = cx.new(|cx| match &shown {
                Some(name) => SearchPanel::in_conversation(session, name, cx),
                None => SearchPanel::new(session, cx),
            });
            let sub = cx.subscribe_in(
                &panel,
                window,
                |this, _, ev: &SearchPanelEvent, window, cx| match ev {
                    SearchPanelEvent::Open(id) => this.open_hit(id.clone(), cx),
                    SearchPanelEvent::OpenConversation(c) => this.open_result(c.clone(), cx),
                    SearchPanelEvent::OpenPerson(u) => this.open_person(u.clone(), cx),
                    SearchPanelEvent::Close => this.close_search(window, cx),
                },
            );
            ready.right = Some(RightPanel::Search(SearchPane { panel, _sub: sub }));
        }
        if let Some(RightPanel::Search(pane)) = &ready.right {
            let focus = pane.panel.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if matches!(ready.right, Some(RightPanel::Search(_))) {
            ready.right = None;
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Shows the hit's conversation with the message brought into view and
    /// highlighted (a jump: held messages scroll, others load around it).
    fn open_hit(&mut self, id: MessageId, cx: &mut Context<Self>) {
        let Some(ready) = &self.ready else { return };
        // Noted for the history entry; the jump itself scrolls and highlights.
        self.pending_reveal = Some(id.clone());
        ready.session.update(cx, |s, cx| s.open_at(id, cx));
        self.apply_pending_reveal(cx);
    }

    fn apply_pending_reveal(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.pending_reveal.clone() else {
            return;
        };
        let Some(chat) = self.ready.as_ref().and_then(|r| r.chat.as_ref()) else {
            return;
        };
        if chat.conversation == id.conversation {
            self.pending_reveal = None;
            chat.timeline.update(cx, |t, cx| t.reveal(id, cx));
        }
    }

    fn open_result(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        let Some(ready) = &self.ready else { return };
        ready
            .session
            .update(cx, |s, cx| s.open_search_result(c, cx));
    }

    /// A person hit shows their profile (its "Message" button opens the DM).
    fn open_person(&mut self, u: domain::UserId, cx: &mut Context<Self>) {
        let Some(ready) = &self.ready else { return };
        ready.session.update(cx, |s, cx| s.show_profile(u, cx));
    }

    /// The open channel details panel, if any.
    pub fn details_panel(&self) -> Option<&Entity<ChannelDetailsPanel<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| match &r.right {
                Some(RightPanel::Details(p)) => Some(p),
                _ => None,
            })
            .map(|p| &p.panel)
    }

    /// Opens the channel details panel in the right column, or closes it.
    pub fn toggle_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = self
            .ready
            .as_ref()
            .is_some_and(|r| matches!(r.right, Some(RightPanel::Details(_))));
        if open {
            if let Some(ready) = self.ready.as_mut() {
                ready.right = None;
            }
            self.focus_composer(window, cx);
            cx.notify();
        } else {
            self.show_details(components::DetailsTab::About, window, cx);
        }
    }

    /// Shows the channel details panel on `tab`: opens it, or only switches
    /// the tab when it is already open (never closes it).
    pub fn show_details(
        &mut self,
        tab: components::DetailsTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if let Some(RightPanel::Details(p)) = &ready.right {
            p.panel.update(cx, |panel, cx| panel.set_tab(tab, cx));
            return;
        }
        ready.close_right(cx);
        let session = ready.session.clone();
        let panel = cx.new(|cx| {
            let mut panel = ChannelDetailsPanel::new(session, cx);
            panel.set_tab(tab, cx);
            panel
        });
        let sub = cx.subscribe_in(
            &panel,
            window,
            |this, _, ev: &ChannelDetailsEvent, window, cx| match ev {
                ChannelDetailsEvent::Edit | ChannelDetailsEvent::Invite => {
                    let open = this
                        .ready
                        .as_ref()
                        .and_then(|r| r.session.read(cx).selected().cloned());
                    if let Some(c) = open {
                        if *ev == ChannelDetailsEvent::Edit {
                            this.edit_channel(c, window, cx);
                        } else {
                            this.invite_people(c, window, cx);
                        }
                    }
                }
                ChannelDetailsEvent::Close => {
                    if let Some(r) = this.ready.as_mut()
                        && matches!(r.right, Some(RightPanel::Details(_)))
                    {
                        r.right = None;
                    }
                    this.focus_composer(window, cx);
                    cx.notify();
                }
            },
        );
        let focus = panel.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        ready.right = Some(RightPanel::Details(DetailsPane { panel, _sub: sub }));
        cx.notify();
    }

    /// The open profile panel, if any.
    pub fn profile_panel(&self) -> Option<&Entity<ProfilePanel<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| match &r.right {
                Some(RightPanel::Profile(p)) => Some(p),
                _ => None,
            })
            .map(|p| &p.panel)
    }

    /// Shows the profile the session has open in the right column.
    pub fn open_profile_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if !matches!(ready.right, Some(RightPanel::Profile(_))) {
            ready.close_right(cx);
            let session = ready.session.clone();
            let panel = cx.new(|cx| ProfilePanel::new(session, cx));
            let sub = cx.subscribe_in(
                &panel,
                window,
                |this, _, ev: &ProfilePanelEvent, window, cx| match ev {
                    ProfilePanelEvent::Close => this.close_profile_panel(window, cx),
                    ProfilePanelEvent::EditStatus => {
                        if let Some(s) = this.ready.as_ref().map(|r| r.session.clone()) {
                            this.open_status_dialog(s, window, cx);
                        }
                    }
                },
            );
            ready.right = Some(RightPanel::Profile(ProfilePane { panel, _sub: sub }));
        }
        if let Some(RightPanel::Profile(p)) = &ready.right {
            let focus = p.panel.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn close_profile_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        if matches!(ready.right, Some(RightPanel::Profile(_))) {
            ready.right = None;
            // Back to the directory this profile was opened from.
            if let Some(pane) = ready.people_back.take() {
                pane.view.update(cx, |v, cx| v.rewatch(cx));
                let focus = pane.view.read(cx).grid_focus();
                ready.right = Some(RightPanel::People(pane));
                window.focus(&focus, cx);
                cx.notify();
                return;
            }
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// The open status dialog, if any.
    pub fn status_dialog(&self) -> Option<&Entity<StatusDialog>> {
        self.status.as_ref().map(|s| &s.dialog)
    }

    /// `workspace::SetStatus`: the status dialog of the shown workspace.
    pub fn set_status_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.ready.as_ref().map(|r| r.session.clone()) else {
            return;
        };
        self.open_status_dialog(session, window, cx);
    }

    /// Opens the status dialog for `session` (when its provider keeps statuses).
    fn open_status_dialog(
        &mut self,
        session: Entity<WorkspaceSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !components::ProfileSource::can_set_status(session.read(cx)) {
            return;
        }
        let current = {
            let s = session.read(cx);
            let st = s.state();
            st.self_user
                .as_ref()
                .and_then(|u| st.members.get(u))
                .and_then(|m| m.status.clone())
        };
        let dialog = cx.new(|cx| StatusDialog::new(current.as_ref(), cx));
        let target = session;
        let sub = cx.subscribe_in(
            &dialog,
            window,
            move |this, _, ev: &StatusDialogEvent, window, cx| {
                if let StatusDialogEvent::Save(st) = ev {
                    let st = st.clone();
                    target.update(cx, |s, cx| components::ProfileSource::set_status(s, st, cx));
                }
                this.close_status_dialog(window, cx);
            },
        );
        let focus = dialog.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.status = Some(StatusPane { dialog, _sub: sub });
        cx.notify();
    }

    fn close_status_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status.take().is_none() {
            return;
        }
        match self.ready.as_ref().and_then(|r| r.chat.as_ref()) {
            Some(chat) => {
                let focus = chat.composer.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// The entries "Set a status..." and "Set yourself as away/active" for
    /// `session`'s workspace menu, as far as its provider supports them.
    fn profile_extras(
        &self,
        session: &Entity<WorkspaceSession>,
        me: gpui::WeakEntity<Self>,
        cx: &App,
    ) -> Vec<components::MenuExtra> {
        use components::ProfileSource as _;
        let l = i18n::gpui::localizer(cx);
        let s = session.read(cx);
        let mut out = Vec::new();
        if s.can_set_status() {
            let session = session.clone();
            out.push(components::MenuExtra {
                danger: false,
                divider_before: false,
                icon: None,
                after: false,
                shortcut: None,
                label: t!(l, "shell-set-status").into_owned().into(),
                action: std::rc::Rc::new(move |window, cx| {
                    let session = session.clone();
                    me.update(cx, |this, cx| this.open_status_dialog(session, window, cx))
                        .ok();
                }),
            });
        }
        if s.can_set_presence() {
            let away = s.own_presence() == Some(domain::PresenceSetting::Away);
            let label = if away {
                t!(l, "shell-set-active")
            } else {
                t!(l, "shell-set-away")
            };
            let session = session.clone();
            out.push(components::MenuExtra {
                danger: false,
                divider_before: false,
                icon: None,
                after: false,
                shortcut: None,
                label: label.into_owned().into(),
                action: std::rc::Rc::new(move |_, cx| {
                    session.update(cx, |s, cx| {
                        let next = components::next_presence(s.own_presence());
                        components::ProfileSource::set_presence(s, next, cx)
                    })
                }),
            });
        }
        out
    }

    /// Opens a URL: jumps in-app for a recognized link to a signed-in workspace,
    /// otherwise opens it in the browser as today.
    pub fn open_link_url(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        let queue_while_starting = self.workspaces.read(cx).is_starting()
            && crate::links::should_queue_while_starting(&url, cx);
        match crate::links::resolve_url(&url, cx) {
            crate::links::ResolvedLink::Jump {
                ws,
                conversation,
                message,
            } => self.navigate(ws, conversation, message, window, cx),
            crate::links::ResolvedLink::OpenThread { ws, root, reply } => {
                self.navigate_thread(ws, root, reply, window, cx)
            }
            crate::links::ResolvedLink::OpenBrowser(u) => {
                if queue_while_starting {
                    self.workspaces.update(cx, |w, cx| w.queue_link(url, cx));
                } else {
                    cx.open_url(&u);
                }
            }
            crate::links::ResolvedLink::Ignored => {
                if queue_while_starting {
                    self.workspaces.update(cx, |w, cx| w.queue_link(url, cx));
                }
            }
        }
    }

    /// Shows conversation `c` of workspace `ws` (making that workspace the
    /// active one) and, for a jump, scrolls to `message`.
    pub fn navigate(
        &mut self,
        ws: WorkspaceKey,
        c: ConversationId,
        message: Option<MessageId>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.workspaces.read(cx).session(&ws) else {
            return;
        };
        if self.workspaces.read(cx).active() != Some(&ws) {
            self.workspaces.update(cx, |w, cx| w.activate(&ws, cx));
        }
        match message {
            Some(id) => {
                self.pending_reveal = Some(id.clone());
                session.update(cx, |s, cx| {
                    s.seed_from_store(id.conversation.clone(), cx);
                    s.open_at(id, cx);
                });
            }
            None => {
                self.pending_reveal = None;
                session.update(cx, |s, cx| s.open(c, cx));
            }
        }
        self.apply_pending_reveal(cx);
        cx.notify();
    }

    /// Shows the thread `root` of workspace `ws` and, for a reply permalink,
    /// jumps to the reply in the thread.
    pub fn navigate_thread(
        &mut self,
        ws: WorkspaceKey,
        root: MessageId,
        reply: Option<MessageId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.workspaces.read(cx).session(&ws) else {
            return;
        };
        if self.workspaces.read(cx).active() != Some(&ws) {
            self.workspaces.update(cx, |w, cx| w.activate(&ws, cx));
        }
        // Open the conversation first so the workspace view is ready.
        self.pending_reveal = None;
        session.update(cx, |s, cx| s.open(root.conversation.clone(), cx));
        // Opening the thread panel reuses the existing open-thread path.
        self.open_thread_panel(root, window, cx);
        if let Some(reply) = reply
            && let Some(panel) = self.thread_panel().cloned()
        {
            panel.update(cx, |p, cx| p.reveal(reply, cx));
        }
        cx.notify();
    }

    /// Notes the conversation now shown in the history (unless the history
    /// itself is moving there).
    fn record_visit(&mut self, cx: &mut Context<Self>) {
        let (Some(ws), Some(conversation)) = (
            self.workspaces.read(cx).active().cloned(),
            self.ready
                .as_ref()
                .and_then(|r| r.session.read(cx).selected().cloned()),
        ) else {
            return;
        };
        if let Some(t) = &self.nav_target {
            // Going back or forward: ignore what is shown on the way there.
            if t.workspace == ws && t.conversation == conversation {
                self.nav_target = None;
            }
            return;
        }
        let message = self
            .pending_reveal
            .clone()
            .filter(|m| m.conversation == conversation);
        self.nav.visit(NavEntry {
            workspace: ws,
            conversation,
            message,
        });
    }

    /// `workspace::GoBack`.
    pub fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(true, window, cx);
    }

    /// `workspace::GoForward`.
    pub fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(false, window, cx);
    }

    fn step_history(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        // An entry whose workspace or conversation is gone is skipped.
        let workspaces = self.workspaces.clone();
        let valid = nav_entry_valid(&workspaces, cx);
        let entry = if back {
            self.nav.back(valid)
        } else {
            self.nav.forward(valid)
        };
        let Some(e) = entry else { return };
        let showing = self.workspaces.read(cx).active() == Some(&e.workspace)
            && self
                .ready
                .as_ref()
                .is_some_and(|r| r.session.read(cx).selected() == Some(&e.conversation));
        self.nav_target = (!showing).then(|| e.clone());
        self.navigate(e.workspace, e.conversation, e.message, window, cx);
    }

    /// `workspace::QuickSwitch`: opens the switcher over the window (closes it when open).
    pub fn open_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_switcher(components::SwitchScope::All, window, cx);
    }

    /// Opens the switcher for `scope` (closes it when open): everything, a
    /// compose target, channels or people.
    pub fn open_switcher(
        &mut self,
        scope: components::SwitchScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.switcher.is_some() {
            self.close_quick_switcher(window, cx);
            return;
        }
        if self.ready.is_none() {
            return;
        }
        let items = self.scoped_switch_items(scope, cx);
        let recent = self.nav.recent();
        let root = cx.entity().downgrade();
        let delegate = SwitchDelegate::new(items, recent)
            .with_scope(scope)
            .with_paste_handler(Rc::new(
                move |text: String, window: &mut Window, cx: &mut App| {
                    let Some(trimmed) = crate::links::lone_pasted_link(&text) else {
                        return components::PasteOutcome::Insert;
                    };
                    let resolved = crate::links::resolve_url(trimmed, cx);
                    match resolved {
                        crate::links::ResolvedLink::Jump { .. }
                        | crate::links::ResolvedLink::OpenThread { .. }
                        | crate::links::ResolvedLink::OpenBrowser(_) => {
                            root.update(cx, |root, cx| {
                                root.open_link_url(trimmed.to_string(), window, cx)
                            })
                            .ok();
                            components::PasteOutcome::Close
                        }
                        crate::links::ResolvedLink::Ignored => components::PasteOutcome::Insert,
                    }
                },
            ));
        let picker = cx.new(|cx| components::QuickSwitcher::new(delegate, cx));
        let sub = cx.subscribe_in(
            &picker,
            window,
            |this, picker, ev: &PickerEvent, window, cx| {
                let chosen = match ev {
                    PickerEvent::Confirmed(_) => picker.read(cx).delegate().chosen().cloned(),
                    PickerEvent::Dismissed => None,
                };
                let scope = picker.read(cx).delegate().scope();
                // A channel the browse found is held before it opens.
                let found = chosen.as_ref().and_then(|item| match &item.target {
                    SwitchTarget::Conversation(c) if !item.joined => this
                        .switcher
                        .as_ref()
                        .and_then(|p| p.found.iter().find(|f| &f.id == c).cloned())
                        .map(|f| (item.workspace.clone(), f)),
                    _ => None,
                });
                this.close_quick_switcher(window, cx);
                if let Some((ws, f)) = found
                    && let Some(session) = this.workspaces.read(cx).session(&ws)
                {
                    session.update(cx, |s, cx| s.preview(f, cx));
                }
                if let Some(item) = chosen {
                    this.pick_scoped(item, scope, window, cx);
                }
            },
        );
        let focus = picker.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        // Typed text also asks the provider for channels the user is not in.
        let browse = scope != components::SwitchScope::People
            && self
                .ready
                .as_ref()
                .is_some_and(|r| r.session.read(cx).can_admin());
        let observe = browse.then(|| cx.observe(&picker, Self::switcher_changed));
        self.switcher = Some(SwitcherPane {
            picker,
            found: Vec::new(),
            query: String::new(),
            pending: None,
            loading: None,
            _sub: sub,
            _observe: observe,
        });
        cx.notify();
    }

    /// The switcher's text changed: once the typing settles, browse the
    /// provider's channels for it.
    fn switcher_changed(
        &mut self,
        picker: Entity<components::QuickSwitcher>,
        cx: &mut Context<Self>,
    ) {
        let q = picker.read(cx).query().trim().to_string();
        let Some(p) = self.switcher.as_mut() else {
            return;
        };
        if p.query == q {
            return;
        }
        p.query = q.clone();
        p.pending = None;
        if let Some(a) = p.loading.take() {
            a.abort();
        }
        if q.is_empty() {
            p.found.clear();
            picker.update(cx, |p, cx| {
                p.update_delegate(cx, |d| d.set_found(Vec::new()))
            });
            return;
        }
        p.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(components::BROWSE_DEBOUNCE)
                .await;
            this.update(cx, |this, cx| this.browse_switcher(q, cx)).ok();
        }));
    }

    fn browse_switcher(&mut self, q: String, cx: &mut Context<Self>) {
        let Some(session) = self.ready.as_ref().map(|r| r.session.clone()) else {
            return;
        };
        let Some(p) = self.switcher.as_mut() else {
            return;
        };
        p.pending = None;
        let root = cx.entity().downgrade();
        let asked = q.clone();
        let abort = session.update(cx, |s, cx| {
            s.browse_channels(q, None, cx, move |s, r, cx| {
                let Ok(page) = r else { return };
                // Public channels the user is not in; the rest are listed.
                let found: Vec<domain::Conversation> = page
                    .items
                    .into_iter()
                    .filter(|c| {
                        c.kind == domain::ConversationKind::Channel
                            && !c.is_archived
                            && !s
                                .state()
                                .conversations
                                .get(&c.id)
                                .is_some_and(|k| k.is_member)
                    })
                    .collect();
                let ws = s.state().workspace.clone();
                root.update(cx, |this, cx| this.switcher_found(ws, asked, found, cx))
                    .ok();
            })
        });
        p.loading = Some(abort);
    }

    fn switcher_found(
        &mut self,
        ws: WorkspaceKey,
        asked: String,
        found: Vec<domain::Conversation>,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.switcher.as_mut() else {
            return;
        };
        if p.query != asked {
            return;
        }
        p.loading = None;
        // Named like the local rows: the workspace only when several are signed in.
        let w = self.workspaces.read(cx);
        let name: gpui::SharedString = w
            .accounts()
            .iter()
            .find(|a| a.key == ws && w.accounts().len() > 1)
            .map(|a| a.name.to_string().into())
            .unwrap_or_default();
        let Some(p) = self.switcher.as_mut() else {
            return;
        };
        let items = found
            .iter()
            .map(|c| SwitchItem {
                workspace: ws.clone(),
                target: SwitchTarget::Conversation(c.id.clone()),
                label: c.name.to_string().into(),
                workspace_name: name.clone(),
                unread: false,
                mentions: 0,
                joined: false,
            })
            .collect();
        p.found = found;
        let picker = p.picker.clone();
        picker.update(cx, |p, cx| p.update_delegate(cx, |d| d.set_found(items)));
    }

    fn close_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.switcher.take() else {
            return;
        };
        // The browse on its way has no switcher to answer any more.
        if let Some(a) = p.loading {
            a.abort();
        }
        match self.ready.as_ref().and_then(|r| r.chat.as_ref()) {
            Some(chat) => {
                let focus = chat.composer.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// Actions that only work in a focused pane (text editing, pickers, composer
    /// formatting) are not listed in the palette. Kept explicit so the a11y test
    /// can enforce the same rule.
    const PALETTE_EXCLUDED: &'static [&'static str] = &[
        // Composer formatting keys: only meaningful in the composer.
        "composer::Bold",
        "composer::Italic",
        "composer::Strike",
        "composer::Code",
        "composer::CodeBlock",
        "composer::Quote",
        "composer::BulletList",
        "composer::NumberedList",
        "composer::Link",
    ];

    fn is_excluded_palette_action(name: &str) -> bool {
        // Framework/internal actions not owned by Parla.
        if name.starts_with("zed::") || name.starts_with("editor::") || name.starts_with("buffer::")
        {
            return true;
        }
        // Text input and picker actions are internal to their widgets.
        if name.starts_with("text_input::") || name.starts_with("picker::") {
            return true;
        }
        // GPUI Base primitives' own actions (registered by `gpui_base::init`).
        if name.starts_with("grid_layout::")
            || name.starts_with("input::")
            || name.starts_with("number_input::")
            || name.starts_with("root::")
            || name.starts_with("ui::")
        {
            return true;
        }
        crate::actions::unsupported_here(name, cfg!(target_os = "macos"))
            || Self::PALETTE_EXCLUDED.contains(&name)
    }

    /// Builds the command palette's items from the actions Parla knows about.
    pub fn command_palette_items(window: &Window, cx: &mut App) -> Vec<CommandItem> {
        let l = i18n::gpui::localizer(cx);
        let os = cx.platform_info().os;
        let mut items: Vec<CommandItem> = cx
            .all_action_names()
            .iter()
            .filter(|name| !Self::is_excluded_palette_action(name))
            .filter_map(|name| {
                let title = match crate::actions::title(&l, name) {
                    Some(t) => t,
                    None => {
                        // An action without a title must not be dropped silently.
                        crate::log::warn(&format!("palette action has no localized title: {name}"));
                        return None;
                    }
                };
                let binding = cx
                    .build_action(name, None)
                    .ok()
                    .and_then(|action| {
                        window
                            .bindings_for_action(&*action)
                            .into_iter()
                            .next()
                            .map(|b| {
                                b.keystrokes()
                                    .iter()
                                    .map(|k| k.unparse(os))
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                    })
                    .map(gpui::SharedString::from);
                Some(CommandItem {
                    name: name.to_string().into(),
                    title: title.into(),
                    binding,
                })
            })
            .collect();
        items.sort_by(|a, b| a.title.cmp(&b.title).then(a.name.cmp(&b.name)));
        items
    }

    /// `app::CommandPalette`: opens the palette (closes it when open).
    pub fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.command_palette.is_some() {
            self.close_command_palette(window, cx);
            return;
        }
        let previous_focus = window.focused(cx);
        let items = Self::command_palette_items(window, cx);
        let picker = cx.new(|cx| CommandPalette::new(CommandDelegate::new(items), cx));
        let sub = cx.subscribe_in(
            &picker,
            window,
            |this, picker, ev: &PickerEvent, window, cx| {
                let chosen = match ev {
                    PickerEvent::Confirmed(_) => picker
                        .read(cx)
                        .delegate()
                        .chosen()
                        .map(|c| c.name.to_string()),
                    PickerEvent::Dismissed => None,
                };
                let prev = this.close_command_palette(window, cx);
                if let Some(name) = chosen {
                    if let Some(h) = prev {
                        window.focus(&h, cx);
                    }
                    if let Ok(action) = cx.build_action(&name, None) {
                        window.dispatch_action(action, cx);
                    }
                }
            },
        );
        let focus = picker.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.command_palette = Some(CommandPalettePane {
            picker,
            previous_focus,
            _sub: sub,
        });
        cx.notify();
    }

    /// `app::OpenKeyboardShortcuts`: opens the shortcuts sheet over the
    /// window (closes it when open), built from the live keymap.
    pub fn open_shortcuts_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use components::shortcuts_sheet::{ShortcutsSheet, ShortcutsSheetEvent};
        if self.shortcuts_sheet.is_some() {
            self.close_shortcuts_sheet(window, cx);
            return;
        }
        let previous_focus = window.focused(cx);
        let mac = cfg!(target_os = "macos");
        let rows = crate::shortcuts::current(cx);
        let l = i18n::gpui::localizer(cx);
        let groups = crate::shortcuts::sheet_groups(&l, &rows, mac);
        let open = rows
            .iter()
            .find(|r| r.action == "app::OpenKeyboardShortcuts")
            .and_then(|r| r.keystrokes.clone())
            .unwrap_or_default();
        let sheet = cx.new(|cx| ShortcutsSheet::new(groups, &open, mac, cx));
        let sub = cx.subscribe_in(
            &sheet,
            window,
            |this, _, ev: &ShortcutsSheetEvent, window, cx| {
                this.close_shortcuts_sheet(window, cx);
                if *ev == ShortcutsSheetEvent::EditKeymap {
                    crate::app_state::open_settings_page(cx, crate::settings_window::Page::Keymap);
                }
            },
        );
        let focus = sheet.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.shortcuts_sheet = Some(ShortcutsSheetPane {
            sheet,
            previous_focus,
            _sub: sub,
        });
        cx.notify();
    }

    fn close_shortcuts_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.shortcuts_sheet.take() else {
            return;
        };
        match pane.previous_focus {
            Some(h) => window.focus(&h, cx),
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// Closes the command palette and returns the focus that was active before it opened.
    fn close_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::FocusHandle> {
        let pane = self.command_palette.take()?;
        // A dismiss (no dispatch) restores focus like the quick switcher.
        if window.focused(cx).as_ref() == Some(&pane.picker.read(cx).focus_handle(cx)) {
            match pane.previous_focus.clone() {
                Some(h) => window.focus(&h, cx),
                None => match self.ready.as_ref().and_then(|r| r.chat.as_ref()) {
                    Some(chat) => {
                        let focus = chat.composer.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                    }
                    None => window.focus(&self.focus, cx),
                },
            }
        }
        cx.notify();
        pane.previous_focus
    }

    /// Opens the notification menu at `at` for `kind` of `session`; one menu at a time.
    fn open_menu(
        &mut self,
        session: Entity<WorkspaceSession>,
        kind: components::MenuKind,
        extra: Vec<components::MenuExtra>,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = cx.new(|cx| components::NotifyMenu::new(session, kind, extra, window, cx));
        let sub = cx.subscribe_in(
            &menu,
            window,
            |this, _, _: &gpui::DismissEvent, window, cx| this.close_menu(window, cx),
        );
        self.menu = Some(MenuPane {
            menu,
            at,
            _sub: sub,
        });
        cx.notify();
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_none() {
            return;
        }
        // An entry that opened an admin dialog keeps the focus in it.
        if self.admin.is_some() {
            cx.notify();
            return;
        }
        match self.ready.as_ref().and_then(|r| r.chat.as_ref()) {
            Some(chat) => {
                let focus = chat.composer.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// Right click on a rail badge: pause notifications and remove the workspace.
    fn open_workspace_menu(
        &mut self,
        key: WorkspaceKey,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.workspaces.read(cx).session(&key) else {
            // Not running (signed out): only removing it is possible.
            self.remove_workspace_key(key, window, cx);
            return;
        };
        let me = cx.entity().downgrade();
        let l = i18n::gpui::localizer(cx);
        let mut extras = self.profile_extras(&session, me.clone(), cx);
        let remove = components::MenuExtra {
            danger: false,
            divider_before: false,
            icon: None,
            after: false,
            shortcut: None,
            label: t!(l, "shell-remove-confirm").into_owned().into(),
            action: std::rc::Rc::new(move |window, cx| {
                let key = key.clone();
                me.update(cx, |this, cx| this.remove_workspace_key(key, window, cx))
                    .ok();
            }),
        };
        self.open_menu(
            session,
            components::MenuKind::Workspace,
            {
                extras.push(remove);
                extras
            },
            at,
            window,
            cx,
        );
    }

    /// The sidebar's workspace name: the workspace's own menu (status,
    /// presence, pause, remove) plus switching, settings and sign out.
    fn open_sidebar_workspace_menu(
        &mut self,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.workspaces.read(cx).active().cloned() else {
            return;
        };
        let Some(session) = self.workspaces.read(cx).session(&key) else {
            return;
        };
        let me = cx.entity().downgrade();
        let l = i18n::gpui::localizer(cx);
        let mut extras = self.profile_extras(&session, me.clone(), cx);
        // The other workspaces, to switch to.
        for a in self
            .workspaces
            .read(cx)
            .accounts()
            .iter()
            .filter(|a| a.key != key)
        {
            let (w, other) = (self.workspaces.clone(), a.key.clone());
            extras.push(components::MenuExtra::new(
                t!(l, "shell-switch-to", workspace = a.name.to_string()).into_owned(),
                std::rc::Rc::new(move |_, cx| w.update(cx, |w, cx| w.activate(&other, cx))),
            ));
        }
        // The sidebar's own popover (show Unreads, sort, density, organise).
        if let Some(sidebar) = self.ready.as_ref().map(|r| r.sidebar.clone()) {
            let organise = sidebar.clone();
            extras.push(
                components::MenuExtra::new(
                    t!(l, "menu-sidebar-settings").into_owned(),
                    std::rc::Rc::new(move |window, cx| {
                        sidebar.update(cx, |v, cx| v.open_settings(at, window, cx))
                    }),
                )
                .icon(components::IconName::Sidebar),
            );
            extras.push(
                components::MenuExtra::new(
                    t!(l, "menu-organise-sidebar").into_owned(),
                    std::rc::Rc::new(move |window, cx| {
                        organise.update(cx, |v, cx| v.set_organising(true, window, cx))
                    }),
                )
                .icon(components::IconName::Layers),
            );
        }
        extras.push(
            components::MenuExtra::new(
                t!(l, "menu-settings").into_owned(),
                std::rc::Rc::new(|_, cx| crate::app_state::open_settings(cx)),
            )
            .icon(components::IconName::Sliders),
        );
        let out = me.clone();
        extras.push(
            components::MenuExtra::new(
                t!(l, "action-auth-sign-out").into_owned(),
                std::rc::Rc::new(move |window, cx| {
                    out.update(cx, |this, cx| this.sign_out(window, cx)).ok();
                }),
            )
            .icon(components::IconName::Logout),
        );
        self.open_menu(
            session,
            components::MenuKind::Workspace,
            extras,
            at,
            window,
            cx,
        );
    }

    fn pop_out_conversation(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ws), Some(c)) = (self.account.clone(), self.shown_conversation().cloned()) else {
            return;
        };
        crate::popout::open_conversation(ws, c, cx);
    }

    fn pop_out_thread(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let (Some(ws), Some(root)) = (
            self.account.clone(),
            self.thread_panel().map(|p| p.read(cx).root().clone()),
        ) else {
            return;
        };
        crate::popout::open_thread(ws, root.conversation, root.ts, cx);
    }

    /// The notification controls (level and mute) of a conversation.
    fn open_notify_controls(
        &mut self,
        c: ConversationId,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.ready.as_ref().map(|r| r.session.clone()) else {
            return;
        };
        // "All notification settings...": calls the open function directly (a
        // dispatched action from a click cannot reach the window).
        let l = i18n::gpui::localizer(cx);
        let extra = vec![
            components::MenuExtra::new(
                t!(l, "shell-notify-settings").into_owned(),
                std::rc::Rc::new(|_, cx| {
                    crate::app_state::open_settings_page(
                        cx,
                        crate::settings_window::Page::Notifications,
                    );
                }),
            )
            .icon(components::IconName::Sliders),
        ];
        self.open_menu(
            session,
            components::MenuKind::Conversation(c),
            extra,
            at,
            window,
            cx,
        );
    }

    /// The header's More menu: the conversation's actions (details, pop out).
    fn open_conversation_actions(
        &mut self,
        c: ConversationId,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((session, ws)) = self
            .ready
            .as_ref()
            .map(|r| r.session.clone())
            .zip(self.account.clone())
        else {
            return;
        };
        let l = i18n::gpui::localizer(cx);
        let root = cx.entity().downgrade();
        let details_open = self
            .ready
            .as_ref()
            .is_some_and(|r| matches!(r.right, Some(RightPanel::Details(_))));
        let details_label = if details_open {
            t!(l, "shell-hide-details")
        } else {
            t!(l, "shell-show-details")
        }
        .into_owned()
        .into();
        let c_for_open = c.clone();
        let extra = vec![
            components::MenuExtra {
                danger: false,
                divider_before: false,
                icon: None,
                after: false,
                shortcut: None,
                label: details_label,
                action: std::rc::Rc::new(move |window, cx| {
                    root.update(cx, |this, cx| this.toggle_details(window, cx))
                        .ok();
                }),
            },
            components::MenuExtra {
                danger: false,
                divider_before: false,
                icon: None,
                after: false,
                shortcut: None,
                label: t!(l, "shell-pop-out").into_owned().into(),
                action: std::rc::Rc::new(move |_, cx| {
                    crate::popout::open_conversation(ws.clone(), c_for_open.clone(), cx);
                }),
            },
        ];
        self.open_menu(
            session,
            components::MenuKind::Actions(c),
            extra,
            at,
            window,
            cx,
        );
    }

    /// The menu of a conversation row of the shown workspace (right click in
    /// the sidebar): details, star, notifications and mute, mark read, pop out,
    /// copy link (when the provider links conversations), and for a channel
    /// with admin: edit, add people, leave and archive.
    fn open_conversation_menu(
        &mut self,
        c: ConversationId,
        at: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((session, ws)) = self
            .ready
            .as_ref()
            .map(|r| r.session.clone())
            .zip(self.account.clone())
        else {
            return;
        };
        let l = i18n::gpui::localizer(cx);
        let root = cx.entity().downgrade();
        let os = cx.platform_info().os;
        let starred = session.read(cx).is_starred(&c);
        let (c_details, s_details) = (c.clone(), session.clone());
        let (c_star, s_star) = (c.clone(), session.clone());
        let (c_read, s_read) = (c.clone(), session.clone());
        let c_open = c.clone();
        // Only a binding the keymap really has is shown.
        let pop_out_key = window
            .bindings_for_action(&PopOutConversation)
            .into_iter()
            .next()
            .map(|b| {
                let keys: Vec<String> = b.keystrokes().iter().map(|k| k.unparse(os)).collect();
                gpui::SharedString::from(keys.join(" "))
            });
        let extra = vec![
            components::MenuExtra::new(
                t!(l, "menu-view-details").into_owned(),
                std::rc::Rc::new(move |window, cx| {
                    s_details.update(cx, |s, cx| s.open(c_details.clone(), cx));
                    root.update(cx, |this, cx| {
                        this.show_details(components::DetailsTab::About, window, cx)
                    })
                    .ok();
                }),
            )
            .icon(components::IconName::Info),
            components::MenuExtra::new(
                if starred {
                    t!(l, "menu-unstar")
                } else {
                    t!(l, "menu-star")
                }
                .into_owned(),
                std::rc::Rc::new(move |_, cx| {
                    s_star.update(cx, |s, cx| s.set_starred(c_star.clone(), !starred, cx));
                }),
            )
            .icon(components::IconName::Star),
            components::MenuExtra::new(
                t!(l, "menu-mark-read").into_owned(),
                std::rc::Rc::new(move |_, cx| {
                    s_read.update(cx, |s, cx| s.mark_conversation_read(&c_read, cx));
                }),
            )
            .icon(components::IconName::Check)
            .after_notifications(),
            components::MenuExtra::new(
                t!(l, "shell-pop-out").into_owned(),
                std::rc::Rc::new(move |_, cx| {
                    crate::popout::open_conversation(ws.clone(), c_open.clone(), cx);
                }),
            )
            .icon(components::IconName::External)
            .shortcut(pop_out_key)
            .after_notifications(),
        ];
        let mut extra = extra;
        // Copy link sits with Mark as read, before Open in new window.
        if let Some(url) = session.read(cx).conversation_web_url(&c) {
            extra.insert(
                3,
                components::MenuExtra::new(
                    t!(l, "menu-copy-link").into_owned(),
                    std::rc::Rc::new(move |_, cx| {
                        components::Toasts::copy_link(url.clone(), cx);
                    }),
                )
                .icon(components::IconName::Copy)
                .after_notifications(),
            );
        }
        // "Move to <section>": the user's own sections, and back to its own
        // section while it sits in one (local; nothing goes to the provider).
        let current = session.read(cx).section_of(&c, cx);
        let mut moves: Vec<(Option<u32>, String)> = session
            .read(cx)
            .custom_sections(cx)
            .into_iter()
            .filter(|(id, _)| Some(*id) != current)
            .map(|(id, name)| (Some(id), t!(l, "menu-move-to", section = name).into_owned()))
            .collect();
        if current.is_some() {
            let direct = session
                .read(cx)
                .state()
                .conversations
                .get(&c)
                .is_some_and(|c| {
                    matches!(
                        c.kind,
                        domain::ConversationKind::Direct | domain::ConversationKind::Group
                    )
                });
            let home = if direct {
                t!(l, "menu-move-to-direct-messages")
            } else {
                t!(l, "menu-move-to-channels")
            };
            moves.push((None, home.into_owned()));
        }
        for (ix, (to, label)) in moves.into_iter().enumerate() {
            let (c_move, s_move) = (c.clone(), session.clone());
            let mut item = components::MenuExtra::new(
                label,
                std::rc::Rc::new(move |_, cx| {
                    let op = components::SectionOp::MoveConversation {
                        conversation: c_move.0.to_string(),
                        to,
                    };
                    s_move.update(cx, |s, cx| s.section_op_now(op, cx));
                }),
            )
            .icon(components::IconName::Layers)
            .after_notifications();
            if ix == 0 {
                item = item.divider_before();
            }
            extra.push(item);
        }
        if self.admin_menu_for(&c, cx) {
            let admin = |label: String, icon, f: fn(&mut Self, ConversationId, &mut Window, &mut Context<Self>)| {
                let (root, c) = (cx.entity().downgrade(), c.clone());
                components::MenuExtra::new(
                    label,
                    std::rc::Rc::new(move |window, cx| {
                        root.update(cx, |this, cx| f(this, c.clone(), window, cx)).ok();
                    }),
                )
                .icon(icon)
                .after_notifications()
            };
            extra.extend([
                admin(
                    t!(l, "menu-edit-channel").into_owned(),
                    components::IconName::Pencil,
                    |this, c, w, cx| this.edit_channel(c, w, cx),
                ),
                admin(
                    t!(l, "menu-add-people").into_owned(),
                    components::IconName::Users,
                    |this, c, w, cx| this.invite_people(c, w, cx),
                ),
                admin(
                    t!(l, "menu-leave-channel").into_owned(),
                    components::IconName::Logout,
                    |this, c, w, cx| this.confirm_admin(c, false, w, cx),
                )
                .danger()
                .divider_before(),
                admin(
                    t!(l, "menu-archive-channel").into_owned(),
                    components::IconName::Archive,
                    |this, c, w, cx| this.confirm_admin(c, true, w, cx),
                )
                .danger(),
            ]);
        }
        self.open_menu(session, components::MenuKind::Row(c), extra, at, window, cx);
    }

    /// `workspace::PauseNotifications`: the pause menu of the shown workspace.
    pub fn pause_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.ready.as_ref().map(|r| r.session.clone()) else {
            return;
        };
        // Under the titlebar's bell, at the right edge.
        let at = gpui::point(
            window.viewport_size().width
                - themes::metrics::LAYOUT_POPOVER_SM
                - themes::metrics::SPACE_5,
            themes::metrics::LAYOUT_TITLEBAR,
        );
        let extras = self.profile_extras(&session, cx.entity().downgrade(), cx);
        self.open_menu(
            session,
            components::MenuKind::Workspace,
            extras,
            at,
            window,
            cx,
        );
    }

    /// The open notification menu, if any.
    pub fn notify_menu(&self) -> Option<&Entity<components::NotifyMenu<WorkspaceSession>>> {
        self.menu.as_ref().map(|m| &m.menu)
    }

    /// The open quick switcher, if any.
    pub fn quick_switcher(&self) -> Option<&Entity<components::QuickSwitcher>> {
        self.switcher.as_ref().map(|s| &s.picker)
    }

    /// The open keyboard shortcuts sheet, if any.
    pub fn shortcuts_sheet(&self) -> Option<&Entity<components::shortcuts_sheet::ShortcutsSheet>> {
        self.shortcuts_sheet.as_ref().map(|s| &s.sheet)
    }

    /// The open command palette, if any.
    pub fn command_palette(&self) -> Option<&Entity<CommandPalette>> {
        self.command_palette.as_ref().map(|s| &s.picker)
    }

    /// A choice in the switcher opens in the workspace it came from.
    fn pick(&mut self, item: SwitchItem, window: &mut Window, cx: &mut Context<Self>) {
        match item.target {
            SwitchTarget::Conversation(c) => self.navigate(item.workspace, c, None, window, cx),
            SwitchTarget::Person(u) => {
                // The DM, when there is one, else the profile.
                let Some(session) = self.workspaces.read(cx).session(&item.workspace) else {
                    return;
                };
                let dm = session
                    .read(cx)
                    .state()
                    .conversations
                    .values()
                    .find_map(|c| {
                        (c.kind == domain::ConversationKind::Direct && c.peer.as_ref() == Some(&u))
                            .then(|| c.id.clone())
                    });
                if let Some(dm) = dm {
                    self.navigate(item.workspace, dm, None, window, cx);
                    return;
                }
                // Start the direct message when the provider can; it opens
                // once the provider answers (`direct_opened`).
                if session.read(cx).state().caps.open_direct {
                    let name = item.label.to_string();
                    self.start_direct(item.workspace.clone(), &session, u, name, cx);
                    return;
                }
                if self.workspaces.read(cx).active() != Some(&item.workspace) {
                    self.workspaces
                        .update(cx, |w, cx| w.activate(&item.workspace, cx));
                }
                // The window shows the new workspace once the activation has settled.
                cx.defer_in(window, move |this, window, cx| {
                    let Some(ready) = this.ready.as_ref() else {
                        return;
                    };
                    ready.session.update(cx, |s, cx| s.show_profile(u, cx));
                    this.open_profile_panel(window, cx);
                });
            }
        }
    }

    /// A switcher pick. From New message or the "+" buttons a person opens
    /// (or starts) the direct message, and the composer gets the keyboard.
    fn pick_scoped(
        &mut self,
        item: SwitchItem,
        scope: components::SwitchScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use components::SwitchScope;
        if scope == SwitchScope::All {
            self.pick(item, window, cx);
            return;
        }
        match item.target.clone() {
            SwitchTarget::Person(u) => {
                let Some(session) = self.workspaces.read(cx).session(&item.workspace) else {
                    return;
                };
                let state = session.read(cx).state();
                let has_dm = state.conversations.values().any(|c| {
                    c.kind == domain::ConversationKind::Direct && c.peer.as_ref() == Some(&u)
                });
                if !has_dm {
                    // Wait for the provider: the new DM opens, then its
                    // composer gets the keyboard (`direct_opened`).
                    let name = item.label.to_string();
                    self.start_direct(item.workspace.clone(), &session, u, name, cx);
                    return;
                }
                self.pick(item, window, cx);
            }
            SwitchTarget::Conversation(_) => self.pick(item, window, cx),
        }
        // The composer of what opens gets the keyboard once it is on screen.
        cx.defer_in(window, |this, window, cx| this.focus_composer(window, cx));
    }

    /// The switcher's items for `scope`: everything, or the active workspace's
    /// channels (Channels), people and direct messages (People), or both
    /// (Compose).
    fn scoped_switch_items(&self, scope: components::SwitchScope, cx: &App) -> Vec<SwitchItem> {
        use components::SwitchScope;
        let items = self.switch_items(cx);
        if scope == SwitchScope::All {
            return items;
        }
        let w = self.workspaces.read(cx);
        let Some(active) = w.active().cloned() else {
            return Vec::new();
        };
        let Some(session) = w.session(&active) else {
            return Vec::new();
        };
        let state = session.read(cx).state();
        items
            .into_iter()
            .filter(|i| i.workspace == active)
            .filter(|i| {
                let channel = match &i.target {
                    SwitchTarget::Conversation(c) => state.conversations.get(c).is_some_and(|c| {
                        matches!(
                            c.kind,
                            domain::ConversationKind::Channel
                                | domain::ConversationKind::PrivateChannel
                        )
                    }),
                    SwitchTarget::Person(_) => false,
                };
                match scope {
                    SwitchScope::Channels => channel,
                    SwitchScope::People => !channel,
                    SwitchScope::All | SwitchScope::Compose => true,
                }
            })
            .collect()
    }

    /// Every conversation of every started workspace, the active one first.
    fn switch_items(&self, cx: &App) -> Vec<SwitchItem> {
        let l = i18n::gpui::localizer(cx);
        let w = self.workspaces.read(cx);
        let active = w.active().cloned();
        let several = w.accounts().len() > 1;
        let mut accounts: Vec<_> = w.accounts().to_vec();
        accounts.sort_by_key(|a| Some(&a.key) != active.as_ref());
        let mut items = Vec::new();
        for a in accounts {
            let Some(session) = w.session(&a.key) else {
                continue;
            };
            let session = session.read(cx);
            let state = session.state();
            let index = components::MemberIndex::build(state);
            let name: gpui::SharedString = if several {
                a.name.to_string().into()
            } else {
                "".into()
            };
            // Public channels the user is not in are listed too, to open and
            // join, when the provider can join them.
            let admin = session.can_admin();
            for c in state.conversations.values().filter(|c| {
                !c.is_archived
                    && (c.is_member || (admin && c.kind == domain::ConversationKind::Channel))
            }) {
                items.push(SwitchItem {
                    workspace: a.key.clone(),
                    target: SwitchTarget::Conversation(c.id.clone()),
                    label: components::sidebar::conversation_label(state, &index, c, &l)
                        .to_string()
                        .into(),
                    workspace_name: name.clone(),
                    unread: c.unread && !c.is_muted,
                    mentions: c.mentions,
                    joined: c.is_member,
                });
            }
            // People without a direct message: picking one starts it (or
            // opens the profile when the provider cannot).
            let with_dm: std::collections::HashSet<&domain::UserId> = state
                .conversations
                .values()
                .filter(|c| c.kind == domain::ConversationKind::Direct)
                .filter_map(|c| c.peer.as_ref())
                .collect();
            for m in state.members.values().filter(|m| {
                !m.deleted && Some(&m.id) != state.self_user.as_ref() && !with_dm.contains(&m.id)
            }) {
                let Some(label) = components::sidebar::best_name(m) else {
                    continue;
                };
                items.push(SwitchItem {
                    workspace: a.key.clone(),
                    target: SwitchTarget::Person(m.id.clone()),
                    label: label.to_string().into(),
                    workspace_name: name.clone(),
                    unread: false,
                    mentions: 0,
                    joined: true,
                });
            }
        }
        items
    }

    /// The open thread panel, if any.
    pub fn thread_panel(&self) -> Option<&Entity<ThreadPanel<WorkspaceSession>>> {
        self.ready
            .as_ref()
            .and_then(|r| match &r.right {
                Some(RightPanel::Thread(t)) => Some(t),
                _ => None,
            })
            .map(|t| &t.panel)
    }

    /// Shows the thread under `root` in the right panel, replacing any open one.
    pub fn open_thread_panel(
        &mut self,
        root: MessageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ready) = self.ready.as_mut() else {
            return;
        };
        ready.close_right(cx);
        let clock = cx.global::<SettingsStore>().current().messages.clock;
        let session = ready.session.clone();
        let session_for_draft = session.clone();
        let panel = cx.new(|cx| ThreadPanel::new(session, root.clone(), clock, cx));
        let sub = cx.subscribe(&panel, |this, panel, ev: &ThreadPanelEvent, cx| match ev {
            ThreadPanelEvent::Close(root) => {
                if let Some(r) = this.ready.as_mut() {
                    let text = panel.read(cx).composer().read(cx).composed(cx);
                    let key = thread_key(root);
                    r.session.update(cx, |s, cx| {
                        s.set_draft(key, text, cx);
                        s.close_thread(root.clone(), cx);
                    });
                    if matches!(r.right, Some(RightPanel::Thread(_))) {
                        r.right = None;
                    }
                }
                cx.notify();
            }
            ThreadPanelEvent::PopOut(root) => {
                if let Some(ws) = this.account.clone() {
                    crate::popout::open_thread(ws, root.conversation.clone(), root.ts.clone(), cx);
                }
            }
        });
        let thread_composer = panel.read(cx).composer().clone();
        restore_draft_for(&session_for_draft, &thread_composer, &thread_key(&root), cx);
        let focus = panel.read(cx).composer().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        ready.right = Some(RightPanel::Thread(ThreadPane { panel, _sub: sub }));
        cx.notify();
    }

    fn center(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = *cx.theme().colors();
        let message = |text: &str| {
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(colors.text_muted)
                .child(text.to_string())
                .into_any_element()
        };
        match (&self.phase, &self.ready) {
            (Phase::NoAccounts, _) => message(&t!(cx, "shell-no-accounts")),
            (Phase::Starting, _) => message(&t!(cx, "shell-connecting")),
            (Phase::Expired, _) => div()
                .flex()
                .flex_col()
                .flex_1()
                .child(expiry_banner(cx, &self.expired_workspace_name(cx), {
                    let this = cx.entity();
                    move |window, cx| this.update(cx, |t, cx| t.sign_in_again(window, cx))
                }))
                .into_any_element(),
            (Phase::Failed(msg), _) => div()
                .flex()
                .flex_col()
                .flex_1()
                .child(error_strip(
                    &colors,
                    parla_tokens(cx.theme()).danger_soft,
                    &i18n::gpui::localizer(cx).text(msg),
                ))
                .child(message(&t!(cx, "shell-start-failed")))
                .into_any_element(),
            (Phase::Ready, Some(r)) => {
                let expired = r.session.read(cx).state().session == Session::Expired;
                let mut col = div().flex().flex_col().flex_1().min_w_0().h_full();
                if let Some(e) = r.session.read(cx).bootstrap_error() {
                    let session = r.session.clone();
                    let soft = parla_tokens(cx.theme()).danger_soft;
                    col = col.child(
                        error_strip(&colors, soft, &load_failed_text(cx, e)).child(
                            components::controls::on_activate_app(
                                components::controls::compact(components::controls::button(
                                    "retry-bootstrap",
                                    t!(cx, "shell-retry").to_string(),
                                    components::controls::ButtonKind::Secondary,
                                    true,
                                    &colors,
                                ))
                                .flex_none()
                                .debug_selector(|| "retry-bootstrap".into()),
                                move |_, cx| {
                                    session.update(cx, |s, cx| s.retry_bootstrap(cx));
                                },
                            ),
                        ),
                    );
                } else if r.session.read(cx).connecting() {
                    col = col.child(
                        div()
                            .id("connecting-banner")
                            .debug_selector(|| "connecting-banner".into())
                            .px(themes::metrics::SPACE_6)
                            .py(themes::metrics::SPACE_5)
                            .bg(parla_tokens(cx.theme()).warning_soft)
                            .text_color(colors.text)
                            .text_size(themes::metrics::FONT_SIZE_MD)
                            .child(t!(cx, "shell-still-connecting").to_string()),
                    );
                }
                if expired {
                    let name = r.session.read(cx).state().name.to_string();
                    let name = if name.is_empty() {
                        self.expired_workspace_name(cx)
                    } else {
                        name
                    };
                    col = col.child(expiry_banner(cx, &name, {
                        let this = cx.entity();
                        move |window, cx| this.update(cx, |t, cx| t.sign_in_again(window, cx))
                    }));
                }
                let (health, name) = {
                    let sess = r.session.read(cx);
                    (sess.store_health(), sess.display_name().to_string())
                };
                if health == StoreHealth::Unreadable {
                    let session = r.session.clone();
                    col = col.child(store_banner(cx, &name, move |_, _, cx| {
                        session.update(cx, |s, cx| s.rebuild_cache(cx));
                    }));
                }
                if crate::store_usage::StoreUsage::banner_visible(cx) {
                    col = col.child(crate::store_banner::store_full_banner(cx, |_, cx| {
                        crate::app_state::open_settings_page(
                            cx,
                            crate::settings_window::Page::Privacy,
                        )
                    }));
                }
                if self.offline_banner_visible(cx) {
                    let session = r.session.clone();
                    col = col.child(crate::offline_banner::offline_banner(cx, move |_, cx| {
                        session.read(cx).retry_connection()
                    }));
                }
                if let Some(c) = &r.collection {
                    let view = match &c.kind {
                        CollectionKind::Threads(v) => v.clone().into_any_element(),
                        CollectionKind::Later(v) => v.clone().into_any_element(),
                    };
                    return col
                        .child(div().flex_1().min_h_0().child(view))
                        .into_any_element();
                }
                match &r.chat {
                    Some(chat) => col
                        .relative()
                        .children({
                            let sess = r.session.read(cx);
                            let state = sess.state();
                            let mut actions = header_actions_for(
                                &self.header_fns,
                                &r.session,
                                &chat.conversation,
                                cx.entity().downgrade(),
                                matches!(r.right, Some(RightPanel::Details(_))),
                                cx,
                            );
                            actions.tab = if chat.files.is_some() {
                                components::ConversationTab::Files
                            } else {
                                components::ConversationTab::Messages
                            };
                            let tab_root = cx.entity().downgrade();
                            actions.switch_tab = Some(std::rc::Rc::new(move |tab, window, cx| {
                                tab_root
                                    .update(cx, |this, cx| this.show_tab(tab, window, cx))
                                    .ok();
                            }));
                            if state.caps.pins {
                                let root = cx.entity().downgrade();
                                actions.pins = Some(std::rc::Rc::new(move |id, window, cx| {
                                    root.update(cx, |this, cx| {
                                        this.toggle_pins(id.clone(), window, cx)
                                    })
                                    .ok();
                                }));
                            }
                            state.conversations.get(&chat.conversation).map(|c| {
                                components::conversation_header(
                                    sess,
                                    state,
                                    c,
                                    cx.theme(),
                                    &actions,
                                    cx,
                                )
                            })
                        })
                        .children(chat.huddle.clone())
                        .children(chat.bookmarks.clone())
                        .map(|col| match &chat.files {
                            Some(files) => col.child(div().flex_1().min_h_0().child(files.clone())),
                            None => col
                                .child(div().flex_1().min_h_0().child(chat.timeline.clone()))
                                .child(chat.composer.clone()),
                        })
                        .children(r.pins.as_ref().map(|p| {
                            div()
                                .absolute()
                                .top(components::header::TOTAL_HEIGHT + themes::metrics::SPACE_2)
                                .right(px(12.0))
                                .child(p.popover.clone())
                        }))
                        .into_any_element(),
                    None => col
                        .child(message(&t!(cx, "shell-select-conversation")))
                        .into_any_element(),
                }
            }
            (Phase::Ready, None) => message(&t!(cx, "shell-connecting")),
        }
    }
}

impl RootView {
    /// The call manager changed: an existing bar takes the new model itself
    /// (it repaints only when the model differs); only a bar that appears or
    /// goes repaints this view.
    fn call_changed(&mut self, cx: &mut Context<Self>) {
        use crate::call_manager::CallManagerHandle;
        let model = cx
            .try_global::<CallManagerHandle>()
            .and_then(|h| h.0.read(cx).bar_model(cx));
        match (model, self.call_bar.as_ref()) {
            (Some(m), Some(h)) => h.bar.update(cx, |b, cx| b.set_model(m, cx)),
            _ => cx.notify(),
        }
    }

    /// Creates, updates or drops the call bar from the call manager's model.
    fn sync_call_bar(&mut self, cx: &mut Context<Self>) {
        use crate::call_manager::{CallManager, CallManagerHandle};
        let model = cx
            .try_global::<CallManagerHandle>()
            .and_then(|h| h.0.read(cx).bar_model(cx));
        match (model, self.call_bar.as_ref()) {
            (Some(m), Some(h)) => h.bar.update(cx, |b, cx| b.set_model(m, cx)),
            (Some(m), None) => {
                let bar = cx.new(|cx| components::CallBar::new(m, cx));
                let sub = cx.subscribe(&bar, |_, _, e: &components::CallBarEvent, cx| match e {
                    components::CallBarEvent::ToggleMute => CallManager::toggle_mute(cx),
                    components::CallBarEvent::Leave => CallManager::leave(cx),
                    components::CallBarEvent::ShowWindow => CallManager::show_window(cx),
                    components::CallBarEvent::OpenInBrowser => CallManager::open_in_browser(cx),
                    components::CallBarEvent::ToggleDevices => CallManager::toggle_devices(cx),
                    components::CallBarEvent::CloseDevices => CallManager::close_devices(cx),
                    components::CallBarEvent::PickInput(d) => {
                        CallManager::pick_input(d.clone(), cx)
                    }
                    components::CallBarEvent::PickOutput(d) => {
                        CallManager::pick_output(d.clone(), cx)
                    }
                });
                self.call_bar = Some(CallBarHost { bar, _sub: sub });
            }
            (None, _) => self.call_bar = None,
        }
    }
}

/// Whether a history entry still points at an existing conversation.
fn nav_entry_valid<'a>(
    workspaces: &'a Entity<Workspaces>,
    cx: &'a App,
) -> impl Fn(&NavEntry) -> bool + 'a {
    move |e: &NavEntry| {
        workspaces.read(cx).session(&e.workspace).is_some_and(|s| {
            s.read(cx)
                .state()
                .conversations
                .contains_key(&e.conversation)
        })
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(not(feature = "fixture"))]
        let _ = &window;
        // Counts frames drawn. A hidden window is not drawn at all (the platform
        // stops its frames), so this stays flat while hidden.
        #[cfg(feature = "fixture")]
        {
            crate::perf::note_frame();
            let at = window.mouse_position();
            crate::perf::note_frame_input(
                f32::from(at.x) as i32,
                f32::from(at.y) as i32,
                window.is_window_active(),
            );
        }
        if crate::idle_lock::is_locked(cx) {
            let focus = self
                .lock_focus
                .get_or_insert_with(|| cx.focus_handle())
                .clone();
            return crate::idle_lock::cover(&focus, window, cx).into_any_element();
        }
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let tokens = parla_tokens(&theme);
        if let Some((ob, _)) = &self.onboarding {
            let error = self.error_banner(cx);
            return div()
                .debug_selector(|| "sign-in-screen".into())
                .size_full()
                .bg(colors.background)
                .text_color(colors.text)
                .font_family(components::live_settings::ui_font(cx).0)
                .text_size(components::live_settings::ui_font(cx).1)
                .children(error.map(|e| error_strip(colors, tokens.danger_soft, &e)))
                .child(titlebar::column(ob.clone()))
                .into_any_element();
        }
        if let Some((screen, _)) = &self.sign_in {
            let error = self.error_banner(cx);
            return div()
                .debug_selector(|| "sign-in-screen".into())
                .size_full()
                .bg(colors.background)
                .text_color(colors.text)
                .font_family(components::live_settings::ui_font(cx).0)
                .text_size(components::live_settings::ui_font(cx).1)
                .children(error.map(|e| error_strip(colors, tokens.danger_soft, &e)))
                .child(titlebar::column(screen.clone()))
                .into_any_element();
        }
        let banner_line = cx.global::<ConfigBanner>().line();
        self.sync_call_bar(cx);

        let items = self.rail_items(cx);
        let rail_me = self.rail_self(cx);
        let me = cx.entity();
        let mut actions = self
            .rail_actions
            .get_or_insert_with(|| {
                // Built once: the callbacks hold the root weakly (no cycle), so a
                // redraw allocates none of them.
                let w = self.workspaces.clone();
                let (a, b, c) = (me.downgrade(), me.downgrade(), me.downgrade());
                RailActions {
                    scroll: Default::default(),
                    activate: std::rc::Rc::new(move |key, _, cx| {
                        w.update(cx, |w, cx| w.activate(key, cx))
                    }),
                    context: std::rc::Rc::new(move |key, at, window, cx| {
                        a.update(cx, |this, cx| {
                            this.open_workspace_menu(key.clone(), at, window, cx)
                        })
                        .ok();
                    }),
                    add: std::rc::Rc::new(move |window, cx| {
                        b.update(cx, |this, cx| this.add_workspace(window, cx)).ok();
                    }),
                    aggregate: std::rc::Rc::new(move |kind, window, cx| {
                        c.update(cx, |this, cx| this.open_aggregate(kind, window, cx))
                            .ok();
                    }),
                    aggregate_open: None,
                    settings: Some(std::rc::Rc::new(|_, cx| {
                        crate::app_state::open_settings(cx)
                    })),
                    me: None,
                }
            })
            .clone();
        actions.aggregate_open = self.aggregate_kind();
        actions.me = rail_me;
        let rail_children = components::rail::render_rail(&items, &actions, cx);
        if actions.scroll.pending() {
            // The list is measured after its first frame; draw once more to
            // scroll the selected workspace into view.
            let me = cx.entity();
            window.on_next_frame(move |_, cx| me.update(cx, |_, cx| cx.notify()));
        }
        let rail = div()
            .debug_selector(|| "workspace-rail".into())
            .key_context("Rail")
            .track_focus(&self.rail_focus)
            .flex()
            .flex_col()
            .items_center()
            .w(themes::metrics::LAYOUT_RAIL)
            .h_full()
            .pt(themes::metrics::SPACE_5)
            .pb(themes::metrics::SPACE_6)
            .gap(themes::metrics::SPACE_5)
            .bg(tokens.sidebar_rail)
            .border_r_1()
            .border_color(tokens.sidebar_border)
            .children(rail_children);
        // Measured only while the tour shows: no extra elements otherwise.
        let tour_shown = self.tour.is_some();
        let rail = if tour_shown {
            self.tour_targets
                .track(components::tour::TourPart::Rail, rail)
        } else {
            rail
        };
        let sidebar = div()
            .debug_selector(|| "workspace-sidebar".into())
            .w(themes::metrics::LAYOUT_SIDEBAR)
            .h_full()
            .bg(tokens.sidebar_background)
            .text_color(tokens.sidebar_text)
            .border_r_1()
            .border_color(tokens.sidebar_border)
            .child(titlebar::body(
                div()
                    .size_full()
                    .children(self.ready.as_ref().map(|r| r.sidebar.clone())),
            ));
        let sidebar = if tour_shown {
            self.tour_targets
                .track(components::tour::TourPart::Sidebar, sidebar)
        } else {
            sidebar
        };
        let center = div()
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(colors.editor_background)
            .relative()
            .child(titlebar::body(match &self.aggregate {
                Some(a) => a
                    .view
                    .clone()
                    .cached(gpui::StyleRefinement::default().size_full())
                    .into_any_element(),
                None => self.center(cx),
            }))
            // Toasts: bottom left of the main pane, past the rail and sidebar.
            .child(components::Toasts::overlay(window, cx));
        // The search panel sizes itself (the hits, and the hit in context
        // beside them); every other panel takes the detail width.
        let sized = match self.ready.as_ref().and_then(|r| r.right.as_ref()) {
            Some(RightPanel::Search(p)) => {
                // The context pane takes only what the timeline can spare.
                let avail = window.viewport_size().width
                    - themes::metrics::LAYOUT_RAIL
                    - themes::metrics::LAYOUT_SIDEBAR;
                let width = components::search_context_width(avail);
                p.panel.update(cx, |p, _| p.set_context_width(width));
                true
            }
            _ => false,
        };
        let side: Option<AnyElement> = if self.aggregate.is_some() {
            None
        } else {
            self.ready
                .as_ref()
                .and_then(|r| r.right.as_ref())
                .map(|p| match p {
                    RightPanel::Search(p) => p.panel.clone().into_any_element(),
                    RightPanel::Scheduled(p) => p.view.clone().into_any_element(),
                    RightPanel::Reminders(p) => p.view.clone().into_any_element(),
                    RightPanel::Drafts(p) => p.view.clone().into_any_element(),
                    RightPanel::People(p) => p.view.clone().into_any_element(),
                    RightPanel::Profile(p) => p.panel.clone().into_any_element(),
                    RightPanel::Details(p) => p.panel.clone().into_any_element(),
                    RightPanel::Thread(p) => p.panel.clone().into_any_element(),
                })
        };
        // Empty until a thread, search or profile opens: the timeline takes the width.
        let panel = side.map(|p| {
            div()
                .debug_selector(|| "side-panel".into())
                .flex_none()
                .when(!sized, |d| d.w(themes::metrics::LAYOUT_DETAIL_PANEL))
                .when(
                    matches!(
                        self.ready.as_ref().and_then(|r| r.right.as_ref()),
                        Some(RightPanel::People(_))
                    ),
                    |d| d.w(themes::metrics::LAYOUT_PEOPLE_PANEL),
                )
                .h_full()
                .bg(colors.panel_background)
                .border_l_1()
                .border_color(colors.border)
                .child(titlebar::body(div().size_full().child(p)))
        });

        let titlebar_bar = cfg!(target_os = "macos").then(|| {
            let w = cx.entity();
            let (w_back, w_fwd, w_search, w_pause) = (w.clone(), w.clone(), w.clone(), w);
            let l = i18n::gpui::localizer(cx);
            let workspace = self
                .ready
                .as_ref()
                .map(|r| r.session.read(cx).state().name.to_string())
                .filter(|n| !n.is_empty());
            let search_label = match workspace {
                Some(name) => t!(l, "titlebar-search", workspace = name).into_owned(),
                None => t!(l, "titlebar-search-plain").into_owned(),
            };
            // The bell is crossed out while notifications are paused.
            let paused = self.ready.as_ref().is_some_and(|r| {
                r.session
                    .read(cx)
                    .notifications_paused(components::now_secs(cx))
            });
            titlebar::bar(
                titlebar::BarActions {
                    back: std::rc::Rc::new(move |_, window, cx| {
                        w_back.update(cx, |this, cx| this.go_back(window, cx))
                    }),
                    forward: std::rc::Rc::new(move |_, window, cx| {
                        w_fwd.update(cx, |this, cx| this.go_forward(window, cx))
                    }),
                    search: std::rc::Rc::new(move |_, window, cx| {
                        w_search.update(cx, |this, cx| this.open_search(window, cx))
                    }),
                    pause: std::rc::Rc::new(move |_, window, cx| {
                        w_pause.update(cx, |this, cx| this.pause_notifications(window, cx))
                    }),
                    help: std::rc::Rc::new(move |_, _, cx| cx.open_url(crate::menus::DOCS_URL)),
                    // Same rule as the buttons' handlers: no enabled button that does nothing.
                    can_back: self.nav.can_back_to(nav_entry_valid(&self.workspaces, cx)),
                    can_forward: self
                        .nav
                        .can_forward_to(nav_entry_valid(&self.workspaces, cx)),
                    paused,
                    search_label: search_label.into(),
                    back_label: t!(l, "titlebar-back").into_owned().into(),
                    forward_label: t!(l, "titlebar-forward").into_owned().into(),
                    pause_label: t!(l, "action-workspace-pause-notifications")
                        .into_owned()
                        .into(),
                    help_label: t!(l, "titlebar-help").into_owned().into(),
                },
                cx,
            )
        });
        let (ui_font, ui_size) = components::live_settings::ui_font(cx);
        let root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .font_family(ui_font)
            .text_size(ui_size);
        let (crash_notice, update_notice, message_line) = {
            let b = cx.global::<ConfigBanner>();
            if b.dismissed {
                (None, None, None)
            } else {
                (b.crash.clone(), b.update.clone(), b.message_line())
            }
        };
        let mut banner_el = None;
        let bar = || {
            div()
                .id("config-banner")
                .debug_selector(|| "config-banner".into())
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_1()
                .bg(colors.element_background)
                .text_color(colors.text_on_accent)
                .border_t_1()
                .border_color(colors.border)
        };
        let dismissable = |line: gpui::SharedString| {
            bar()
                .cursor_pointer()
                .on_click(|_, window, cx| {
                    cx.global_mut::<ConfigBanner>().dismissed = true;
                    window.refresh();
                })
                .child(line)
                .into_any_element()
        };
        if let Some(update) = update_notice {
            banner_el = Some(crate::update_banner::update_banner(cx, &update).into_any_element());
        } else if let Some(line) = message_line {
            banner_el = Some(dismissable(line));
        } else if let Some(crash) = crash_notice {
            banner_el = Some(
                bar()
                    .child(
                        div()
                            .id("config-banner-dismiss")
                            .flex_1()
                            .min_w_0()
                            .cursor_pointer()
                            .on_click(|_, window, cx| {
                                let banner = cx.global_mut::<ConfigBanner>();
                                banner.dismissed = true;
                                banner.crash = None;
                                window.refresh();
                            })
                            .child(crash.text.clone()),
                    )
                    .child({
                        let dir = crash.dir.clone();
                        div()
                            .id("crash-show")
                            .flex_none()
                            .px_2()
                            .rounded_md()
                            .border_1()
                            .border_color(colors.text_on_accent)
                            .cursor_pointer()
                            .on_click(move |_, window, cx| {
                                crate::crash::open_dir(&dir);
                                cx.global_mut::<ConfigBanner>().crash = None;
                                window.refresh();
                            })
                            .child(crash.show_label.clone())
                    })
                    .child(
                        div()
                            .id("crash-delete")
                            .debug_selector(|| "crash-delete".into())
                            .flex_none()
                            .px_2()
                            .rounded_md()
                            .border_1()
                            .border_color(colors.text_on_accent)
                            .cursor_pointer()
                            .on_click(move |_, window, cx| {
                                crate::crash::delete_reports(&crash.dir);
                                cx.global_mut::<ConfigBanner>().crash = None;
                                window.refresh();
                            })
                            .child(crash.delete_label.clone()),
                    )
                    .into_any_element(),
            );
        } else if let Some(line) = banner_line {
            banner_el = Some(dismissable(line));
        }
        root.track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &RebuildCache, _, cx| this.rebuild_cache(cx)))
            .on_action(
                cx.listener(|this, _: &crate::actions::FocusNextPane, window, cx| {
                    this.cycle_focus(true, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::actions::FocusPreviousPane, window, cx| {
                    this.cycle_focus(false, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ActivateWorkspace1, _, cx| {
                let k = components::rail::key_at(&this.rail_items(cx), 0);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateWorkspace2, _, cx| {
                let k = components::rail::key_at(&this.rail_items(cx), 1);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateWorkspace3, _, cx| {
                let k = components::rail::key_at(&this.rail_items(cx), 2);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateWorkspace4, _, cx| {
                let k = components::rail::key_at(&this.rail_items(cx), 3);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateWorkspace5, _, cx| {
                let k = components::rail::key_at(&this.rail_items(cx), 4);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateWorkspace6, _, cx| {
                let k = components::rail::key_at(&this.rail_items(cx), 5);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateWorkspace7, _, cx| {
                let k = components::rail::key_at(&this.rail_items(cx), 6);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateWorkspace8, _, cx| {
                let k = components::rail::key_at(&this.rail_items(cx), 7);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateWorkspace9, _, cx| {
                let k = components::rail::key_at(&this.rail_items(cx), 8);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &NextWorkspace, _, cx| {
                let k = components::rail::key_step(&this.rail_items(cx), 1);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousWorkspace, _, cx| {
                let k = components::rail::key_step(&this.rail_items(cx), -1);
                this.switch_workspace(k, cx)
            }))
            .on_action(cx.listener(|this, _: &NextUnread, _, cx| this.next_unread(cx)))
            .on_action(cx.listener(|this, _: &Search, window, cx| this.open_search(window, cx)))
            .on_action(
                cx.listener(|this, _: &QuickSwitch, window, cx| {
                    this.open_quick_switcher(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::actions::CommandPalette, window, cx| {
                    this.open_command_palette(window, cx)
                }),
            )
            .on_action(cx.listener(
                |this, _: &crate::actions::OpenKeyboardShortcuts, window, cx| {
                    this.open_shortcuts_sheet(window, cx)
                },
            ))
            .on_action(cx.listener(|this, _: &AllUnreads, window, cx| {
                this.open_aggregate(AggregateKind::Unreads, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AllMentions, window, cx| {
                this.open_aggregate(AggregateKind::Mentions, window, cx)
            }))
            .on_action(cx.listener(|this, _: &AllScheduled, window, cx| {
                this.open_aggregate(AggregateKind::Scheduled, window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.go_back(window, cx)))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| this.go_forward(window, cx)))
            .on_action(
                cx.listener(|this, _: &SignInAgain, window, cx| this.sign_in_again(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SignOut, window, cx| this.sign_out(window, cx)))
            .on_action(
                cx.listener(|this, _: &SetStatus, window, cx| this.set_status_action(window, cx)),
            )
            .on_action(cx.listener(|this, _: &PauseNotifications, window, cx| {
                this.pause_notifications(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RemoveWorkspace, window, cx| {
                this.remove_workspace(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &DeleteAllData, window, cx| this.delete_all_data(window, cx)),
            )
            .on_action(cx.listener(|this, _: &PopOutConversation, window, cx| {
                this.pop_out_conversation(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &PopOutThread, window, cx| this.pop_out_thread(window, cx)),
            )
            .children(titlebar_bar)
            .children(
                self.notice
                    .as_deref()
                    .map(|n| error_strip(colors, tokens.danger_soft, n)),
            )
            .children(
                cx.try_global::<crate::ringer::Ringer>()
                    .and_then(|r| r.banner(cx))
                    .map(|d| {
                        components::ring_banner(
                            &d,
                            |_, _, cx| crate::ringer::Ringer::join(cx),
                            |_, _, cx| crate::ringer::Ringer::decline(cx),
                            cx,
                        )
                    }),
            )
            .children(self.call_bar.as_ref().map(|h| h.bar.clone()))
            .child(
                div()
                    .flex()
                    .flex_1()
                    // Without it the row grows to its tallest column and pushes
                    // the composer and the bottom bars out of the window.
                    .min_h_0()
                    .child(rail)
                    .child(sidebar)
                    .child(center)
                    .children(panel),
            )
            // Below the columns so it never sits under the traffic lights.
            .children(banner_el)
            .children(crate::mcp_status::McpStatus::text(cx).map(|text| {
                div()
                    .id("mcp-status")
                    .debug_selector(|| "mcp-status".into())
                    .px_3()
                    .py_0p5()
                    .text_xs()
                    .text_color(colors.text_muted)
                    .border_t_1()
                    .border_color(colors.border)
                    .cursor_pointer()
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::OpenAuditLog), cx)
                    })
                    .child(text)
            }))
            .children(self.menu.as_ref().map(|m| {
                gpui::deferred(
                    gpui::anchored()
                        .position(m.at)
                        .snap_to_window_with_margin(px(8.0))
                        .child(
                            div()
                                .debug_selector(|| "notify-menu-host".into())
                                .on_mouse_down_out(
                                    cx.listener(|this, _, window, cx| this.close_menu(window, cx)),
                                )
                                .child(m.menu.clone()),
                        ),
                )
                .priority(1)
            }))
            .children(self.status.as_ref().map(|s| {
                scrim("status-scrim")
                    .debug_selector(|| "status-dialog".into())
                    .pt(themes::metrics::LAYOUT_DIALOG_TOP)
                    .child(
                        div()
                            .w(themes::metrics::LAYOUT_DIALOG_SM)
                            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                                this.close_status_dialog(window, cx)
                            }))
                            .child(s.dialog.clone()),
                    )
            }))
            .children(self.render_admin(cx))
            .children(
                self.delete_confirm
                    .as_ref()
                    .map(|p| crate::delete_confirm::render_scrim(p, cx)),
            )
            .children(self.switcher.as_ref().map(|s| {
                scrim("switcher-scrim")
                    .debug_selector(|| "quick-switcher".into())
                    .px(themes::metrics::SPACE_6)
                    .pt(themes::metrics::LAYOUT_SWITCHER_TOP)
                    .child(
                        div()
                            .w_full()
                            .max_w(themes::metrics::LAYOUT_SWITCHER_WIDTH)
                            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                                this.close_quick_switcher(window, cx)
                            }))
                            .child(s.picker.clone()),
                    )
            }))
            .children(self.shortcuts_sheet.as_ref().map(|s| {
                scrim("shortcuts-sheet-scrim")
                    .px(themes::metrics::SPACE_6)
                    .pt(themes::metrics::LAYOUT_DIALOG_TOP)
                    .pb(themes::metrics::SPACE_9)
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .max_w(themes::metrics::LAYOUT_SHEET_WIDTH)
                            .max_h_full()
                            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                                this.close_shortcuts_sheet(window, cx);
                            }))
                            .child(s.sheet.clone()),
                    )
            }))
            .children(self.command_palette.as_ref().map(|s| {
                scrim("command-palette-scrim")
                    .debug_selector(|| "command-palette".into())
                    .px(themes::metrics::SPACE_6)
                    .pt(themes::metrics::LAYOUT_SWITCHER_TOP)
                    .child(
                        div()
                            .w_full()
                            .max_w(themes::metrics::LAYOUT_SWITCHER_WIDTH)
                            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                                this.close_command_palette(window, cx);
                            }))
                            .child(s.picker.clone()),
                    )
            }))
            // The tour card steps aside while an overlay is open and comes back
            // when it closes.
            .children(
                self.tour
                    .as_ref()
                    .filter(|_| {
                        self.switcher.is_none()
                            && self.shortcuts_sheet.is_none()
                            && self.command_palette.is_none()
                    })
                    .map(|(t, _)| t.clone()),
            )
            .idle_input()
            .into_any_element()
    }
}

impl RootView {
    /// The name of the focused major pane, for tests and accessibility.
    pub fn focused_pane_name(&self, window: &Window, cx: &App) -> Option<&'static str> {
        self.focused_pane(window, cx).map(|p| match p {
            Pane::Rail => "rail",
            Pane::Sidebar => "sidebar",
            Pane::Timeline => "timeline",
            Pane::Composer => "composer",
            Pane::RightPanel => "right_panel",
        })
    }

    /// The focused major pane.
    pub fn focused_pane(&self, window: &Window, cx: &App) -> Option<Pane> {
        for pane in [
            Pane::Rail,
            Pane::Sidebar,
            Pane::Timeline,
            Pane::Composer,
            Pane::RightPanel,
        ] {
            if let Some(h) = self.pane_focus_handle(pane, cx)
                && h.contains_focused(window, cx)
            {
                return Some(pane);
            }
        }
        None
    }

    /// Moves focus to `pane` when it is present.
    pub fn focus_pane(&mut self, pane: Pane, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = self.pane_focus_handle(pane, cx) {
            window.focus(&h, cx);
        }
    }

    fn right_panel_focus(&self, cx: &App) -> Option<gpui::FocusHandle> {
        let ready = self.ready.as_ref()?;
        if self.aggregate.is_some() {
            return None;
        }
        ready.right.as_ref().map(|p| p.focus_handle(cx))
    }

    fn pane_focus_handle(&self, pane: Pane, cx: &App) -> Option<gpui::FocusHandle> {
        match pane {
            Pane::Rail => Some(self.rail_focus.clone()),
            Pane::Sidebar => self.ready.as_ref().map(|r| r.sidebar.focus_handle(cx)),
            Pane::Timeline => {
                if let Some(a) = &self.aggregate {
                    return Some(a.view.focus_handle(cx));
                }
                if let Some(c) = self.ready.as_ref().and_then(|r| r.collection.as_ref()) {
                    return match &c.kind {
                        CollectionKind::Threads(v) => Some(v.focus_handle(cx)),
                        CollectionKind::Later(v) => Some(v.focus_handle(cx)),
                    };
                }
                self.ready
                    .as_ref()
                    .and_then(|r| r.chat.as_ref())
                    .map(|c| c.timeline.focus_handle(cx))
            }
            Pane::Composer => {
                if self.aggregate.is_some()
                    || self
                        .ready
                        .as_ref()
                        .and_then(|r| r.collection.as_ref())
                        .is_some()
                {
                    return None;
                }
                self.ready
                    .as_ref()
                    .and_then(|r| r.chat.as_ref())
                    .map(|c| c.composer.focus_handle(cx))
            }
            Pane::RightPanel => self.right_panel_focus(cx),
        }
    }

    fn focus_cycle(&self, cx: &App) -> Vec<Pane> {
        let mut panes = vec![Pane::Rail];
        if self.ready.is_some() {
            panes.push(Pane::Sidebar);
        }
        let collection = self
            .ready
            .as_ref()
            .and_then(|r| r.collection.as_ref())
            .is_some();
        if self.aggregate.is_some() || collection {
            panes.push(Pane::Timeline);
        } else if self.ready.as_ref().and_then(|r| r.chat.as_ref()).is_some() {
            panes.push(Pane::Timeline);
            panes.push(Pane::Composer);
        }
        if self.right_panel_focus(cx).is_some() {
            panes.push(Pane::RightPanel);
        }
        panes
    }

    fn cycle_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let cycle = self.focus_cycle(cx);
        if cycle.is_empty() {
            return;
        }
        let current = self.focused_pane(window, cx);
        match current {
            None => {
                let target = if forward {
                    cycle[0]
                } else {
                    cycle[cycle.len() - 1]
                };
                self.focus_pane(target, window, cx);
            }
            Some(p) => {
                let idx = cycle.iter().position(|c| *c == p).unwrap_or(0);
                let next = if forward {
                    (idx + 1) % cycle.len()
                } else {
                    (idx + cycle.len().saturating_sub(1)) % cycle.len()
                };
                self.focus_pane(cycle[next], window, cx);
            }
        }
    }
}

/// Channel links and mentions in the header topic open like in a message.
/// [`header_actions`] for `open`: the callbacks are built once per session
/// (they take the conversation as an argument), only the flags per frame.
fn header_actions_for(
    cache: &std::cell::RefCell<Option<(gpui::EntityId, components::HeaderActions)>>,
    session: &Entity<WorkspaceSession>,
    open: &ConversationId,
    root: gpui::WeakEntity<RootView>,
    details_open: bool,
    cx: &App,
) -> components::HeaderActions {
    let id = session.entity_id();
    let mut cache = cache.borrow_mut();
    if cache.as_ref().is_none_or(|(s, _)| *s != id) {
        *cache = Some((id, header_actions(session, root)));
    }
    let mut a = cache.as_ref().map(|(_, a)| a.clone()).unwrap_or_default();
    a.starred = session.read(cx).is_starred(open);
    a.details_open = details_open;
    a.clock = cx
        .try_global::<SettingsStore>()
        .map(|s| s.current().messages.clock);
    if components::huddle_action(session.read(cx), open, cx).is_none() {
        a.huddle = None;
    }
    a
}

fn header_actions(
    session: &Entity<WorkspaceSession>,
    root: gpui::WeakEntity<RootView>,
) -> components::HeaderActions {
    let for_user = session.clone();
    let for_huddle = session.clone();
    let menu_root = root.clone();
    let details_root = root.clone();
    let members_root = root.clone();
    let more_root = root.clone();
    let notify: components::NotifyMenuFn = std::rc::Rc::new(move |id, at, window, cx| {
        menu_root
            .update(cx, |this, cx| {
                this.open_notify_controls(id.clone(), at, window, cx)
            })
            .ok();
    });
    let more: components::NotifyMenuFn = std::rc::Rc::new(move |id, at, window, cx| {
        more_root
            .update(cx, |this, cx| {
                this.open_conversation_actions(id.clone(), at, window, cx)
            })
            .ok();
    });
    let session = session.clone();
    // Stars are Parla's own (local, per workspace), so every provider has them.
    let star_session = session.clone();
    let (starred, details_open) = (false, false);
    components::HeaderActions {
        clock: None,
        tab: components::ConversationTab::Messages,
        switch_tab: None,
        pins: None,
        notify_menu: Some(notify),
        more_menu: Some(more),
        star: Some(std::rc::Rc::new(move |id, _, cx| {
            let id = id.clone();
            star_session.update(cx, |s, cx| {
                let now = !s.is_starred(&id);
                s.set_starred(id, now, cx)
            });
        })),
        starred,
        details: Some(std::rc::Rc::new(move |_, window, cx| {
            details_root
                .update(cx, |this, cx| this.toggle_details(window, cx))
                .ok();
        })),
        details_open,
        members: Some(std::rc::Rc::new(move |_, window, cx| {
            members_root
                .update(cx, |this, cx| {
                    this.show_details(components::DetailsTab::Members, window, cx)
                })
                .ok();
        })),
        huddle: Some(
            std::rc::Rc::new(move |id: &ConversationId, _: &mut Window, cx: &mut App| {
                let id = id.clone();
                for_huddle.update(cx, |s, cx| components::HuddleSource::join_huddle(s, id, cx));
            }) as components::ToggleFn,
        ),
        conversation: Some(std::rc::Rc::new(move |id, _, cx| {
            session.update(cx, |s, cx| {
                if s.state().conversations.contains_key(&id) {
                    s.open(id, cx);
                }
            })
        })),
        user: Some(std::rc::Rc::new(move |user, _, cx| {
            for_user.update(cx, |s, cx| {
                components::TimelineSource::open_profile(s, user, cx)
            })
        })),
    }
}

/// The rail's own avatar and what it was built from.
struct RailSelfCache {
    session: gpui::EntityId,
    name: std::sync::Arc<str>,
    active: Option<bool>,
    avatar: Option<domain::ImageRef>,
    item: components::rail::RailSelf,
}
