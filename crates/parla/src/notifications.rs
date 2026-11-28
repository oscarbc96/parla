//! Native notifications and the dock badge.
//!
//! [`Notifications`] is a GPUI global shared by every workspace session. It
//! owns the [`Notifier`] (real or fake), remembers the notification delivered
//! per conversation (for replacement, dismissal and click-to-open) and sums
//! mentions over workspaces for the badge.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use domain::notify::{Notification, SessionExpiredNotice};
use domain::{ConversationId, MessageId, Ring};
use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Entity, Global, SystemNotification, SystemNotificationAction, WeakEntity};
use settings::WorkspaceKey;

use crate::platform::NotificationPermission;
use crate::session::WorkspaceSession;
use crate::toast::ConfigBanner;

/// Where notifications and the badge go. `cx` is the GPUI app so the real
/// implementation can call the platform.
pub trait Notifier {
    fn show(&self, n: &Notification, cx: &mut App);
    fn dismiss_conversation(&self, ws: &WorkspaceKey, c: &ConversationId, cx: &mut App);
    fn set_badge(&self, count: Option<u32>, cx: &mut App);
    /// Shows the incoming-huddle notification with Join and Decline. The
    /// default shows nothing (the in-app banner still does).
    fn ring(&self, _ws: &WorkspaceKey, _r: &Ring, _caller: &str, _cx: &mut App) {}
    /// Removes the incoming-huddle notification of `c`.
    fn stop_ring(&self, _ws: &WorkspaceKey, _c: &ConversationId, _cx: &mut App) {}
    /// Shows an account-problem notification for an expired session. The
    /// default shows nothing (the in-app banner still does).
    fn show_session_expired(&self, _n: &SessionExpiredNotice, _cx: &mut App) {}
    /// Removes a delivered notification by tag (used when a workspace is
    /// signed out or removed). No-op unless the notifier tracks tags.
    fn dismiss_tag(&self, _tag: &str, _cx: &mut App) {}
}

/// Plays the alert sound of a delivered notification. GPUI's system
/// notifications carry no sound, so the app plays it itself, once per
/// notification that is not silent.
pub trait NotificationSound {
    fn play(&self);
}

/// The platform's alert sound (a system sound on macOS, nothing elsewhere).
pub struct SystemSound;

impl NotificationSound for SystemSound {
    fn play(&self) {
        crate::platform::play_notification_sound();
    }
}

/// Plays nothing; the default until [`Notifications::with_sound`].
struct NoSound;

impl NotificationSound for NoSound {
    fn play(&self) {}
}

/// The system side of notification permission: the request/grant check and
/// the System Settings pane. Faked in tests.
pub trait PermissionChecker {
    fn request(&self, tx: UnboundedSender<NotificationPermission>);
    fn open_settings(&self);
}

/// The real checker: UserNotifications on macOS, a no-op elsewhere.
pub struct SystemPermissionChecker;

impl PermissionChecker for SystemPermissionChecker {
    fn request(&self, tx: UnboundedSender<NotificationPermission>) {
        crate::platform::request_notification_permission(tx);
    }

    fn open_settings(&self) {
        crate::platform::open_notification_settings();
    }
}

/// The notice key of the "notifications are off" banner.
pub const PERMISSION_NOTICE: &str = "notifications";

/// Id of the incoming-huddle notification of a conversation:
/// `<provider>/<workspace>/<conversation>/ring`.
pub fn ring_tag(ws: &WorkspaceKey, c: &ConversationId) -> String {
    format!("{ws}/{}/ring", c.0)
}

/// Action ids of the ring notification's buttons.
pub const RING_JOIN: &str = "join";
pub const RING_DECLINE: &str = "decline";

/// Action ids of a message notification's buttons.
pub const ACTION_REPLY: &str = "reply";
pub const ACTION_MARK_READ: &str = "mark-read";

