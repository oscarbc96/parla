//! The multi-workspace manager: one [`WorkspaceSession`] per signed-in
//! account, all started at once; lean mode for workspaces nobody is looking at;
//! removal of one workspace and "Delete all data".

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use components::{
    AggregateSource, MentionItem, MentionLine, ScheduledItem, SendLaterConfig, UnreadItem,
    WorkspaceTag,
};
use domain::{
    AccountInfo, ChatProvider, Input, ProviderError, WorkspaceRef, all_mentions, all_scheduled,
    all_unreads,
};
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, SharedString, Task};
use settings::WorkspaceKey;
use store::Store;
use store_sqlite::SqliteStore;

use crate::providers::{ProviderRegistry, RemoveError};
use crate::removal::{AppDirs, remove_dir_all_checked, remove_workspace_media};
use crate::root_view::SqliteHandle;
use crate::session::WorkspaceSession;

/// Name of the non-secret app state file in the data dir.
const STATE_FILE: &str = "state.json";

#[derive(Debug)]
pub enum WorkspacesEvent {
    /// The set of sessions, the accounts or a lean flag changed.
    Changed,
    /// This workspace became the active one.
    Activated(WorkspaceKey),
    /// No registered provider owns this account.
    UnknownProvider(WorkspaceKey),
    /// A workspace has overdue local schedules to ask about.
    LatePrompt(WorkspaceKey),
    /// A workspace could not start (an expired session included).
    StartFailed(WorkspaceKey, ProviderError),
    /// A queued link is ready to open now.
    OpenLink(String),
}

struct Entry {
    key: WorkspaceKey,
    session: Entity<WorkspaceSession>,
    provider: Arc<dyn ChatProvider>,
    /// The one-shot lean timer, while the workspace is hidden.
    timer: Option<Task<()>>,
}

pub struct Workspaces {
    registry: ProviderRegistry,
    store: Arc<dyn Store>,
    sqlite: Option<SqliteStore>,
    dirs: Option<AppDirs>,
    accounts: Vec<AccountInfo>,
    entries: Vec<Entry>,
    active: Option<WorkspaceKey>,
    window_visible: bool,
    /// Orders the background `state.json` writes: newest wins, one at a time.
    active_writes: Arc<(std::sync::atomic::AtomicU64, std::sync::Mutex<()>)>,
    /// No window yet (`parla --headless`): every workspace goes lean at once.
    headless: bool,
    /// The idle lock is on: every workspace is lean until it is lifted.
    locked: bool,
    /// Workspaces that finished starting while locked: their sessions (and
    /// stores) open at unlock.
    deferred: Vec<(Arc<dyn ChatProvider>, Arc<dyn domain::ProviderFactory>)>,
    started: bool,
    /// Open pop-out windows per workspace; any keeps it out of lean mode.
    popouts: HashMap<WorkspaceKey, usize>,
    /// Inbound links waiting for the workspace sessions they need.
    pending_links: Vec<String>,
    /// Workspace starts not finished yet (success or failure).
    start_outstanding: usize,
    /// Per workspace, bumped by every start and stop: a start that finishes
    /// after a later one or after a removal is stale and dropped.
    start_gen: HashMap<WorkspaceKey, u64>,
}

/// The manager entity, so views and actions can reach it.
pub struct WorkspacesHandle(pub Entity<Workspaces>);

impl gpui::Global for WorkspacesHandle {}

impl EventEmitter<WorkspacesEvent> for Workspaces {}

impl Workspaces {
    /// `store` is used when the app runs without the encrypted store. The
    /// encrypted store and the data and cache dirs come from the
    /// [`SqliteHandle`] and [`AppDirs`] globals when set.
    pub fn new(registry: &ProviderRegistry, store: Arc<dyn Store>, cx: &mut Context<Self>) -> Self {
        Self {
            registry: registry.clone(),
            store,
            sqlite: cx.try_global::<SqliteHandle>().map(|h| h.0.clone()),
            dirs: cx.try_global::<AppDirs>().cloned(),
            accounts: registry.account_infos(),
            entries: Vec::new(),
            active: None,
            window_visible: true,
            active_writes: Default::default(),
            headless: false,
            locked: false,
            deferred: Vec::new(),
            started: false,
            popouts: HashMap::new(),
            pending_links: Vec::new(),
            start_outstanding: 0,
            start_gen: HashMap::new(),
        }
    }

