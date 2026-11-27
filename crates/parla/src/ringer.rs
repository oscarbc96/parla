//! Ringing: an incoming huddle rings with a notification (Join and Decline),
//! the chime and an in-app banner. `domain` decides whether a ring starts
//! (DND and pause, `calls.ring`, reconnects, the huddle you are already in);
//! this file only presents it. Quiet hours and the sound settings are held
//! here like for any other notification.
//!
//! One ring at a time across workspaces: a second ring is a missed
//! notification only. The only timer is one one-shot `RING_TIMEOUT`, armed
//! while a ring is active and dropped with it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use components::RingBannerData;
use domain::notify::Notification;
use domain::{ConversationId, HuddleId, Input, MessageId, MessageTs, RING_TIMEOUT, Ring, RingEnd};
use gpui::{
    App, AppContext as _, Context, Entity, Global, SharedString, SystemNotificationResponse, Task,
};
use settings::WorkspaceKey;

use crate::call_manager::CallManagerHandle;
use crate::notifications::{Notifications, RING_DECLINE, RING_JOIN, ring_tag};
use crate::session::WorkspaceSession;

/// A playing ringtone; dropping it stops the sound.
pub struct RingtoneHandle(Option<Box<dyn FnOnce()>>);

impl RingtoneHandle {
    pub fn new(stop: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(stop)))
    }
}

impl Drop for RingtoneHandle {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            stop();
        }
    }
}

/// Plays the ringtone: the platform's in the app, a recorder in tests.
pub trait RingtonePlayer {
    fn play(&self, looped: bool) -> Option<RingtoneHandle>;
}

/// The platform player (`NSSound` on macOS, silent elsewhere until M7).
pub struct SystemPlayer;

impl RingtonePlayer for SystemPlayer {
    fn play(&self, looped: bool) -> Option<RingtoneHandle> {
        crate::platform::play_ringtone(looped).map(RingtoneHandle::new)
    }
}

/// How the chime plays for a ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Play {
    /// `calls.ringtone` or the sound setting is `none`.
    Off,
    /// The window is focused on that conversation.
    Once,
    Loop,
}

/// Everything the ringer needs to present one ring.
pub struct RingStart {
    pub workspace: WorkspaceKey,
    pub ring: Ring,
    pub caller: String,
    pub conversation: String,
    pub play: Play,
}

/// What the banner shows; the window observes it.
#[derive(Default)]
pub struct RingState {
    banner: Option<RingBannerData>,
}

struct Active {
    workspace: WorkspaceKey,
    conversation: ConversationId,
    huddle: HuddleId,
    caller: String,
    name: String,
    chime: Option<RingtoneHandle>,
    timer: Option<Task<()>>,
}

struct Inner {
    player: Rc<dyn RingtonePlayer>,
    active: RefCell<Option<Active>>,
    state: Entity<RingState>,
}

/// The app's ringer; a cheap handle in a GPUI global.
#[derive(Clone)]
pub struct Ringer(Rc<Inner>);

impl Global for Ringer {}

/// Who is ringing, where (tests and the shell).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveRing {
    pub workspace: WorkspaceKey,
    pub conversation: ConversationId,
    pub caller: String,
}

impl Ringer {
    pub fn new(player: Rc<dyn RingtonePlayer>, cx: &mut App) -> Self {
        Self(Rc::new(Inner {
            player,
            active: RefCell::default(),
            state: cx.new(|_| RingState::default()),
        }))
    }

    pub fn install(self, cx: &mut App) {
        cx.set_global(self);
    }

    /// The model the window observes to show the banner.
    pub fn state(&self) -> Entity<RingState> {
        self.0.state.clone()
    }

    pub fn active(&self) -> Option<ActiveRing> {
        self.0.active.borrow().as_ref().map(|a| ActiveRing {
            workspace: a.workspace.clone(),
            conversation: a.conversation.clone(),
            caller: a.caller.clone(),
        })
    }

    /// The banner to show now.
    pub fn banner(&self, cx: &App) -> Option<RingBannerData> {
        self.0.state.read(cx).banner.clone()
    }

    /// The one-shot ring timer is armed (tests).
    pub fn timer_armed(&self) -> bool {
        self.0
            .active
            .borrow()
            .as_ref()
            .is_some_and(|a| a.timer.is_some())
    }