/// Prefix of the message ts of a missed-huddle notification (not a message).
pub(crate) const MISSED_TS_PREFIX: &str = "missed-";

/// The buttons of a message notification: Reply and Mark as read. Reminders
/// and missed huddles are not messages to answer, so they carry none.
pub fn actions(n: &Notification, l: &i18n::Localizer) -> Vec<SystemNotificationAction> {
    if n.conversation
        .0
        .starts_with(crate::reminders::REMINDER_PREFIX)
        || n.message.ts.0.starts_with(MISSED_TS_PREFIX)
    {
        return Vec::new();
    }
    vec![
        SystemNotificationAction {
            id: ACTION_REPLY.into(),
            label: t!(l, "notification-action-reply").into_owned().into(),
        },
        SystemNotificationAction {
            id: ACTION_MARK_READ.into(),
            label: t!(l, "notification-action-mark-read").into_owned().into(),
        },
    ]
}

/// The notification title in the active language: the author, plus the
/// channel for a channel message. Names are user content and never translated.
pub fn title(n: &Notification, l: &i18n::Localizer) -> String {
    match &n.channel {
        Some(channel) => t!(
            l,
            "notification-title-channel",
            author = &*n.author,
            channel = &**channel
        )
        .into_owned(),
        None => n.author.to_string(),
    }
}

/// The notification body: the message text, or the number of files when the
/// message carries only files.
pub fn body(n: &Notification, l: &i18n::Localizer) -> String {
    if n.text.is_empty() && n.file_count == 0 {
        String::new()
    } else if n.text.is_empty() {
        t!(l, "notification-body-files", count = n.file_count).into_owned()
    } else {
        n.text.to_string()
    }
}

/// `<provider>/<workspace>/<conversation>/<ts>`.
pub fn tag(n: &Notification) -> String {
    format!("{}/{}/{}", n.workspace, n.conversation.0, n.message.ts.0)
}

/// Tag of a workspace's expired-session notification. Distinct from message
/// tags so a click is routed to sign-in, not to a conversation.
pub fn session_expired_tag(ws: &WorkspaceKey) -> String {
    format!("{ws}/session-expired")
}

/// Localised title of an expired-session notification. The workspace name is
/// user content and is never translated.
pub fn session_expired_title(n: &SessionExpiredNotice, l: &i18n::Localizer) -> String {
    t!(l, "session-expired-title", workspace = &*n.workspace_name).into_owned()
}

/// Localised body of an expired-session notification.
pub fn session_expired_body(l: &i18n::Localizer) -> String {
    t!(l, "session-expired-body").into_owned()
}

type ConvKey = (WorkspaceKey, ConversationId);

/// Delivers through GPUI's system notifications; at most one delivered tag
/// per conversation, a newer one replaces (dismisses) the older.
#[derive(Default)]
pub struct GpuiNotifier {
    tags: RefCell<HashMap<ConvKey, String>>,
}

impl GpuiNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `new` as the conversation's tag; returns the one it replaces.
    fn replace_tag(&self, key: ConvKey, new: String) -> Option<String> {
        self.tags.borrow_mut().insert(key, new)
    }
}

impl Notifier for GpuiNotifier {
    fn show(&self, n: &Notification, cx: &mut App) {
        let new = tag(n);
        let old = self.replace_tag((n.workspace.clone(), n.conversation.clone()), new.clone());
        if let Some(old) = old.filter(|o| *o != new) {
            // Dropping the task does not cancel the dismissal.
            drop(cx.dismiss_system_notification(&old));
        }
        let l = i18n::gpui::localizer(cx);
        crate::log::info("notification handed to the system (content not logged)");
        cx.show_system_notification(SystemNotification {
            tag: new.into(),
            title: title(n, &l).into(),
            body: body(n, &l).into(),
            actions: actions(n, &l),
        });
    }