    /// Overrides the data and cache dirs (tests use scratch dirs).
    pub fn with_dirs(mut self, data: PathBuf, cache: PathBuf) -> Self {
        self.dirs = Some(AppDirs { data, cache });
        self
    }

    pub fn sessions(&self) -> impl Iterator<Item = &Entity<WorkspaceSession>> {
        self.entries.iter().map(|e| &e.session)
    }

    pub fn active(&self) -> Option<&WorkspaceKey> {
        self.active.as_ref()
    }

    pub fn session(&self, key: &WorkspaceKey) -> Option<Entity<WorkspaceSession>> {
        self.entries
            .iter()
            .find(|e| &e.key == key)
            .map(|e| e.session.clone())
    }

    /// The active workspace's session, once it has started.
    pub fn active_session(&self) -> Option<Entity<WorkspaceSession>> {
        self.active.as_ref().and_then(|k| self.session(k))
    }

    pub fn accounts(&self) -> &[AccountInfo] {
        &self.accounts
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// Lean timers still pending (one per hidden, not yet lean workspace).
    pub fn lean_timers(&self) -> usize {
        self.entries.iter().filter(|e| e.timer.is_some()).count()
    }

    /// Starts every account of every provider at once. The workspace that was
    /// active last time (from `state.json`), else the first, becomes active.
    pub fn start_all(&mut self, cx: &mut Context<Self>) {
        self.started = true;
        self.accounts = self.registry.account_infos();
        let keys: Vec<WorkspaceKey> = self.accounts.iter().map(|a| a.key.clone()).collect();
        if self.active.is_none() {
            let saved = self.read_active();
            self.active = saved
                .filter(|k| keys.contains(k))
                .or_else(|| keys.first().cloned());
        }
        if keys.is_empty() {
            // No workspaces will start: queued https permalinks can fall back now.
            self.process_pending(cx, true);
        }
        for key in keys {
            self.start(key, cx);
        }
    }

    /// Starts one account through its factory and adds (or replaces) its
    /// session. Used at startup and after a new sign-in.
    pub fn start(&mut self, key: WorkspaceKey, cx: &mut Context<Self>) {
        let Some(factory) = self.registry.factory_for(&key) else {
            cx.emit(WorkspacesEvent::UnknownProvider(key));
            self.process_pending(cx, self.start_outstanding == 0);
            return;
        };
        if !self.accounts.iter().any(|a| a.key == key) {
            self.accounts = self.registry.account_infos();
        }
        self.start_outstanding += 1;
        let generation = self.bump_start_gen(&key);
        let handle = crate::runtime::handle(cx);
        cx.spawn(async move |this, cx| {
            let k = key.clone();
            let f = factory.clone();
            let start = async move { f.start(&k).await };
            let result = match handle {
                Some(h) => h
                    .spawn(start)
                    .await
                    .unwrap_or_else(|e| Err(ProviderError::Other(e.to_string()))),
                None => start.await,
            };
            this.update(cx, |w, cx| match result {
                _ if w.start_gen.get(&key) != Some(&generation) => {
                    // Removed or restarted while starting: drop the late result.
                    w.start_outstanding = w.start_outstanding.saturating_sub(1);
                    let done = w.start_outstanding == 0;
                    w.process_pending(cx, done);
                }
                Ok(provider) => w.add_session(provider, factory, cx),
                Err(e) => {
                    w.start_outstanding = w.start_outstanding.saturating_sub(1);
                    let done = w.start_outstanding == 0;
                    cx.emit(WorkspacesEvent::StartFailed(key, e));
                    w.process_pending(cx, done);
                }
            })
            .ok();
        })
        .detach();
    }

    /// A new or re-signed-in account: starts it and makes it the active one.
    pub fn adopt(&mut self, key: WorkspaceKey, cx: &mut Context<Self>) {
        self.accounts = self.registry.account_infos();
        let previous = self.active.replace(key.clone());
        if let Some(prev) = previous.filter(|p| p != &key) {
            self.refresh_timer(&prev, cx);
        }
        self.write_active(&key, cx);
        self.start(key, cx);
    }

    /// The account was signed out elsewhere: drops its session and shows the
    /// next workspace, if any.
    pub fn forget(&mut self, key: &WorkspaceKey, cx: &mut Context<Self>) {
        self.stop(key, cx);
        self.clear_popout_count(key);
        Self::close_popouts_later(key, cx);
        self.accounts = self.registry.account_infos();
        self.next_active(key, cx);
        self.changed(cx);
    }

    fn add_session(
        &mut self,
        provider: Arc<dyn ChatProvider>,
        factory: Arc<dyn domain::ProviderFactory>,
        cx: &mut Context<Self>,
    ) {
        self.start_outstanding = self.start_outstanding.saturating_sub(1);
        if self.locked {
            // No store opens and no key is read while locked.
            let key = provider.workspace().clone();
            self.deferred.retain(|(p, _)| p.workspace() != &key);
            self.deferred.push((provider, factory));
            return;
        }
        self.open_session(provider, factory, cx);
    }

    fn open_session(
        &mut self,
        provider: Arc<dyn ChatProvider>,
        factory: Arc<dyn domain::ProviderFactory>,
        cx: &mut Context<Self>,
    ) {
        let key = provider.workspace().clone();
        // A restart after re-sign-in keeps the open conversation.
        let reopen = self
            .session(&key)
            .and_then(|s| s.read(cx).selected().cloned());
        let replacing = self.session(&key).is_some() || self.popout_count(&key) > 0;
        crate::popout::allow_persist(cx);
        self.entries.retain(|e| e.key != key);
        if replacing {
            self.clear_popout_count(&key);
            Self::close_popouts_later(&key, cx);
        }
        let (store, sqlite) = (self.store.clone(), self.sqlite.clone());
        let (codec, parser, name) = (
            factory.ext_codec(),
            factory.content_parser(),
            factory.display_name().to_string(),
        );
        let encoder = factory.encoder();
        let p = provider.clone();
        let session = cx.new(|cx| {
            let mut s = match sqlite {
                Some(db) => WorkspaceSession::new_sqlite(p, db, codec, cx),
                None => WorkspaceSession::new(p, store, cx),
            };
            s.set_content(parser, name);
            s.set_encoder(encoder);
            s
        });
        if let Some(c) = reopen {
            session.update(cx, |s, cx| s.open(c, cx));
        }
        // Overdue local schedules are asked about whichever workspace is shown,
        // and the cross-workspace views read every session: tell them when the
        // lists they show may have changed (not for each message).
        let late_key = key.clone();
        cx.subscribe(
            &session,
            move |_, _, ev: &crate::session::SessionEvent, cx| {
                use crate::session::SessionEvent as E;
                match ev {
                    E::LatePrompt => cx.emit(WorkspacesEvent::LatePrompt(late_key.clone())),
                    E::Conversations | E::Scheduled | E::Members | E::Lean => cx.notify(),
                    _ => {}
                }
            },
        )
        .detach();
        self.registry.set_running(&key, true);
        self.entries.push(Entry {
            key: key.clone(),
            session,
            provider,
            timer: None,
        });
        if self.active.is_none() {
            self.active = Some(key.clone());
        }
        if self.active.as_ref() == Some(&key) {
            self.install_media(&key, cx);
        }
        self.refresh_timer(&key, cx);
        // Before `process_pending`: a queued link opens in the window, which
        // must already show this session.
        cx.emit(WorkspacesEvent::Changed);
        crate::tray::Tray::request(cx);
        self.process_pending(cx, self.start_outstanding == 0);
        cx.spawn(async move |_, cx| {
            let _ = cx.update(crate::popout::restore_pending);
        })
        .detach();
        cx.notify();
    }

    fn target_workspace(url: &str) -> Option<WorkspaceKey> {
        match crate::links::parse_parla_url(url)? {
            crate::links::ParlaLink::Conversation { ws, .. }
            | crate::links::ParlaLink::Thread { ws, .. } => Some(ws),
            crate::links::ParlaLink::Open { .. } => None,
        }
    }

    /// Queues an inbound link so it opens once the workspace session it needs is
    /// ready. Links that can be resolved now are emitted as [`WorkspacesEvent::OpenLink`].
    /// When all saved workspaces have started, https permalinks that match no
    /// signed-in workspace fall back to the browser; other unresolvable links are dropped.
    pub fn queue_link(&mut self, url: String, cx: &mut Context<Self>) {
        self.pending_links.push(url);
        self.process_pending(cx, self.start_outstanding == 0);
    }

    /// True while links must wait for workspaces that have not started yet.
    pub fn is_starting(&self) -> bool {
        self.start_outstanding > 0
    }

    fn process_pending(&mut self, cx: &mut Context<Self>, all_done: bool) {
        if self.pending_links.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending_links);
        let mut keep = Vec::new();
        for url in pending {
            let resolved = crate::links::resolve_url_with(&url, Some(self), cx);
            match resolved {
                crate::links::ResolvedLink::Jump { .. }
                | crate::links::ResolvedLink::OpenThread { .. } => {
                    cx.emit(WorkspacesEvent::OpenLink(url));
                }
                crate::links::ResolvedLink::OpenBrowser(u) => {
                    if all_done {
                        cx.open_url(&u);
                    } else {
                        // A permalink may match a workspace that has not started yet.
                        keep.push(url);
                    }
                }
                crate::links::ResolvedLink::Ignored => {
                    if all_done {
                        // A workspace-specific link for a workspace that never signed in.
                    } else {
                        let waiting = Self::target_workspace(&url)
                            .map(|ws| self.accounts.iter().any(|a| a.key == ws))
                            .unwrap_or(true);
                        if waiting {
                            keep.push(url);
                        }
                    }
                }
            }
        }
        self.pending_links = keep;
    }

