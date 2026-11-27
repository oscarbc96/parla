//! Pop-out windows: conversations and threads in their own windows, sharing
//! the same session entities as the main window.

use std::path::PathBuf;

use components::{BookmarksBar, Composer, ComposerEvent, ThreadPanel, ThreadPanelEvent, Timeline};
use domain::{ConversationId, DraftKey, MessageId};
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, Global, IntoElement, KeyDownEvent, Render,
    Subscription, Window, WindowBounds, WindowHandle, div, prelude::*, size,
};
use settings::WorkspaceKey;
use themes::ActiveTheme;

use crate::actions::{PopOutConversation, PopOutThread};
use crate::session::WorkspaceSession;
use crate::window_state::{self, PopOutTarget, SavedWindow};
use crate::workspaces::WorkspacesHandle;

/// Height of the pop-out thread window's header, which carries the traffic lights.
const POPOUT_THREAD_HEADER: gpui::Pixels = themes::metrics::LAYOUT_POPOUT_HEADER;

#[derive(Clone)]
struct Entry {
    target: PopOutTarget,
    handle: WindowHandle<PopoutView>,
}

#[derive(Default, Clone)]
pub struct PopoutRegistry {
    entries: Vec<Entry>,
    pending: Option<Vec<SavedWindow>>,
    quitting: bool,
    suppress_persist: bool,
    quit_handler_installed: bool,
    quit_handler_registrations: usize,
}

impl Global for PopoutRegistry {}

fn registry(cx: &App) -> Option<&PopoutRegistry> {
    cx.try_global::<PopoutRegistry>()
}

fn registry_mut(cx: &mut App) -> &mut PopoutRegistry {
    if !cx.has_global::<PopoutRegistry>() {
        cx.set_global(PopoutRegistry::default());
    }
    cx.global_mut::<PopoutRegistry>()
}

fn data_dir(cx: &App) -> PathBuf {
    cx.try_global::<crate::removal::AppDirs>()
        .map(|d| d.data.clone())
        .unwrap_or_else(settings::paths::data_dir)
}

fn windows_path(cx: &App) -> PathBuf {
    window_state::path_in(&data_dir(cx))
}

/// Every open pop-out with its current bounds, in registry order.
fn saved_windows(cx: &mut App) -> Vec<SavedWindow> {
    let entries = registry(cx).map(|r| r.entries.clone()).unwrap_or_default();
    entries
        .into_iter()
        .map(|e| {
            let bounds = e
                .handle
                .update(cx, |_, window, _| window_bounds(window))
                .ok()
                .flatten();
            SavedWindow {
                target: e.target,
                bounds,
            }
        })
        .collect()
}

pub fn persist_now(cx: &mut App) {
    if registry(cx).is_some_and(|r| r.suppress_persist) {
        return;
    }
    let path = windows_path(cx);
    let windows = saved_windows(cx);
    let _ = window_state::save(&path, &windows);
}

/// Persists current pop-outs once for an app quit and stops later close
/// callbacks from clearing the file while windows are torn down.
pub fn prepare_for_quit(cx: &mut App) {
    if !cx.has_global::<PopoutRegistry>() {
        return;
    }
    let suppress = cx.global::<PopoutRegistry>().suppress_persist;
    cx.global_mut::<PopoutRegistry>().quitting = true;
    if suppress {
        return;
    }
    let path = windows_path(cx);
    let windows = saved_windows(cx);
    let _ = window_state::save(&path, &windows);
}

/// The registry, created on first use, with the quit handler in place.
fn ensure_registry(cx: &mut App) {
    if !cx.has_global::<PopoutRegistry>() {
        cx.set_global(PopoutRegistry::default());
    }
    install_quit_handler(cx);
}

pub fn install_quit_handler(cx: &mut App) {
    let reg = registry_mut(cx);
    if reg.quit_handler_installed {
        return;
    }
    reg.quit_handler_installed = true;
    reg.quit_handler_registrations += 1;
    cx.on_app_quit(|cx| {
        prepare_for_quit(cx);
        // Unsent text in the pop-outs is saved before the app goes.
        let handles: Vec<_> = registry(cx)
            .map(|r| r.entries.iter().map(|e| e.handle).collect())
            .unwrap_or_default();
        let tasks: Vec<_> = handles
            .into_iter()
            .filter_map(|h| {
                h.update(cx, |view, _, cx| save_draft(view, cx))
                    .ok()
                    .flatten()
            })
            .collect();
        async move {
            futures::future::join_all(tasks).await;
        }
    })
    .detach();
}