    fn dismiss_conversation(&self, ws: &WorkspaceKey, c: &ConversationId, cx: &mut App) {
        if let Some(t) = self.tags.borrow_mut().remove(&(ws.clone(), c.clone())) {
            drop(cx.dismiss_system_notification(&t));
        }
    }

    fn set_badge(&self, count: Option<u32>, _cx: &mut App) {
        crate::platform::set_badge(count);
    }

    fn ring(&self, ws: &WorkspaceKey, r: &Ring, caller: &str, cx: &mut App) {
        let l = i18n::gpui::localizer(cx);
        cx.show_system_notification(SystemNotification {
            tag: ring_tag(ws, &r.conversation).into(),
            title: t!(l, "ring-title", caller = caller).into_owned().into(),
            body: t!(l, "ring-body").into_owned().into(),
            actions: vec![
                SystemNotificationAction {
                    id: RING_JOIN.into(),
                    label: t!(l, "ring-join").into_owned().into(),
                },
                SystemNotificationAction {
                    id: RING_DECLINE.into(),
                    label: t!(l, "ring-decline").into_owned().into(),
                },
            ],
        });
    }

    fn stop_ring(&self, ws: &WorkspaceKey, c: &ConversationId, cx: &mut App) {
        drop(cx.dismiss_system_notification(&ring_tag(ws, c)));
    }

    fn show_session_expired(&self, n: &SessionExpiredNotice, cx: &mut App) {
        let l = i18n::gpui::localizer(cx);
        crate::log::info("session-expired notification handed to the system");
        cx.show_system_notification(SystemNotification {
            tag: session_expired_tag(&n.workspace).into(),
            title: session_expired_title(n, &l).into(),
            body: session_expired_body(&l).into(),
            actions: Vec::new(),
        });
    }

    fn dismiss_tag(&self, tag: &str, cx: &mut App) {
        drop(cx.dismiss_system_notification(tag));
    }
}

/// Used when notifications cannot work (not running from the app bundle).
#[derive(Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn show(&self, _: &Notification, _: &mut App) {}
    fn dismiss_conversation(&self, _: &WorkspaceKey, _: &ConversationId, _: &mut App) {}
    fn set_badge(&self, _: Option<u32>, _: &mut App) {}
}

pub const NEEDS_BUNDLE: &str =
    "notifications are off: not running from an app bundle (use mise run run, or mise run bundle)";

/// Whether system notifications can work for this process: on macOS only from
/// an app bundle with an identifier. (`App::app_path` is no test: it succeeds
/// for a bare binary too, which once left this always true.)
pub fn can_notify() -> bool {
    crate::platform::is_bundled()
}

/// The notifier for this process: GPUI's when bundled (or off macOS), else a
/// no-op that logs [`NEEDS_BUNDLE`] once.
pub fn notifier_for(bundled: bool) -> Rc<dyn Notifier> {
    if bundled {
        Rc::new(GpuiNotifier::new())
    } else {
        crate::log::stderr_line(NEEDS_BUNDLE);
        crate::log::warn(NEEDS_BUNDLE);
        Rc::new(NoopNotifier)
    }
}

/// Puts a permission result into the config banner: Denied shows the notice,
/// Allowed or Undetermined clears it. Outside an app bundle nothing is done.
fn apply_permission_to(
    banner: &mut ConfigBanner,
    permission: NotificationPermission,
    bundled: bool,
    text: &str,
) {
    if !bundled {
        return;
    }
    let notice = (permission == NotificationPermission::Denied).then(|| text.to_string());
    banner.set_notice(PERMISSION_NOTICE, notice);
}

/// Records and logs a permission answer (the log line carries no content).
fn apply_permission(permission: NotificationPermission, bundled: bool, cx: &mut App) {
    if permission == NotificationPermission::Denied {
        crate::log::warn(&format!("notification permission: {}", permission.as_str()));
    } else {
        crate::log::info(&format!("notification permission: {}", permission.as_str()));
    }
    let l = i18n::gpui::localizer(cx);
    let text = t!(l, "notifications-permission-denied").into_owned();
    apply_permission_to(cx.global_mut::<ConfigBanner>(), permission, bundled, &text);
}