    fn install_media(&self, key: &WorkspaceKey, cx: &mut Context<Self>) {
        let Some(e) = self.entries.iter().find(|e| &e.key == key) else {
            return;
        };
        let mb = e.session.read(cx).settings(cx).performance.image_cache_mb;
        let cache = self
            .dirs
            .as_ref()
            .map_or_else(settings::paths::cache_dir, |d| d.cache.clone());
        crate::media::install(cx, e.provider.clone(), key, &cache, mb);
    }

    /// Makes `key` the active workspace: cancels its lean timer, wakes it if
    /// it was lean, and starts the timer of the one it replaces.
    pub fn activate(&mut self, key: &WorkspaceKey, cx: &mut Context<Self>) {
        if !self.accounts.iter().any(|a| &a.key == key) && self.session(key).is_none() {
            return;
        }
        let previous = self.active.replace(key.clone());
        let changed = previous.as_ref() != Some(key);
        if let Some(prev) = previous.filter(|p| p != key) {
            self.refresh_timer(&prev, cx);
        }
        self.refresh_timer(key, cx);
        // Reinstalling replaces the image cache and drops its decoded entries.
        if changed {
            // The viewer shows another workspace's files: it never carries over.
            crate::media_viewer::close(cx);
            crate::code_viewer::close(cx);
            self.install_media(key, cx);
        }
        self.write_active(key, cx);
        cx.emit(WorkspacesEvent::Activated(key.clone()));
        self.changed(cx);
    }