#[doc(hidden)]
pub fn quit_handler_registrations(cx: &App) -> usize {
    registry(cx).map_or(0, |r| r.quit_handler_registrations)
}

/// Allows pop-outs to be persisted again after a delete-all removed them.
pub fn allow_persist(cx: &mut App) {
    if cx.has_global::<PopoutRegistry>() {
        let reg = cx.global_mut::<PopoutRegistry>();
        reg.suppress_persist = false;
        reg.quitting = false;
    }
}

fn window_bounds(window: &Window) -> Option<Bounds<gpui::Pixels>> {
    match window.window_bounds() {
        WindowBounds::Windowed(b) => Some(b),
        WindowBounds::Maximized(b) => Some(b),
        WindowBounds::Fullscreen(b) => Some(b),
    }
}

pub enum PopoutKind {
    Conversation {
        conversation: ConversationId,
        bookmarks: Option<Entity<BookmarksBar<WorkspaceSession>>>,
        huddle: Option<Entity<components::HuddleBar<WorkspaceSession>>>,
        timeline: Entity<Timeline<WorkspaceSession>>,
        composer: Entity<Composer<WorkspaceSession>>,
        _subs: Vec<Subscription>,
    },
    Thread {
        root: MessageId,
        panel: Entity<ThreadPanel<WorkspaceSession>>,
        _sub: Subscription,
        released: bool,
    },
}

impl crate::delete_confirm::DeleteConfirmHost for PopoutView {
    fn delete_confirm_slot(&mut self) -> &mut Option<crate::delete_confirm::DeleteConfirm> {
        &mut self.delete_confirm
    }
}

pub struct PopoutView {
    target: PopOutTarget,
    workspace: WorkspaceKey,
    session: Entity<WorkspaceSession>,
    kind: PopoutKind,
    delete_confirm: Option<crate::delete_confirm::DeleteConfirm>,
    _session_obs: Subscription,
    /// The title last handed to the window, so renders only set it on change.
    last_title: std::cell::RefCell<String>,
    /// Focus for the lock screen's Unlock while the idle lock is on.
    lock_focus: Option<gpui::FocusHandle>,
}

impl PopoutView {
    pub fn target(&self) -> &PopOutTarget {
        &self.target
    }

    pub fn session(&self) -> &Entity<WorkspaceSession> {
        &self.session
    }

    pub fn composer(&self) -> Option<Entity<Composer<WorkspaceSession>>> {
        match &self.kind {
            PopoutKind::Conversation { composer, .. } => Some(composer.clone()),
            PopoutKind::Thread { .. } => None,
        }
    }

    #[doc(hidden)]
    /// The open delete confirmation, if any.
    pub fn delete_confirm(&self) -> Option<&crate::delete_confirm::DeleteConfirm> {
        self.delete_confirm.as_ref()
    }

    pub fn timeline(&self) -> Option<Entity<Timeline<WorkspaceSession>>> {
        match &self.kind {
            PopoutKind::Conversation { timeline, .. } => Some(timeline.clone()),
            PopoutKind::Thread { .. } => None,
        }
    }

    #[doc(hidden)]
    pub fn thread_panel(&self) -> Option<Entity<ThreadPanel<WorkspaceSession>>> {
        match &self.kind {
            PopoutKind::Thread { panel, .. } => Some(panel.clone()),
            PopoutKind::Conversation { .. } => None,
        }
    }

    fn release_thread(&mut self, cx: &mut App) {
        if let PopoutKind::Thread { root, released, .. } = &mut self.kind
            && !*released
        {
            *released = true;
            let root = root.clone();
            self.session.update(cx, |s, cx| s.close_thread(root, cx));
        }
    }

    pub fn send(&mut self, cx: &mut Context<Self>) {
        if let PopoutKind::Conversation {
            composer,
            conversation,
            ..
        } = &self.kind
        {
            let body = composer.read(cx).composed(cx);
            let (session, c) = (self.session.clone(), conversation.clone());
            crate::root_view::chat_pane::handle_composer_event(
                &session,
                &c,
                &ComposerEvent::Submit { body },
                cx,
            );
        }
    }