/// Asks the system for permission now (so the prompt appears while the user
/// looks at Parla) and applies the answer. No-op outside an app bundle.
fn request_permission_with(hub: &Notifications, bundled: bool, cx: &mut App) {
    if !bundled {
        return;
    }
    let permissions = hub.0.permissions.borrow().clone();
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    permissions.request(tx);
    cx.spawn(async move |cx| {
        use futures::StreamExt as _;
        while let Some(permission) = rx.next().await {
            cx.update(|cx| apply_permission(permission, bundled, cx));
        }
    })
    .detach();
}

/// Startup check: ask for notification permission right after the notifier is
/// installed. macOS asks lazily inside the first notification otherwise, and a
/// missed prompt leaves notifications silently off.
pub fn request_permission(cx: &mut App) {
    let Some(hub) = cx.try_global::<Notifications>().cloned() else {
        return;
    };
    request_permission_with(&hub, can_notify(), cx);
}

/// Re-checks the permission whenever the main window becomes active, so the
/// notice clears once the user turns notifications on in System Settings.
pub fn watch_activation<T: 'static>(window: &mut gpui::Window, cx: &mut gpui::Context<T>) {
    if !can_notify() {
        return;
    }
    cx.observe_window_activation(window, |_, window, cx| {
        if window.is_window_active() {
            request_permission(cx);
        }
    })
    .detach();
}

/// Opens the sign-in window after an expired-session notification is clicked.
/// The shell installs one that routes through the main window's "Sign in
/// again" path; tests install a recorder.
pub type SignInOpener = Rc<dyn Fn(&WorkspaceKey, &mut App) + 'static>;

struct Inner {
    notifier: Rc<dyn Notifier>,
    sound: RefCell<Rc<dyn NotificationSound>>,
    permissions: RefCell<Rc<dyn PermissionChecker>>,
    delivered: RefCell<HashMap<ConvKey, Notification>>,
    mentions: RefCell<HashMap<WorkspaceKey, u32>>,
    badge: RefCell<Option<Option<u32>>>,
    sessions: RefCell<HashMap<WorkspaceKey, WeakEntity<WorkspaceSession>>>,
    /// Workspaces already reported as expired. A workspace is removed here on
    /// a signed-in event, so a later expiry can notify again.
    expired: RefCell<HashSet<WorkspaceKey>>,
    sign_in_opener: RefCell<Option<SignInOpener>>,
}

/// Shared handle; cheap to clone out of the global.
#[derive(Clone)]
pub struct Notifications(Rc<Inner>);

impl Global for Notifications {}

impl Notifications {
    pub fn new(notifier: Rc<dyn Notifier>) -> Self {
        Self(Rc::new(Inner {
            notifier,
            sound: RefCell::new(Rc::new(NoSound)),
            permissions: RefCell::new(Rc::new(SystemPermissionChecker)),
            delivered: RefCell::default(),
            mentions: RefCell::default(),
            badge: RefCell::default(),
            sessions: RefCell::default(),
            expired: RefCell::default(),
            sign_in_opener: RefCell::default(),
        }))
    }

    /// Plays `sound` for every shown notification that is not silent.
    pub fn with_sound(self, sound: Rc<dyn NotificationSound>) -> Self {
        *self.0.sound.borrow_mut() = sound;
        self
    }

    /// Replaces the platform permission checker (tests use a fake).
    pub fn with_permissions(self, permissions: Rc<dyn PermissionChecker>) -> Self {
        *self.0.permissions.borrow_mut() = permissions;
        self
    }