    /// Shows a ring: notification, chime, banner and the timeout timer. A
    /// ring while another is active is only a missed notification, and the
    /// workspace's ring ends at once so it can ring again later.
    pub fn start(&self, s: RingStart, cx: &mut App) {
        let Some(hub) = cx.try_global::<Notifications>().cloned() else {
            return;
        };
        let in_call = cx
            .try_global::<CallManagerHandle>()
            .map(|h| h.0.clone())
            .is_some_and(|m| m.read(cx).in_call(cx));
        if self.0.active.borrow().is_some() || in_call {
            missed(
                &hub,
                &s.workspace,
                &s.ring.conversation,
                &s.caller,
                &s.conversation,
                &s.ring.huddle,
                cx,
            );
            let ws = s.workspace.clone();
            let since = s.ring.since;
            cx.defer(move |cx| end_ring(&ws, since, cx));
            return;
        }
        hub.ring(&s.workspace, &s.ring, &s.caller, cx);
        let chime = match s.play {
            Play::Off => None,
            Play::Once => self.0.player.play(false),
            Play::Loop => self.0.player.play(true),
        };
        let wait = RING_TIMEOUT.saturating_sub(s.ring.since.elapsed());
        let timer = self.arm(wait, s.workspace.clone(), s.ring.since, cx);
        self.0.state.update(cx, |st, cx| {
            st.banner = Some(RingBannerData {
                caller: SharedString::from(s.caller.clone()),
            });
            cx.notify();
        });
        *self.0.active.borrow_mut() = Some(Active {
            workspace: s.workspace,
            conversation: s.ring.conversation,
            huddle: s.ring.huddle,
            caller: s.caller,
            name: s.conversation,
            chime,
            timer: Some(timer),
        });
    }

    /// One one-shot timer; on expiry the workspace's reducer ends the ring.
    fn arm(
        &self,
        wait: Duration,
        ws: WorkspaceKey,
        since: std::time::Instant,
        cx: &mut App,
    ) -> Task<()> {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(wait).await;
            cx.update(|cx| end_ring(&ws, since, cx));
        })
    }

    /// The ring of `c` ended with `reason`: stops sound, banner, timer and
    /// notification; a timeout leaves a "missed" notification.
    pub fn stop(&self, ws: &WorkspaceKey, c: &ConversationId, reason: RingEnd, cx: &mut App) {
        let mine = self
            .0
            .active
            .borrow()
            .as_ref()
            .is_some_and(|a| a.workspace == *ws && a.conversation == *c);
        if !mine {
            return;
        }
        let Some(a) = self.0.active.borrow_mut().take() else {
            return;
        };
        drop(a.chime);
        drop(a.timer);
        self.0.state.update(cx, |st, cx| {
            st.banner = None;
            cx.notify();
        });
        if let Some(hub) = cx.try_global::<Notifications>().cloned() {
            hub.stop_ring(ws, c, cx);
            if reason == RingEnd::TimedOut {
                missed(&hub, ws, c, &a.caller, &a.name, &a.huddle, cx);
            }
        }
    }

    /// Join (banner or notification button): opens the huddle.
    pub fn join(cx: &mut App) {
        Self::with_active(cx, |s, cx, c| s.apply(Input::JoinHuddle(c), cx));
    }

    /// Decline (banner or notification button): tells the provider.
    pub fn decline(cx: &mut App) {
        Self::with_active(cx, |s, cx, _| s.apply(Input::DeclineRing, cx));
    }

    fn with_active(
        cx: &mut App,
        f: impl FnOnce(&mut WorkspaceSession, &mut Context<WorkspaceSession>, ConversationId),
    ) {
        let Some(active) = cx.try_global::<Ringer>().and_then(|r| r.active()) else {
            return;
        };
        let Some(sess) = cx
            .try_global::<Notifications>()
            .and_then(|h| h.session(&active.workspace))
        else {
            return;
        };
        sess.update(cx, |s, cx| f(s, cx, active.conversation));
    }

    /// A press on the ring notification: its Join or Decline button, or the
    /// body (shows the conversation, the ring goes on). False when `r` is not
    /// the ring notification.
    pub fn handle_response(r: &SystemNotificationResponse, cx: &mut App) -> bool {
        let Some(active) = cx.try_global::<Ringer>().and_then(|x| x.active()) else {
            return r.tag.ends_with("/ring");
        };
        if *r.tag != *ring_tag(&active.workspace, &active.conversation) {
            return false;
        }
        match r.action_id.as_deref() {
            Some(RING_JOIN) => Self::join(cx),
            Some(RING_DECLINE) => Self::decline(cx),
            _ => {
                if let Some(w) = cx.try_global::<crate::workspaces::WorkspacesHandle>() {
                    let w = w.0.clone();
                    w.update(cx, |w, cx| w.activate(&active.workspace, cx));
                }
                if let Some(s) = cx
                    .try_global::<Notifications>()
                    .and_then(|h| h.session(&active.workspace))
                {
                    s.update(cx, |s, cx| s.open(active.conversation.clone(), cx));
                }
                crate::tray::bring_to_front(cx);
            }
        }
        true
    }
}