    pub fn title(&self, cx: &App) -> String {
        let conv_id = match &self.target {
            PopOutTarget::Conversation { id, .. } => id.clone(),
            PopOutTarget::Thread { conversation, .. } => conversation.clone(),
        };
        let conv_name = self
            .session
            .read(cx)
            .state()
            .conversations
            .get(&conv_id)
            .map(|c| c.name.to_string())
            .unwrap_or_else(|| conv_id.0.to_string());
        let ws_name = cx
            .try_global::<WorkspacesHandle>()
            .and_then(|h| {
                h.0.read(cx)
                    .accounts()
                    .iter()
                    .find(|a| a.key == self.workspace)
                    .map(|a| a.name.to_string())
            })
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.workspace.id.0.clone());
        t!(
            cx,
            "shell-pop-out-title",
            conversation = conv_name,
            workspace = ws_name
        )
        .into_owned()
    }
}

/// The thread window's header: drag area with the traffic lights, the
/// "Thread" title and the conversation and workspace names below it.
pub fn thread_header(subtitle: &str, cx: &App) -> gpui::Div {
    use themes::metrics::{
        FONT_SIZE_LG, FONT_SIZE_XS, FONT_WEIGHT_BOLD, LAYOUT_TITLEBAR_LIGHTS, SPACE_1, SPACE_6,
    };
    let colors = *cx.theme().colors();
    let tokens = themes::parla_tokens(cx.theme());
    let left = if cfg!(target_os = "macos") {
        LAYOUT_TITLEBAR_LIGHTS
    } else {
        SPACE_6
    };
    crate::titlebar::draggable(
        div()
            .flex()
            .flex_none()
            .flex_col()
            .justify_center()
            .h(POPOUT_THREAD_HEADER)
            .pl(left)
            .pr(SPACE_6)
            .gap(SPACE_1)
            .bg(colors.surface_background)
            .border_b_1()
            .border_color(colors.border),
    )
    .child(
        div()
            .text_size(FONT_SIZE_LG)
            .font_weight(FONT_WEIGHT_BOLD)
            .text_color(colors.text)
            .child(
                i18n::gpui::localizer(cx)
                    .format("components", "thread-title", None)
                    .into_owned(),
            ),
    )
    .child(
        div()
            .text_size(FONT_SIZE_XS)
            .text_color(tokens.text_subtle)
            .truncate()
            .child(subtitle.to_string()),
    )
}

impl Render for PopoutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        if crate::idle_lock::is_locked(cx) {
            let focus = self
                .lock_focus
                .get_or_insert_with(|| cx.focus_handle())
                .clone();
            return crate::idle_lock::cover(&focus, window, cx);
        }
        // keep window title in sync when names change
        let title = self.title(cx);
        if *self.last_title.borrow() != title {
            window.set_window_title(&title);
            *self.last_title.borrow_mut() = title.clone();
        }
        let body = match &self.kind {
            PopoutKind::Conversation {
                bookmarks,
                huddle,
                timeline,
                composer,
                ..
            } => {
                let mut col = div().flex().flex_col().flex_1().min_w_0().h_full();
                if let Some(h) = huddle {
                    col = col.child(h.clone());
                }
                if let Some(b) = bookmarks {
                    col = col.child(b.clone());
                }
                col.child(div().flex_1().min_h_0().child(timeline.clone()))
                    .child(composer.clone())
                    .into_any_element()
            }
            PopoutKind::Thread { panel, .. } => div()
                .flex_1()
                .min_h_0()
                .child(panel.clone())
                .into_any_element(),
        };
        let content = match &self.kind {
            PopoutKind::Conversation { .. } => {
                crate::titlebar::column(div().size_full().child(body)).into_any_element()
            }
            PopoutKind::Thread { .. } => div()
                .flex()
                .flex_col()
                .size_full()
                .child(thread_header(&title, cx))
                .child(body)
                .into_any_element(),
        };
        let (ui_font, ui_size) = components::live_settings::ui_font(cx);
        let mut root = crate::idle_lock::IdleInput::idle_input(div())
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .font_family(ui_font)
            .text_size(ui_size)
            .relative()
            .child(content)
            // Toasts pushed while this window is the active one show here.
            .child(components::Toasts::overlay(window, cx))
            .children(
                self.delete_confirm
                    .as_ref()
                    .map(|p| crate::delete_confirm::render_scrim(p, cx)),
            )
            .on_action(cx.listener(|this, _: &PopOutConversation, _, cx| {
                if let PopoutKind::Conversation { conversation, .. } = &this.kind {
                    let target =
                        PopOutTarget::conversation(this.workspace.clone(), conversation.clone());
                    open(target, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PopOutThread, _, cx| {
                if let PopoutKind::Thread { root, .. } = &this.kind {
                    let target = PopOutTarget::thread(
                        this.workspace.clone(),
                        root.conversation.clone(),
                        root.ts.clone(),
                    );
                    open(target, cx);
                }
            }));
        if matches!(self.kind, PopoutKind::Thread { .. }) {
            root = root.on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    this.release_thread(cx);
                    window.remove_window();
                    cx.stop_propagation();
                }
            }));
        }
        root
    }
}