    /// Makes this the app's notifications and routes clicks to
    /// [`Notifications::handle_response`].
    pub fn install(self, cx: &mut App) {
        cx.on_system_notification_response(|r, cx| {
            if crate::ringer::Ringer::handle_response(&r, cx) {
                return;
            }
            // GPUI's response carries no typed text yet, so a Reply press
            // arrives without it and only opens the conversation.
            Notifications::handle_action(&r.tag, r.action_id.as_deref(), None, cx);
        });
        cx.set_global(self);
    }

    pub(crate) fn register(&self, ws: WorkspaceKey, session: WeakEntity<WorkspaceSession>) {
        self.0.sessions.borrow_mut().insert(ws, session);
    }

    /// The session of workspace `ws`, if it is running.
    pub(crate) fn session(&self, ws: &WorkspaceKey) -> Option<Entity<WorkspaceSession>> {
        self.0.sessions.borrow().get(ws).and_then(|s| s.upgrade())
    }

    /// Shows the incoming-huddle notification.
    pub fn ring(&self, ws: &WorkspaceKey, r: &Ring, caller: &str, cx: &mut App) {
        self.0.notifier.ring(ws, r, caller, cx);
    }

    /// Removes the incoming-huddle notification.
    pub fn stop_ring(&self, ws: &WorkspaceKey, c: &ConversationId, cx: &mut App) {
        self.0.notifier.stop_ring(ws, c, cx);
    }

    /// Shows `n`, replacing the conversation's previous notification.
    pub fn show(&self, n: Notification, cx: &mut App) {
        self.0.notifier.show(&n, cx);
        if !n.silent {
            let sound = self.0.sound.borrow().clone();
            sound.play();
        }
        self.0
            .delivered
            .borrow_mut()
            .insert((n.workspace.clone(), n.conversation.clone()), n);
    }

    /// Installs the action for a click on an expired-session notification.
    pub fn set_sign_in_opener(&self, opener: SignInOpener) {
        *self.0.sign_in_opener.borrow_mut() = Some(opener);
    }

    /// Builds the notice for `ws`: `name` when not empty, else the account
    /// row's name, else the workspace id (a bootstrap that found the session
    /// already expired never learned the name).
    pub fn expired_notice(ws: &WorkspaceKey, name: &str, cx: &App) -> SessionExpiredNotice {
        let workspace_name: Arc<str> = if !name.is_empty() {
            Arc::from(name)
        } else {
            cx.try_global::<crate::workspaces::WorkspacesHandle>()
                .and_then(|h| {
                    h.0.read(cx)
                        .accounts()
                        .iter()
                        .find(|a| &a.key == ws)
                        .map(|a| a.name.clone())
                })
                .unwrap_or_else(|| Arc::from(ws.id.0.as_str()))
        };
        SessionExpiredNotice {
            workspace: ws.clone(),
            workspace_name,
        }
    }

    /// Reports an expired session once per workspace. DND and quiet hours do
    /// not apply: this is an account problem, not a message, and it is the
    /// only way the user learns the workspace needs them. Repeated provider
    /// events for the same expiry are dropped until [`Self::signed_in`].
    pub fn session_expired(&self, n: SessionExpiredNotice, cx: &mut App) {
        if !self.0.expired.borrow_mut().insert(n.workspace.clone()) {
            return;
        }
        self.0.notifier.show_session_expired(&n, cx);
    }

    /// A workspace signed in again: release the latch so a later expiry may
    /// notify again, and remove the delivered notice (a click would do
    /// nothing now).
    pub fn signed_in(&self, ws: &WorkspaceKey, cx: &mut App) {
        self.forget_expired(ws, cx);
    }

    /// Removes the expired-session notification of a workspace that is being
    /// forgotten (signed out or removed) or signed in again.
    fn forget_expired(&self, ws: &WorkspaceKey, cx: &mut App) {
        if self.0.expired.borrow_mut().remove(ws) {
            self.0.notifier.dismiss_tag(&session_expired_tag(ws), cx);
        }
    }