/// Ends the ring of `ws` in its session: the deadline is `since + RING_TIMEOUT`
/// (the caller's clock, not a new `now`).
fn end_ring(ws: &WorkspaceKey, since: std::time::Instant, cx: &mut App) {
    if let Some(sess) = cx.try_global::<Notifications>().and_then(|h| h.session(ws)) {
        sess.update(cx, |s, cx| {
            s.apply(
                Input::RingTimeout {
                    now: since + RING_TIMEOUT,
                },
                cx,
            )
        });
    }
}

fn conversation_name(s: &WorkspaceSession, c: &ConversationId) -> String {
    s.state()
        .conversations
        .get(c)
        .map(|c| c.name.to_string())
        .unwrap_or_default()
}

/// "Missed huddle from {caller}", no actions; clicking opens the conversation.
fn missed(
    hub: &Notifications,
    ws: &WorkspaceKey,
    c: &ConversationId,
    caller: &str,
    conversation: &str,
    huddle: &HuddleId,
    cx: &mut App,
) {
    let l = i18n::gpui::localizer(cx);
    let title = t!(l, "ring-missed-title", caller = caller).into_owned();
    let text = if conversation.is_empty() {
        caller
    } else {
        conversation
    };
    hub.show(
        Notification {
            workspace: ws.clone(),
            conversation: c.clone(),
            message: MessageId {
                conversation: c.clone(),
                ts: MessageTs(Arc::from(format!(
                    "{}{}",
                    crate::notifications::MISSED_TS_PREFIX,
                    huddle.0
                ))),
            },
            thread: None,
            author: Arc::from(title.as_str()),
            channel: None,
            text: Arc::from(text),
            file_count: 0,
            silent: false,
        },
        cx,
    );
}

impl WorkspaceSession {
    /// `Effect::StartRing`: presents the ring unless quiet hours or the
    /// notification settings hold it back. `domain` has already applied DND,
    /// `calls.ring` and the reconnect rule.
    pub(crate) fn start_ring(&self, r: Ring, cx: &mut Context<Self>) {
        let Some(ringer) = cx.try_global::<Ringer>().cloned() else {
            return;
        };
        let settings = self.settings(cx);
        let now = components::now(cx).to_zoned_datetime(jiff::tz::TimeZone::system());
        if !domain::notify::reminder_alert_allowed(
            &settings.notifications,
            now.time(),
            now.weekday(),
        ) {
            // Held back: end the ring in the reducer so it cannot refuse later rings.
            let ws = self.state.workspace.clone();
            let since = r.since;
            cx.defer(move |cx| end_ring(&ws, since, cx));
            return;
        }
        // Focused means this workspace is the one on screen, not only that its
        // session has the conversation open in the background.
        let on_screen = cx
            .try_global::<crate::workspaces::WorkspacesHandle>()
            .is_none_or(|w| {
                w.0.read(cx)
                    .active()
                    .is_none_or(|a| a == &self.state.workspace)
            });
        let focused =
            cx.active_window().is_some() && on_screen && self.selected() == Some(&r.conversation);
        let play = if settings.calls.ringtone == "none" || settings.notifications.sound == "none" {
            Play::Off
        } else if focused {
            Play::Once
        } else {
            Play::Loop
        };
        let caller = self
            .state
            .members
            .get(&r.from)
            .map(|m| m.display_name.to_string())
            .unwrap_or_else(|| r.from.0.to_string());
        let conversation = conversation_name(self, &r.conversation);
        ringer.start(
            RingStart {
                workspace: self.state.workspace.clone(),
                ring: r,
                caller,
                conversation,
                play,
            },
            cx,
        );
    }

    /// `Effect::StopRing`.
    pub(crate) fn stop_ring(&self, c: &ConversationId, reason: RingEnd, cx: &mut Context<Self>) {
        if let Some(ringer) = cx.try_global::<Ringer>().cloned() {
            ringer.stop(&self.state.workspace, c, reason, cx);
        }
    }

    /// Before a ring is reduced, `calls.ring` is read from the settings (the
    /// cached effective settings, so this is a lookup, not polling).
    pub(crate) fn sync_ring_setting(&mut self, cx: &mut Context<Self>) {
        let on = self.settings(cx).calls.ring;
        if on != self.state.ring_enabled {
            self.apply(Input::SetRingEnabled(on), cx);
        }
    }
}