fn workspace_session(cx: &App, ws: &WorkspaceKey) -> Option<Entity<WorkspaceSession>> {
    cx.try_global::<WorkspacesHandle>()?.0.read(cx).session(ws)
}

pub fn open(target: PopOutTarget, cx: &mut App) -> Option<WindowHandle<PopoutView>> {
    ensure_registry(cx);
    if let Some(handle) = registry(cx).and_then(|r| {
        r.entries
            .iter()
            .find(|e| e.target == target)
            .map(|e| e.handle)
    }) {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
        cx.activate(true);
        return Some(handle);
    }
    open_inner(target, None, cx)
}

fn open_inner(
    target: PopOutTarget,
    bounds: Option<Bounds<gpui::Pixels>>,
    cx: &mut App,
) -> Option<WindowHandle<PopoutView>> {
    let ws = target.workspace().clone();
    let session = workspace_session(cx, &ws)?;
    let bounds = bounds.unwrap_or_else(|| {
        Bounds::centered(
            None,
            size(
                themes::metrics::LAYOUT_WINDOW_POPOUT_WIDTH,
                themes::metrics::LAYOUT_WINDOW_POPOUT_HEIGHT,
            ),
            cx,
        )
    });
    let t = target.clone();
    let wsk = ws.clone();
    let sess = session.clone();
    let opened = cx
        .open_window(
            crate::window_options::popout(bounds, matches!(target, PopOutTarget::Thread { .. })),
            move |window, cx| {
                let view = build_view(t.clone(), wsk.clone(), sess.clone(), window, cx);
                let title = view.read(cx).title(cx);
                window.set_window_title(&title);
                view
            },
        )
        .ok()?;

    registry_mut(cx).entries.push(Entry {
        target: target.clone(),
        handle: opened,
    });
    if let Some(wh) = cx.try_global::<WorkspacesHandle>().map(|h| h.0.clone()) {
        wh.update(cx, |w, cx| w.popout_opened(&ws, cx));
    }
    persist_now(cx);

    // Track close via view release.
    if let Ok(entity) = opened.entity(cx) {
        let target_for_close = target.clone();
        let ws_for_close = ws.clone();
        cx.observe_release(&entity, move |view, cx| {
            if let Some(task) = save_draft(view, cx) {
                task.detach();
            }
            view.release_thread(cx);
            let quitting = cx
                .try_global::<PopoutRegistry>()
                .is_some_and(|r| r.quitting);
            if quitting {
                return;
            }
            if cx.has_global::<PopoutRegistry>() {
                let reg = cx.global_mut::<PopoutRegistry>();
                if let Some(i) = reg
                    .entries
                    .iter()
                    .position(|e| e.target == target_for_close)
                {
                    reg.entries.remove(i);
                } else {
                    return;
                }
            }
            if let Some(wh) = cx.try_global::<WorkspacesHandle>().map(|h| h.0.clone()) {
                wh.update(cx, |w, cx| w.popout_closed(&ws_for_close, cx));
            }
            persist_now(cx);
        })
        .detach();
    }
    Some(opened)
}

/// Saves the composer text of a pop-out as its draft (blank deletes); the task
/// resolves once the store has it. `None` while an edit is in the composer.
fn save_draft(view: &PopoutView, cx: &mut App) -> Option<gpui::Task<()>> {
    let (key, text) = match &view.kind {
        PopoutKind::Conversation {
            conversation,
            composer,
            ..
        } => {
            let c = composer.read(cx);
            if c.editing().is_some() {
                return None;
            }
            let key = DraftKey {
                conversation: conversation.clone(),
                thread: None,
            };
            (key, c.composed(cx))
        }
        PopoutKind::Thread { root, panel, .. } => (
            crate::root_view::thread_key(root),
            panel.read(cx).composer().read(cx).composed(cx),
        ),
    };
    Some(view.session.update(cx, |s, cx| s.save_draft(key, text, cx)))
}

