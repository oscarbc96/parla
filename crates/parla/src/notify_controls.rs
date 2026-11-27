//! Notification controls of a workspace session: pausing notifications
//! (the user's own do-not-disturb), per-conversation levels, and Parla's own
//! levels for providers that cannot keep them. The rules live in
//! `domain::dnd_logic`; this file wires them to the session and the store.

use std::time::Duration;

use components::{NotifyControls, SendLaterConfig};
use domain::{ConversationId, ConversationPrefs, DndEnds, Input};
use gpui::{App, Context, Task};
use jiff::tz::TimeZone;

use crate::session::WorkspaceSession;

/// The one timer that wakes the views when the user's pause ends.
#[derive(Default)]
pub(crate) struct NotifyRunner {
    /// Unix time the timer is armed for.
    pub(crate) armed: Option<i64>,
    pub(crate) timer: Option<Task<()>>,
}

/// The one timer that ends the earliest timed mute of a conversation.
#[derive(Default)]
pub(crate) struct MuteRunner {
    /// Unix time the timer is armed for.
    pub(crate) armed: Option<i64>,
    pub(crate) timer: Option<Task<()>>,
}

impl WorkspaceSession {
    /// Reads Parla's own conversation levels from the store (once per open).
    pub(crate) fn load_local_notify_prefs(&self, cx: &mut Context<Self>) {
        let (st, ws) = (self.store_handle(), self.state.workspace.clone());
        self.run(
            cx,
            async move { st.local_notify_prefs(&ws).await },
            |s, r, cx| {
                if let Ok(all) = r {
                    s.apply(Input::LocalNotifyPrefsLoaded(all), cx);
                }
            },
        );
    }

    /// Reads Parla's own per-conversation extras from the store (once per open).
    pub(crate) fn load_local_notify_extras(&self, cx: &mut Context<Self>) {
        let (st, ws) = (self.store_handle(), self.state.workspace.clone());
        self.run(
            cx,
            async move { st.local_notify_extras(&ws).await },
            |s, r, cx| {
                if let Ok(all) = r {
                    s.apply(Input::NotifyExtrasLoaded(all), cx);
                }
            },
        );
    }

    /// Unix time the timed-mute wake-up is armed for, if any.
    pub fn mute_armed_at(&self) -> Option<i64> {
        self.mute_runner.armed
    }

    /// Arms one timer for the earliest timed mute, so the mute ends at its
    /// stored deadline (also after a restart: the deadline is persisted).
    /// Not polling: it fires once and is re-armed only by a change.
    pub(crate) fn arm_mute_wake(&mut self, cx: &mut Context<Self>) {
        let next = domain::next_mute_deadline(&self.state);
        let now = components::now_secs(cx);
        if next.is_some_and(|at| at <= now) {
            self.mute_runner.timer = None;
            self.mute_runner.armed = None;
            self.apply(Input::NotifyMuteTick { now }, cx);
            return;
        }
        if self.mute_runner.armed == next {
            return;
        }
        self.mute_runner.timer = None;
        self.mute_runner.armed = next;
        let Some(at) = next else { return };
        let secs = (at - now) as u64;
        self.mute_runner.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(secs))
                .await;
            this.update(cx, |s, cx| {
                s.mute_runner.timer = None;
                s.mute_runner.armed = None;
                let now = components::now_secs(cx);
                s.apply(Input::NotifyMuteTick { now }, cx);
                // A timer that fired early ends nothing: arm again.
                s.arm_mute_wake(cx);
            })
            .ok();
        }));
    }

    /// Reads Parla's own stars from the store (once per open).
    pub(crate) fn load_starred(&self, cx: &mut Context<Self>) {
        let (st, ws) = (self.store_handle(), self.state.workspace.clone());
        self.run(
            cx,
            async move { st.local_starred(&ws).await },
            |s, r, cx| {
                if let Ok(all) = r {
                    s.apply(Input::StarredLoaded(all), cx);
                }
            },
        );
    }

    /// Whether the user starred the conversation.
    pub fn is_starred(&self, c: &ConversationId) -> bool {
        self.state.starred.contains(c)
    }

    /// Stars or unstars a conversation: Parla's own list, no provider call.
    pub fn set_starred(&mut self, c: ConversationId, starred: bool, cx: &mut Context<Self>) {
        self.apply(
            Input::SetStarred {
                conversation: c,
                starred,
            },
            cx,
        );
    }

    /// Marks the conversation read up to its latest message.
    pub fn mark_conversation_read(&mut self, c: &ConversationId, cx: &mut Context<Self>) {
        let ts = self
            .state
            .conversations
            .get(c)
            .and_then(|c| c.latest.clone());
        if let Some(ts) = ts {
            self.apply(
                Input::MarkRead {
                    conversation: c.clone(),
                    ts,
                },
                cx,
            );
        }
    }

    /// Arms one timer for the end of the user's pause so the rail and the
    /// sidebar drop their paused marks; no timer while nothing is paused.
    /// Not polling: it fires once and is re-armed only by a change.
    pub(crate) fn arm_pause_wake(&mut self, cx: &mut Context<Self>) {
        let now = components::now_secs(cx);
        let end = self
            .state
            .self_user
            .as_ref()
            .and_then(|me| self.state.dnd.get(me))
            .and_then(|d| domain::dnd_ends(d, now, &TimeZone::UTC))
            .map(|e| e.at.timestamp().as_second())
            .filter(|&end| end > now);
        if self.notify_runner.armed == end {
            return;
        }
        self.notify_runner.timer = None;
        self.notify_runner.armed = end;
        let Some(end) = end else { return };
        let secs = (end - now) as u64;
        self.notify_runner.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(secs))
                .await;
            this.update(cx, |s, cx| {
                s.notify_runner.timer = None;
                s.notify_runner.armed = None;
                s.arm_pause_wake(cx);
                // Reminder alerts held by the pause come now.
                s.arm_reminder_wake(cx);
                cx.notify();
            })
            .ok();
        }));
    }
}