    /// Dismisses delivered notifications of `ws` for which `is_read` holds.
    pub fn dismiss_read(
        &self,
        ws: &WorkspaceKey,
        is_read: impl Fn(&ConversationId) -> bool,
        cx: &mut App,
    ) {
        let gone: Vec<ConvKey> = self
            .0
            .delivered
            .borrow()
            .keys()
            .filter(|(w, c)| w == ws && is_read(c))
            .cloned()
            .collect();
        for key in gone {
            self.0.delivered.borrow_mut().remove(&key);
            self.0.notifier.dismiss_conversation(&key.0, &key.1, cx);
        }
    }

    /// Records a workspace's mention total and updates the badge (sum over
    /// workspaces, hidden at 0) when it changed.
    pub fn set_mentions(&self, ws: &WorkspaceKey, mentions: u32, cx: &mut App) {
        self.0.mentions.borrow_mut().insert(ws.clone(), mentions);
        self.refresh_badge(cx);
    }

    /// Drops everything of a removed or signed-out workspace: its mentions
    /// and session, its delivered notifications; recomputes the badge.
    pub fn forget(&self, ws: &WorkspaceKey, cx: &mut App) {
        self.forget_expired(ws, cx);
        self.0.sessions.borrow_mut().remove(ws);
        self.0.mentions.borrow_mut().remove(ws);
        let gone: Vec<ConvKey> = self
            .0
            .delivered
            .borrow()
            .keys()
            .filter(|(w, _)| w == ws)
            .cloned()
            .collect();
        for key in gone {
            self.0.delivered.borrow_mut().remove(&key);
            self.0.notifier.dismiss_conversation(&key.0, &key.1, cx);
        }
        self.refresh_badge(cx);
    }

    fn refresh_badge(&self, cx: &mut App) {
        let sum: u32 = self.0.mentions.borrow().values().sum();
        let badge = (sum > 0).then_some(sum);
        if *self.0.badge.borrow() != Some(badge) {
            *self.0.badge.borrow_mut() = Some(badge);
            self.0.notifier.set_badge(badge, cx);
        }
    }

    /// A response to the notification with `tag`. `action` is the pressed
    /// button (`None` for the body), `text` the inline reply. Reply with text
    /// sends it to the notification's conversation (the thread, for a thread
    /// message); Mark as read marks that conversation (or thread) read; a body
    /// click, or Reply without text, opens the conversation. False for an
    /// unknown or expired tag or an unknown action. Nothing is logged but the
    /// outcome: no ids, no text.
    pub fn handle_action(
        tag_str: &str,
        action: Option<&str>,
        text: Option<String>,
        cx: &mut App,
    ) -> bool {
        let Some(this) = cx.try_global::<Notifications>().cloned() else {
            return false;
        };
        match action {
            None => Self::handle_response(tag_str, cx),
            Some(ACTION_REPLY) => {
                let text = text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
                let Some(text) = text else {
                    return Self::handle_response(tag_str, cx);
                };
                let Some((n, session)) = this.delivered_with_session(tag_str) else {
                    return false;
                };
                session.update(cx, |s, cx| match n.thread.clone() {
                    Some(root) => s.reply(
                        MessageId {
                            conversation: n.conversation.clone(),
                            ts: root,
                        },
                        text,
                        false,
                        cx,
                    ),
                    None => s.send(n.conversation.clone(), text, cx),
                });
                crate::log::info("notification reply sent (content not logged)");
                this.consume(&n, cx);
                true
            }
            Some(ACTION_MARK_READ) => {
                let Some((n, session)) = this.delivered_with_session(tag_str) else {
                    return false;
                };
                session.update(cx, |s, cx| match n.thread.clone() {
                    Some(root) => s.mark_thread_read(
                        MessageId {
                            conversation: n.conversation.clone(),
                            ts: root,
                        },
                        n.message.ts.clone(),
                        cx,
                    ),
                    None => {
                        let ts = s
                            .state()
                            .conversations
                            .get(&n.conversation)
                            .and_then(|c| c.latest.clone())
                            .filter(|l| *l > n.message.ts)
                            .unwrap_or_else(|| n.message.ts.clone());
                        s.apply(
                            domain::Input::MarkRead {
                                conversation: n.conversation.clone(),
                                ts,
                            },
                            cx,
                        );
                    }
                });
                this.consume(&n, cx);
                true
            }
            Some(_) => false,
        }
    }