    /// Whether [`Workspaces::start_all`] ran.
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// Headless mode: no window, so no workspace is looked at and each one is
    /// lean as soon as it starts. Leaving it behaves like a window shown.
    pub fn set_headless(&mut self, headless: bool, cx: &mut Context<Self>) {
        if self.headless == headless {
            return;
        }
        self.headless = headless;
        if headless {
            self.window_visible = false;
            let keys: Vec<_> = self.entries.iter().map(|e| e.key.clone()).collect();
            for k in keys {
                self.refresh_timer(&k, cx);
            }
        } else {
            self.window_visible = false;
            self.set_window_visible(true, cx);
        }
    }

    /// The window was hidden or shown. While hidden, even the active workspace
    /// is not looked at, so its lean timer runs.
    pub fn set_window_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.window_visible == visible {
            return;
        }
        self.window_visible = visible;
        let keys: Vec<_> = self.entries.iter().map(|e| e.key.clone()).collect();
        for k in keys {
            self.refresh_timer(&k, cx);
        }
    }

    pub fn popout_count(&self, key: &WorkspaceKey) -> usize {
        self.popouts.get(key).copied().unwrap_or(0)
    }

    pub fn clear_popout_count(&mut self, key: &WorkspaceKey) {
        self.popouts.remove(key);
    }

    pub fn clear_all_popout_counts(&mut self) {
        self.popouts.clear();
    }

    pub fn popout_opened(&mut self, key: &WorkspaceKey, cx: &mut Context<Self>) {
        *self.popouts.entry(key.clone()).or_insert(0) += 1;
        self.refresh_timer(key, cx);
        cx.notify();
    }

    pub fn popout_closed(&mut self, key: &WorkspaceKey, cx: &mut Context<Self>) {
        if let Some(n) = self.popouts.get_mut(key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                self.popouts.remove(key);
            }
        }
        self.refresh_timer(key, cx);
        cx.notify();
    }

    /// Announces a change that shows in the window, the rail and the tray.
    fn changed(&self, cx: &mut Context<Self>) {
        cx.emit(WorkspacesEvent::Changed);
        crate::tray::Tray::request(cx);
        cx.notify();
    }

    /// Closes the pop-outs of `key` once the current update is done.
    fn close_popouts_later(key: &WorkspaceKey, cx: &mut Context<Self>) {
        let k = key.clone();
        cx.spawn(async move |_, cx| {
            cx.update(|cx| crate::popout::close_workspace(&k, cx));
        })
        .detach();
    }

    /// Locks or unlocks every workspace (the idle lock). Locked, each one
    /// goes lean at once and nothing wakes it; unlocked, the one looked at
    /// wakes (reopening its store) and the others start their lean timers.
    pub fn set_locked(&mut self, locked: bool, cx: &mut Context<Self>) {
        if self.locked == locked {
            return;
        }
        self.locked = locked;
        let keys: Vec<_> = self.entries.iter().map(|e| e.key.clone()).collect();
        for k in keys {
            self.refresh_timer(&k, cx);
        }
        if !locked {
            for (provider, factory) in std::mem::take(&mut self.deferred) {
                self.open_session(provider, factory, cx);
            }
        }
        cx.emit(WorkspacesEvent::Changed);
        cx.notify();
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Whether every workspace has started and loaded its store, with no
    /// reopen in flight: the idle lock waits for this, so it never closes a
    /// store half loaded.
    pub fn ready_to_lock(&self, cx: &App) -> bool {
        self.started
            && self.start_outstanding == 0
            && self.entries.iter().all(|e| {
                let s = e.session.read(cx);
                s.sync_progress().store_ready && !s.store_busy()
            })
    }

    /// The workspaces whose encrypted store works, whose keys Unlock reads
    /// again before reopening anything.
    pub fn open_store_keys(&self, cx: &App) -> Vec<WorkspaceKey> {
        if self.sqlite.is_none() {
            return Vec::new();
        }
        self.entries
            .iter()
            .filter(|e| e.session.read(cx).store_health() == crate::session::StoreHealth::Ok)
            .map(|e| e.key.clone())
            .collect()
    }

    fn is_looked_at(&self, key: &WorkspaceKey) -> bool {
        !self.locked
            && ((self.window_visible && self.active.as_ref() == Some(key))
                || self.popout_count(key) > 0)
    }

    /// Starts the one-shot lean timer for a workspace nobody looks at, or
    /// cancels it and wakes the workspace when it is looked at again.
    fn refresh_timer(&mut self, key: &WorkspaceKey, cx: &mut Context<Self>) {
        let looked_at = self.is_looked_at(key);
        let Some(i) = self.entries.iter().position(|e| &e.key == key) else {
            return;
        };
        if looked_at {
            self.entries[i].timer = None;
            let session = self.entries[i].session.clone();
            session.update(cx, |s, cx| s.set_lean(false, cx));
            return;
        }
        let session = self.entries[i].session.clone();
        if self.headless || self.locked {
            self.entries[i].timer = None;
            session.update(cx, |s, cx| s.set_lean(true, cx));
            return;
        }
        if self.entries[i].timer.is_some() || session.read(cx).is_lean() {
            return;
        }
        let after = Duration::from_secs(
            u64::from(session.read(cx).settings(cx).performance.lean_after_minutes) * 60,
        );
        let k = key.clone();
        self.entries[i].timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            this.update(cx, |w, cx| w.go_lean(&k, cx)).ok();
        }));
    }

    fn go_lean(&mut self, key: &WorkspaceKey, cx: &mut Context<Self>) {
        let Some(e) = self.entries.iter_mut().find(|e| &e.key == key) else {
            return;
        };
        e.timer = None;
        e.session.clone().update(cx, |s, cx| s.set_lean(true, cx));
        self.changed(cx);
    }

    fn bump_start_gen(&mut self, key: &WorkspaceKey) -> u64 {
        let g = self.start_gen.entry(key.clone()).or_insert(0);
        *g += 1;
        *g
    }

    /// Stops the session of `key` and forgets it (timer, running flag).
    fn stop(&mut self, key: &WorkspaceKey, cx: &mut App) {
        self.bump_start_gen(key);
        self.entries.retain(|e| &e.key != key);
        self.deferred.retain(|(p, _)| p.workspace() != key);
        self.registry.set_running(key, false);
        if let Some(n) = cx
            .try_global::<crate::notifications::Notifications>()
            .cloned()
        {
            n.forget(key, cx);
        }
    }

    fn next_active(&mut self, gone: &WorkspaceKey, cx: &mut Context<Self>) {
        if self.active.as_ref() != Some(gone) {
            return;
        }
        let next = self
            .accounts
            .iter()
            .map(|a| a.key.clone())
            .find(|k| k != gone);
        match next {
            Some(k) => self.activate(&k, cx),
            None => self.active = None,
        }
    }

    /// `workspace::RemoveWorkspace`: stops the session, then deletes the
    /// database, store key, credentials, account row and media cache. The
    /// workspace's settings file stays (user-authored). Safe to repeat after a
    /// failure part way.
    pub fn remove(
        &mut self,
        key: &WorkspaceKey,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), RemoveError>> {
        self.stop(key, cx);
        self.clear_popout_count(key);
        Self::close_popouts_later(key, cx);
        let (registry, sqlite) = (self.registry.clone(), self.sqlite.clone());
        let cache = self.dirs.as_ref().map(|d| d.cache.clone());
        let key = key.clone();
        self.accounts.retain(|a| a.key != key);
        self.next_active(&key, cx);
        self.changed(cx);
        cx.spawn(async move |this, cx| {
            let result = async {
                registry.remove_workspace_opt(&key, sqlite.as_ref()).await?;
                if let Some(db) = &sqlite {
                    // Its audit rows go too; `Closed` when `app.db` is not open.
                    let _ = db.app().audit_forget_workspace(&key).await;
                }
                if let Some(cache) = cache {
                    let k = key.clone();
                    let r = cx
                        .background_executor()
                        .spawn(async move { remove_workspace_media(&cache, &k) })
                        .await;
                    r.map_err(|e| RemoveError::Provider(ProviderError::Other(e.to_string())))?;
                }
                Ok(())
            }
            .await;
            // Also after a failure: the account may still exist and must be listed.
            this.update(cx, |w, cx| {
                w.accounts = w.registry.account_infos();
                w.changed(cx);
            })
            .ok();
            result
        })
    }

    /// `app::DeleteAllData`: removes every workspace, the app database and its
    /// key, then deletes the data and cache dirs. The config dir stays.
    pub fn delete_all(&mut self, cx: &mut Context<Self>) -> Task<Result<(), RemoveError>> {
        let keys: Vec<WorkspaceKey> = self
            .registry
            .accounts()
            .into_iter()
            .map(|(_, k)| k)
            .collect();
        for k in &keys {
            self.stop(k, cx);
        }
        self.entries.clear();
        self.active = None;
        self.clear_all_popout_counts();
        cx.spawn(async move |_, cx| {
            cx.update(|cx| crate::popout::close_all(cx, false));
        })
        .detach();
        let (registry, sqlite) = (self.registry.clone(), self.sqlite.clone());
        let dirs = self.dirs.clone();
        self.changed(cx);
        cx.spawn(async move |this, cx| {
            let result = async {
                for k in &keys {
                    registry.remove_workspace_opt(k, sqlite.as_ref()).await?;
                }
                if let Some(db) = &sqlite {
                    db.remove_app()
                        .await
                        .map_err(|e| RemoveError::Provider(ProviderError::Other(e.to_string())))?;
                }
                if let Some(dirs) = dirs {
                    let r = cx
                        .background_executor()
                        .spawn(async move {
                            remove_dir_all_checked(&dirs.data)?;
                            remove_dir_all_checked(&dirs.cache)
                        })
                        .await;
                    r.map_err(|e| RemoveError::Provider(ProviderError::Other(e.to_string())))?;
                }
                Ok(())
            }
            .await;
            // Also after a failure: accounts that survived must stay listed.
            this.update(cx, |w, cx| {
                w.accounts = w.registry.account_infos();
                w.changed(cx);
            })
            .ok();
            result
        })
    }

    fn state_path(&self) -> Option<PathBuf> {
        self.dirs.as_ref().map(|d| d.data.join(STATE_FILE))
    }

    fn read_active(&self) -> Option<WorkspaceKey> {
        let text = std::fs::read_to_string(self.state_path()?).ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let a = v.get("active")?;
        Some(WorkspaceKey::new(
            a.get("provider")?.as_str()?,
            a.get("id")?.as_str()?,
        ))
    }

    /// Remembers the active workspace across restarts (non-secret).
    fn write_active(&self, key: &WorkspaceKey, cx: &mut App) {
        let Some(path) = self.state_path() else {
            return;
        };
        let body = serde_json::json!({
            "active": { "provider": key.provider.0, "id": key.id.0 }
        })
        .to_string();
        use std::sync::atomic::Ordering;
        let writes = self.active_writes.clone();
        let ticket = writes.0.fetch_add(1, Ordering::SeqCst) + 1;
        cx.background_executor()
            .spawn(async move {
                // One write at a time; a write overtaken by a newer switch is skipped.
                let _guard = writes.1.lock().unwrap_or_else(|e| e.into_inner());
                if writes.0.load(Ordering::SeqCst) != ticket {
                    return;
                }
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, body).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            })
            .detach();
    }
}