impl NotifyControls for WorkspaceSession {
    fn can_pause(&self) -> bool {
        self.provider.capabilities().dnd
    }

    fn paused(&self, now: i64, tz: &TimeZone) -> Option<DndEnds> {
        let me = self.state.self_user.as_ref()?;
        domain::dnd_ends(self.state.dnd.get(me)?, now, tz)
    }

    fn pause(&mut self, minutes: u32, cx: &mut App) {
        if !self.can_pause() {
            return;
        }
        let now_unix = components::now_secs(cx);
        self.defer_to(cx, move |s, cx| {
            s.apply(Input::Snooze { minutes, now_unix }, cx)
        });
    }

    fn resume(&mut self, cx: &mut App) {
        if !self.can_pause() {
            return;
        }
        self.defer_to(cx, |s, cx| s.apply(Input::EndSnooze, cx));
    }

    fn conversation_prefs(&self, c: &ConversationId) -> ConversationPrefs {
        self.state.prefs_for(c).copied().unwrap_or_else(|| {
            // Muted by the provider: the menu offers to unmute.
            ConversationPrefs {
                muted: self.state.conversations.get(c).is_some_and(|c| c.is_muted),
                ..Default::default()
            }
        })
    }

    fn set_conversation_prefs(&mut self, c: ConversationId, p: ConversationPrefs, cx: &mut App) {
        // A provider that keeps the levels gets them; otherwise Parla does, and
        // the provider (whose route is unverified) is never called.
        let server = self.provider.capabilities().notify_prefs;
        self.defer_to(cx, move |s, cx| {
            let input = if server {
                Input::SetNotifyPrefs {
                    conversation: c,
                    prefs: p,
                }
            } else {
                Input::SetLocalNotifyPrefs {
                    conversation: c,
                    prefs: p,
                }
            };
            s.apply(input, cx)
        });
    }

    fn notify_extras(&self, c: &ConversationId) -> domain::NotifyExtras {
        self.state
            .local_notify_extras
            .get(c)
            .copied()
            .unwrap_or_default()
    }

    fn set_notify_extras(&mut self, c: ConversationId, e: domain::NotifyExtras, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            s.apply(
                Input::SetNotifyExtras {
                    conversation: c,
                    extras: e,
                },
                cx,
            )
        });
    }

    fn notify_config(&self, cx: &App) -> SendLaterConfig {
        self.send_later_settings(cx)
    }
}