fn build_view(
    target: PopOutTarget,
    workspace: WorkspaceKey,
    session: Entity<WorkspaceSession>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<PopoutView> {
    cx.new(|cx| {
        let session_obs = cx.observe(&session, |_this: &mut PopoutView, _, cx| cx.notify());
        // The idle lock covers this window too.
        if let Some(h) = cx.try_global::<crate::idle_lock::IdleLockHandle>() {
            cx.observe(&h.0.clone(), |_: &mut PopoutView, _, cx| cx.notify())
                .detach();
        }
        let kind = match &target {
            PopOutTarget::Conversation { id, .. } => {
                let conversation = id.clone();
                session.update(cx, |s, cx| s.open_unfocused(conversation.clone(), cx));
                let pane = crate::root_view::chat_pane::ChatPane::new(
                    session.clone(),
                    conversation,
                    window,
                    cx,
                    None,
                    |this, root, _, cx| {
                        open(
                            PopOutTarget::thread(
                                this.workspace.clone(),
                                root.conversation.clone(),
                                root.ts.clone(),
                            ),
                            cx,
                        );
                    },
                );
                PopoutKind::Conversation {
                    conversation: pane.conversation,
                    bookmarks: pane.bookmarks,
                    huddle: pane.huddle,
                    timeline: pane.timeline,
                    composer: pane.composer,
                    _subs: pane.subs,
                }
            }
            PopOutTarget::Thread {
                conversation, root, ..
            } => {
                let root_id = MessageId {
                    conversation: conversation.clone(),
                    ts: root.clone(),
                };
                let clock = cx
                    .global::<settings::SettingsStore>()
                    .current()
                    .messages
                    .clock;
                let panel = cx.new(|cx| {
                    let mut p = ThreadPanel::new(session.clone(), root_id.clone(), clock, cx);
                    // The window header replaces the panel's own.
                    p.set_show_header(false);
                    p
                });
                let thread_composer = panel.read(cx).composer().clone();
                crate::root_view::restore_draft_for(
                    &session,
                    &thread_composer,
                    &crate::root_view::thread_key(&root_id),
                    cx,
                );
                let sub = cx.subscribe_in(
                    &panel,
                    window,
                    move |this, _, ev: &ThreadPanelEvent, window, cx| match ev {
                        ThreadPanelEvent::Close(_) => {
                            this.release_thread(cx);
                            window.remove_window();
                        }
                        ThreadPanelEvent::PopOut(root) => {
                            let target = PopOutTarget::thread(
                                this.workspace.clone(),
                                root.conversation.clone(),
                                root.ts.clone(),
                            );
                            open(target, cx);
                        }
                    },
                );
                PopoutKind::Thread {
                    root: root_id,
                    panel,
                    _sub: sub,
                    released: false,
                }
            }
        };
        PopoutView {
            target,
            workspace,
            session,
            kind,
            delete_confirm: None,
            _session_obs: session_obs,
            last_title: Default::default(),
            lock_focus: None,
        }
    })
}

/// Loads saved pop-outs and opens those whose workspace is available now.
/// Targets whose workspace is gone from the saved account set are dropped.
/// Returns how many windows were opened in this pass.
pub fn restore(path: &std::path::Path, cx: &mut App) -> usize {
    // `windows.restore` off: windows start fresh, like the main window.
    if cx
        .try_global::<settings::SettingsStore>()
        .is_some_and(|s| !s.current().windows.restore)
    {
        return 0;
    }
    ensure_registry(cx);
    if registry(cx).and_then(|r| r.pending.as_ref()).is_none() {
        cx.global_mut::<PopoutRegistry>().pending = Some(window_state::load(path));
    }
    restore_pending(cx)
}

/// Try opening any pending saved windows whose sessions are now available.
pub fn restore_pending(cx: &mut App) -> usize {
    if !cx.has_global::<PopoutRegistry>() {
        return 0;
    }
    ensure_registry(cx);
    let Some(pending) = registry(cx).and_then(|r| r.pending.clone()) else {
        return 0;
    };
    let known: Option<std::collections::HashSet<WorkspaceKey>> =
        cx.try_global::<WorkspacesHandle>().map(|h| {
            h.0.read(cx)
                .accounts()
                .iter()
                .map(|a| a.key.clone())
                .collect()
        });
    let mut remaining = Vec::new();
    let mut opened = 0;
    for w in pending {
        let ws = w.target.workspace().clone();
        // If we know the account set and this workspace is gone, skip permanently.
        if let Some(known) = &known
            && !known.contains(&ws)
        {
            continue;
        }
        if workspace_session(cx, &ws).is_none() {
            remaining.push(w);
            continue;
        }
        if find(&w.target, cx).is_some() || open_inner(w.target, w.bounds, cx).is_some() {
            opened += 1;
        }
    }
    cx.global_mut::<PopoutRegistry>().pending = if remaining.is_empty() {
        None
    } else {
        Some(remaining)
    };
    opened
}

/// Restore from default data dir after main window.
pub fn restore_default(cx: &mut App) -> usize {
    restore(&windows_path(cx), cx)
}

/// The open pop-out handle for `target`, if any.
pub fn find(target: &PopOutTarget, cx: &App) -> Option<WindowHandle<PopoutView>> {
    registry(cx).and_then(|r| {
        r.entries
            .iter()
            .find(|e| &e.target == target)
            .map(|e| e.handle)
    })
}

/// The composers of the pop-outs of workspace `ws` that hold the draft `key`
/// (a conversation's, or a thread's).
pub fn composers_for(
    ws: &WorkspaceKey,
    key: &domain::DraftKey,
    cx: &App,
) -> Vec<Entity<Composer<WorkspaceSession>>> {
    let Some(reg) = registry(cx) else {
        return Vec::new();
    };
    reg.entries
        .iter()
        .filter_map(|e| {
            let hit = match (&e.target, &key.thread) {
                (PopOutTarget::Conversation { ws: w, id }, None) => {
                    w == ws && *id == key.conversation
                }
                (
                    PopOutTarget::Thread {
                        ws: w,
                        conversation,
                        root,
                    },
                    Some(t),
                ) => w == ws && *conversation == key.conversation && root == t,
                _ => false,
            };
            if !hit {
                return None;
            }
            let view = e.handle.read(cx).ok()?;
            match &view.kind {
                PopoutKind::Conversation { composer, .. } => Some(composer.clone()),
                PopoutKind::Thread { panel, .. } => Some(panel.read(cx).composer().clone()),
            }
        })
        .collect()
}

pub fn open_conversation(
    ws: WorkspaceKey,
    id: ConversationId,
    cx: &mut App,
) -> Option<WindowHandle<PopoutView>> {
    open(PopOutTarget::conversation(ws, id), cx)
}

pub fn open_thread(
    ws: WorkspaceKey,
    conversation: ConversationId,
    root: domain::MessageTs,
    cx: &mut App,
) -> Option<WindowHandle<PopoutView>> {
    open(PopOutTarget::thread(ws, conversation, root), cx)
}

/// Closes all pop-outs for `ws`, removing them from the registry and persisted
/// state. Safe to call from `App` context.
pub fn close_workspace(ws: &WorkspaceKey, cx: &mut App) {
    let Some(reg) = cx.try_global::<PopoutRegistry>().cloned() else {
        return;
    };
    let mut keep = Vec::new();
    let mut closing = Vec::new();
    for e in reg.entries {
        if e.target.workspace() == ws {
            closing.push(e);
        } else {
            keep.push(e);
        }
    }
    cx.global_mut::<PopoutRegistry>().entries = keep;
    if let Some(wh) = cx.try_global::<WorkspacesHandle>().map(|h| h.0.clone()) {
        wh.update(cx, |w, _| w.clear_popout_count(ws));
    }
    for e in closing {
        let _ = e.handle.update(cx, |_, window, _| window.remove_window());
    }
    persist_now(cx);
}

/// Closes every pop-out. When `persist` is false, no `windows.json` write
/// happens afterwards (used by delete-all before removing the data dir).
pub fn close_all(cx: &mut App, persist: bool) {
    if !cx.has_global::<PopoutRegistry>() {
        return;
    }
    let entries = std::mem::take(&mut cx.global_mut::<PopoutRegistry>().entries);
    let reg = cx.global_mut::<PopoutRegistry>();
    reg.pending = None;
    if !persist {
        reg.suppress_persist = true;
        reg.quitting = false;
    }
    if let Some(wh) = cx.try_global::<WorkspacesHandle>().map(|h| h.0.clone()) {
        wh.update(cx, |w, _| w.clear_all_popout_counts());
    }
    for e in entries {
        let _ = e.handle.update(cx, |_, window, _| window.remove_window());
    }
    if persist {
        persist_now(cx);
    }
}