    /// The delivered notification with `tag_str` and its running session.
    fn delivered_with_session(
        &self,
        tag_str: &str,
    ) -> Option<(Notification, Entity<WorkspaceSession>)> {
        let n = self
            .0
            .delivered
            .borrow()
            .values()
            .find(|n| tag(n) == tag_str)
            .cloned()?;
        let session = self.session(&n.workspace)?;
        Some((n, session))
    }

    /// Forgets and dismisses `n` once an action handled it.
    fn consume(&self, n: &Notification, cx: &mut App) {
        let key = (n.workspace.clone(), n.conversation.clone());
        if self.0.delivered.borrow_mut().remove(&key).is_some() {
            self.0.notifier.dismiss_conversation(&key.0, &key.1, cx);
        }
    }

    /// A click on the notification with `tag`: opens its workspace,
    /// conversation and thread and activates the app. False if unknown.
    pub fn handle_response(tag_str: &str, cx: &mut App) -> bool {
        let Some(this) = cx.try_global::<Notifications>().cloned() else {
            return false;
        };
        // An expired-session click opens the same sign-in window as the
        // banner's "Sign in again", for the workspace that expired.
        let expired_ws = this
            .0
            .expired
            .borrow()
            .iter()
            .find(|ws| session_expired_tag(ws) == tag_str)
            .cloned();
        if let Some(ws) = expired_ws {
            crate::tray::bring_to_front(cx);
            if let Some(opener) = this.0.sign_in_opener.borrow().clone() {
                opener(&ws, cx);
            }
            return true;
        }
        let found = this
            .0
            .delivered
            .borrow()
            .values()
            .find(|n| tag(n) == tag_str)
            .cloned();
        let Some(n) = found else { return false };
        if n.conversation
            .0
            .starts_with(crate::reminders::REMINDER_PREFIX)
        {
            // The reminder's own workspace first, then its Reminders view.
            if let Some(w) = cx.try_global::<crate::workspaces::WorkspacesHandle>() {
                let w = w.0.clone();
                w.update(cx, |w, cx| w.activate(&n.workspace, cx));
            }
            let session = this.0.sessions.borrow().get(&n.workspace).cloned();
            if let Some(session) = session.and_then(|s| s.upgrade()) {
                session.update(cx, |s, cx| s.show_reminders(cx));
            }
            crate::tray::bring_to_front(cx);
            return true;
        }
        // The message's own workspace first, so the window follows it.
        if let Some(w) = cx.try_global::<crate::workspaces::WorkspacesHandle>() {
            let w = w.0.clone();
            w.update(cx, |w, cx| w.activate(&n.workspace, cx));
        }
        let session = this.0.sessions.borrow().get(&n.workspace).cloned();
        if let Some(session) = session.and_then(|s| s.upgrade()) {
            session.update(cx, |s, cx| {
                s.open(n.conversation.clone(), cx);
                if let Some(root) = n.thread.clone() {
                    s.open_thread(
                        MessageId {
                            conversation: n.conversation.clone(),
                            ts: root,
                        },
                        cx,
                    );
                }
            });
        }
        crate::tray::bring_to_front(cx);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui::TestAppContext;

    use super::*;
    use domain::test_support::{cid, mid};

    struct FakePermission {
        result: Cell<NotificationPermission>,
        requests: Cell<u32>,
        opens: Cell<u32>,
    }

    impl FakePermission {
        fn new(result: NotificationPermission) -> Rc<Self> {
            Rc::new(Self {
                result: Cell::new(result),
                requests: Cell::new(0),
                opens: Cell::new(0),
            })
        }
    }

    impl PermissionChecker for FakePermission {
        fn request(&self, tx: UnboundedSender<NotificationPermission>) {
            self.requests.set(self.requests.get() + 1);
            let _ = tx.unbounded_send(self.result.get());
        }

        fn open_settings(&self) {
            self.opens.set(self.opens.get() + 1);
        }
    }

    fn install(cx: &mut TestAppContext, fake: Rc<FakePermission>) -> Notifications {
        cx.update(|cx| {
            i18n::gpui::init(
                cx,
                &[crate::catalog()],
                i18n::LocalePref::Code("en".parse().unwrap()),
            );
            cx.set_global(ConfigBanner::default());
            let hub = Notifications::new(Rc::new(NoopNotifier)).with_permissions(fake);
            cx.set_global(hub.clone());
            hub
        })
    }

    fn notice(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| {
            cx.global::<ConfigBanner>()
                .notices
                .iter()
                .find(|(k, _)| *k == PERMISSION_NOTICE)
                .map(|(_, t)| t.clone())
        })
    }