/// Most characters of a held mention shown in a row.
const MENTION_CHARS: usize = 200;

/// Newest unread messages previewed under an "All unreads" row.
const UNREAD_PREVIEWS: usize = 3;

impl Workspaces {
    /// Name and accent of a workspace, for the rows of the cross-workspace views.
    fn tag(&self, key: &WorkspaceKey, cx: &App) -> WorkspaceTag {
        let name = self
            .accounts
            .iter()
            .find(|a| &a.key == key)
            .map(|a| a.name.to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| key.id.0.clone());
        let accent = cx.try_global::<settings::SettingsStore>().and_then(|s| {
            s.for_workspace(key)
                .accent
                .as_deref()
                .and_then(components::rail::parse_accent)
        });
        WorkspaceTag {
            key: key.clone(),
            name: name.into(),
            accent,
        }
    }

    /// Key, name, mentions and "session expired" of each started workspace in
    /// start order, for the tray menu.
    pub fn tray_rows(&self, cx: &App) -> Vec<(WorkspaceKey, SharedString, u32, bool)> {
        self.entries
            .iter()
            .map(|e| {
                let state = e.session.read(cx).state();
                (
                    e.key.clone(),
                    self.tag(&e.key, cx).name,
                    state.totals().mentions,
                    state.session == domain::Session::Expired,
                )
            })
            .collect()
    }

    /// Conversations with unread messages over every started workspace
    /// (muted ones left out), for the tray menu's header.
    pub fn tray_unread(&self, cx: &App) -> u32 {
        self.entries
            .iter()
            .map(|e| e.session.read(cx).state().totals().conversations)
            .sum()
    }

    /// Every started workspace's state, in start order.
    fn states<'a>(&'a self, cx: &'a App) -> Vec<WorkspaceRef<'a>> {
        self.entries
            .iter()
            .map(|e| WorkspaceRef {
                key: &e.key,
                state: e.session.read(cx).state(),
            })
            .collect()
    }

    fn label(&self, key: &WorkspaceKey, c: &domain::ConversationId, cx: &App) -> SharedString {
        self.session(key)
            .map(|s| s.read(cx).label_of(c, cx))
            .unwrap_or_else(|| c.0.to_string())
            .into()
    }

    /// Name and accent of each started workspace, read once per listing.
    fn tags(&self, cx: &App) -> HashMap<WorkspaceKey, WorkspaceTag> {
        self.entries
            .iter()
            .map(|e| (e.key.clone(), self.tag(&e.key, cx)))
            .collect()
    }

    fn unread_item(
        &self,
        r: domain::AggregateRow,
        tags: &HashMap<WorkspaceKey, WorkspaceTag>,
        cx: &App,
    ) -> UnreadItem {
        UnreadItem {
            workspace: tags
                .get(&r.workspace)
                .cloned()
                .unwrap_or_else(|| self.tag(&r.workspace, cx)),
            label: self.label(&r.workspace, &r.conversation, cx),
            conversation: r.conversation,
            unread: r.unread,
            mentions: r.mentions,
            latest: r.latest,
            last_read: r.last_read,
        }
    }