    #[gpui::test]
    fn denied_shows_the_notice_and_allowed_clears_it(cx: &mut TestAppContext) {
        let fake = FakePermission::new(NotificationPermission::Denied);
        let hub = install(cx, fake.clone());
        cx.update(|cx| request_permission_with(&hub, true, cx));
        cx.run_until_parked();
        assert_eq!(fake.requests.get(), 1);
        let shown = notice(cx).expect("the denied notice");
        assert!(shown.contains("System Settings"), "{shown}");

        fake.result.set(NotificationPermission::Allowed);
        cx.update(|cx| request_permission_with(&hub, true, cx));
        cx.run_until_parked();
        assert_eq!(fake.requests.get(), 2);
        assert_eq!(notice(cx), None, "allowed clears the notice");
    }

    #[gpui::test]
    fn an_undetermined_result_shows_no_notice(cx: &mut TestAppContext) {
        let fake = FakePermission::new(NotificationPermission::Undetermined);
        let hub = install(cx, fake.clone());
        cx.update(|cx| {
            cx.global_mut::<ConfigBanner>()
                .set_notice(PERMISSION_NOTICE, Some("stale".into()));
        });
        cx.update(|cx| request_permission_with(&hub, true, cx));
        cx.run_until_parked();
        assert_eq!(notice(cx), None);
    }

    #[gpui::test]
    fn nothing_is_asked_outside_an_app_bundle(cx: &mut TestAppContext) {
        let fake = FakePermission::new(NotificationPermission::Denied);
        let hub = install(cx, fake.clone());
        cx.update(|cx| request_permission_with(&hub, false, cx));
        cx.run_until_parked();
        assert_eq!(fake.requests.get(), 0);
        assert_eq!(notice(cx), None);
        let mut banner = ConfigBanner::default();
        apply_permission_to(&mut banner, NotificationPermission::Denied, false, "off");
        assert!(banner.notices.is_empty());
    }

    fn sample(ts: &str) -> Notification {
        Notification {
            workspace: WorkspaceKey::new("fake", "w1"),
            conversation: cid("D1"),
            message: mid("D1", ts),
            thread: None,
            author: "t".into(),
            channel: None,
            text: "b".into(),
            file_count: 0,
            silent: false,
        }
    }

    #[test]
    fn a_newer_notification_replaces_the_conversations_tag() {
        let g = GpuiNotifier::new();
        let n1 = sample("2.000000");
        let n2 = sample("3.000000");
        assert_eq!(
            g.replace_tag((n1.workspace.clone(), n1.conversation.clone()), tag(&n1)),
            None
        );
        assert_eq!(
            g.replace_tag((n2.workspace.clone(), n2.conversation.clone()), tag(&n2)),
            Some(tag(&n1))
        );
    }
}