    /// The newest held unread messages of `c`, as preview lines.
    fn preview_lines(
        &self,
        key: &WorkspaceKey,
        c: &domain::ConversationId,
        cx: &App,
    ) -> Vec<components::PreviewLine> {
        use components::TimelineSource as _;
        let Some(session) = self.session(key) else {
            return Vec::new();
        };
        let s = session.read(cx);
        let msgs = domain::unread_previews(s.state(), c, UNREAD_PREVIEWS);
        if msgs.is_empty() {
            return Vec::new();
        }
        let cfg = s.send_later_settings(cx);
        let parser = s.content();
        let l = i18n::gpui::localizer(cx);
        msgs.into_iter()
            .map(|m| {
                let author = m.author.as_ref().and_then(|u| s.state().members.get(u));
                components::preview_line(
                    m,
                    author,
                    &*parser,
                    cfg.clock,
                    &cfg.zone,
                    &l,
                    &s.state().user_groups,
                )
            })
            .collect()
    }

    /// Marks each row's conversation read at its newest message.
    fn mark_rows(&self, rows: Vec<domain::AggregateRow>, cx: &mut App) {
        for r in rows {
            let (Some(ts), Some(session)) = (r.latest, self.session(&r.workspace)) else {
                continue;
            };
            let c = r.conversation;
            session.update(cx, |s, cx| {
                s.apply(
                    Input::MarkRead {
                        conversation: c,
                        ts,
                    },
                    cx,
                )
            });
        }
    }
}

impl AggregateSource for Workspaces {
    fn unreads(&self, cx: &App) -> Vec<UnreadItem> {
        let tags = self.tags(cx);
        all_unreads(self.states(cx))
            .into_iter()
            .map(|r| self.unread_item(r, &tags, cx))
            .collect()
    }

    fn mentions(&self, cx: &App) -> Vec<MentionItem> {
        let tags = self.tags(cx);
        all_mentions(self.states(cx))
            .into_iter()
            .map(|(r, ids)| {
                let state = self.session(&r.workspace).map(|s| s.read(cx).state());
                let messages = ids
                    .iter()
                    .filter_map(|id| {
                        let state = state?;
                        let m = state
                            .windows
                            .get(&id.conversation)?
                            .messages()
                            .iter()
                            .find(|m| m.id == *id)?;
                        let author = m
                            .author
                            .as_ref()
                            .and_then(|u| state.members.get(u))
                            .and_then(components::sidebar::best_name)
                            .map(|n| n.to_string())
                            .unwrap_or_default();
                        let text = if m.plain.is_empty() {
                            &m.text
                        } else {
                            &m.plain
                        };
                        // A label-less group token reads as its id; show the handle.
                        let text =
                            domain::resolve_group_text(text, &m.mentions, &state.user_groups);
                        Some(MentionLine {
                            id: id.clone(),
                            author: author.into(),
                            text: text.chars().take(MENTION_CHARS).collect::<String>().into(),
                        })
                    })
                    .collect();
                MentionItem {
                    row: self.unread_item(r, &tags, cx),
                    messages,
                }
            })
            .collect()
    }

    fn scheduled(&self, cx: &App) -> Vec<ScheduledItem> {
        let tags = self.tags(cx);
        all_scheduled(self.states(cx))
            .into_iter()
            .map(|r| ScheduledItem {
                workspace: tags
                    .get(&r.workspace)
                    .cloned()
                    .unwrap_or_else(|| self.tag(&r.workspace, cx)),
                conversation: self.label(&r.workspace, &r.conversation, cx),
                id: r.id,
                text: r.text.to_string().into(),
                post_at: r.post_at,
                local: r.local,
                state: r.state,
            })
            .collect()
    }

    fn mark_read(
        &mut self,
        workspace: &WorkspaceKey,
        conversation: &domain::ConversationId,
        latest: domain::MessageTs,
        cx: &mut App,
    ) {
        if let Some(session) = self.session(workspace) {
            let c = conversation.clone();
            session.update(cx, |s, cx| {
                s.apply(
                    Input::MarkRead {
                        conversation: c,
                        ts: latest,
                    },
                    cx,
                )
            });
        }
    }

    fn previews(
        &self,
        workspace: &WorkspaceKey,
        conversation: &domain::ConversationId,
        cx: &App,
    ) -> Vec<components::PreviewLine> {
        self.preview_lines(workspace, conversation, cx)
    }

    fn mark_all_read(&mut self, cx: &mut App) {
        let rows = all_unreads(self.states(cx));
        self.mark_rows(rows, cx);
    }

    fn config(&self, cx: &App) -> SendLaterConfig {
        match self.active_session() {
            Some(s) => s.read(cx).send_later_settings(cx),
            None => SendLaterConfig {
                presets: Vec::new(),
                zone: domain::schedule_zone(None, None),
                clock: settings::Clock::H24,
                server: false,
            },
        }
    }
}
