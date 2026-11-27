//! Reminders: the session side of the Reminders view. A provider that keeps
//! reminders is asked (domain effects); one that does not leaves them to Parla,
//! which keeps them in the store and reads `when` itself.

use std::sync::Arc;

use components::{RemindersSource, SendLaterConfig};
use domain::{Input, Reminder, ReminderId, ReminderRepeat};
use std::time::Duration;

use domain::notify::Notification;
use domain::{ConversationId, MessageId, MessageTs};
use gpui::{App, Context, Task};

use crate::notifications::Notifications;

use crate::session::{WorkspaceSession, new_client_id};

/// The one timer that raises the alert of the next due local reminder.
#[derive(Default)]
pub(crate) struct ReminderRunner {
    /// Unix time the timer is armed for.
    pub(crate) armed: Option<i64>,
    pub(crate) timer: Option<Task<()>>,
}

/// Marks a notification as a reminder's (not a conversation to open).
pub(crate) const REMINDER_PREFIX: &str = "reminder:";

impl WorkspaceSession {
    /// Unix time of the armed reminder wake-up, if any.
    pub fn reminder_armed_at(&self) -> Option<i64> {
        self.reminder_runner.armed
    }

    /// Arms the single timer for the earliest pending local reminder, or none
    /// when there is none. A reminder already due is alerted now (startup,
    /// resume, a reminder added in the past). Not polling: re-armed only when
    /// reminders change or the timer fires.
    pub(crate) fn arm_reminder_wake(&mut self, cx: &mut Context<Self>) {
        let next = domain::next_reminder_due(&self.state);
        let now = components::now_secs(cx);
        if next.is_some_and(|at| at <= now) {
            self.reminder_runner.timer = None;
            self.reminder_runner.armed = None;
            self.apply(Input::ReminderTick { now }, cx);
            return;
        }
        if self.reminder_runner.armed == next {
            return;
        }
        self.reminder_runner.timer = None;
        self.reminder_runner.armed = next;
        let Some(at) = next else { return };
        let secs = (at - now) as u64;
        self.reminder_runner.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(secs))
                .await;
            this.update(cx, |s, cx| {
                s.reminder_runner.timer = None;
                s.reminder_runner.armed = None;
                let now = components::now_secs(cx);
                s.apply(Input::ReminderTick { now }, cx);
                // A timer that fired early alerts nothing: arm again.
                s.arm_reminder_wake(cx);
            })
            .ok();
        }));
    }

    /// Raises the normal desktop notification of a due local reminder.
    pub(crate) fn alert_reminder(&self, r: &Reminder, cx: &mut App) {
        let Some(hub) = cx.try_global::<Notifications>().cloned() else {
            return;
        };
        let settings = self.settings(cx).notifications.clone();
        let now = components::now(cx).to_zoned_datetime(jiff::tz::TimeZone::system());
        if !domain::notify::reminder_alert_allowed(&settings, now.time(), now.weekday()) {
            return;
        }
        let l = i18n::gpui::localizer(cx);
        let conversation = ConversationId(Arc::from(format!("{REMINDER_PREFIX}{}", r.id.0)));
        hub.show(
            Notification {
                workspace: self.state.workspace.clone(),
                message: MessageId {
                    conversation: conversation.clone(),
                    ts: MessageTs(r.id.0.clone()),
                },
                conversation,
                thread: None,
                author: Arc::from(t!(l, "notification-reminder-title").as_ref()),
                channel: None,
                // Preview off: only the title shows, no reminder text.
                text: if !settings.show_preview {
                    Arc::from("")
                } else if r.about_message() {
                    Arc::from(t!(l, "reminder-about-message").as_ref())
                } else {
                    r.text.clone()
                },
                file_count: 0,
                silent: settings.sound == "none",
            },
            cx,
        );
    }

    /// Asks the window to show this workspace's Reminders view.
    pub(crate) fn show_reminders(&mut self, cx: &mut Context<Self>) {
        self.apply(Input::OpenReminders, cx);
        cx.emit(crate::session::SessionEvent::ShowReminders);
    }

    /// Reads Parla's own reminders from the store, when the provider keeps none.
    pub(crate) fn load_local_reminders(&self, cx: &mut Context<Self>) {
        if self.provider.capabilities().reminders {
            return;
        }
        let (st, ws) = (self.store_handle(), self.state.workspace.clone());
        self.run(
            cx,
            async move { st.local_reminders(&ws).await },
            |s, r, cx| {
                // An unreadable store still ends the loading state.
                s.apply(Input::LocalRemindersLoaded(r.unwrap_or_default()), cx);
            },
        );
    }

    /// Reminders still to do (the sidebar count).
    pub(crate) fn reminders_open(&self) -> usize {
        self.state
            .reminders
            .as_ref()
            .map_or(0, |all| all.iter().filter(|r| !r.completed).count())
    }

    /// The input that adds a local reminder for the user, or `None` when `when`
    /// is not a time Parla can place or the reminder is for someone else.
    pub(crate) fn local_reminder(
        &self,
        text: &str,
        when: &str,
        for_me: bool,
        cx: &App,
    ) -> Option<Input> {
        let me = self.state.self_user.clone()?;
        if !for_me {
            return None;
        }
        let zone = self.send_later_settings(cx).zone;
        let now = components::now(cx).to_zoned_datetime(zone);
        let due = domain::parse_when(when, &now)?;
        Some(Input::AddLocalReminder {
            id: ReminderId(Arc::from(format!("local-{}", new_client_id(cx).0))),
            text: Arc::from(text),
            due: due.at,
            recurring: due.recurring,
            interval_secs: due.interval_secs,
            user: me,
            link: None,
        })
    }

    /// A message the session holds: in a conversation window, an open thread
    /// or the messages fetched for lists.
    fn held_message(&self, id: &MessageId) -> Option<&domain::Message> {
        let in_window = self
            .state
            .windows
            .get(&id.conversation)
            .and_then(|w| w.messages().iter().find(|m| m.id == *id));
        in_window
            .or_else(|| {
                self.state
                    .threads
                    .values()
                    .flat_map(|t| t.root.iter().chain(t.replies.messages()))
                    .find(|m| m.id == *id)
            })
            .or_else(|| self.state.referenced.get(id))
    }

    /// Adds one of Parla's own reminders about message `id`, due at `at`, with
    /// the message's author and the start of its text kept for the list.
    pub(crate) fn remind_about_message(&mut self, id: MessageId, at: i64, cx: &mut Context<Self>) {
        if self.state.caps.reminders {
            return;
        }
        let Some(me) = self.state.self_user.clone() else {
            return;
        };
        let (author, text) = self
            .held_message(&id)
            .map(|m| {
                let plain = self.content.parse_text(&m.text).plain_text();
                (m.author.clone(), plain)
            })
            .unwrap_or_default();
        // No text of its own: the list and the alert show a label in the
        // language of the moment.
        let input = Input::AddLocalReminder {
            id: ReminderId(Arc::from(format!("local-{}", new_client_id(cx).0))),
            text: Arc::from(""),
            due: at,
            recurring: false,
            interval_secs: 0,
            user: me,
            link: Some(domain::ReminderLink::new(id, author, &text)),
        };
        self.apply(input, cx);
    }

    /// When `when` falls for a local reminder (unix seconds).
    fn local_when(&self, when: &str, cx: &App) -> Option<domain::Due> {
        let zone = self.send_later_settings(cx).zone;
        domain::parse_when(when, &components::now(cx).to_zoned_datetime(zone))
    }

    /// Adds a reminder for the user: the provider is asked with `when` as
    /// typed; without provider reminders Parla keeps it.
    pub fn add_reminder(&mut self, text: String, when: String, cx: &mut Context<Self>) {
        let input = if self.state.caps.reminders {
            Some(Input::AddReminder {
                text: Arc::from(text.as_str()),
                when: Arc::from(when.as_str()),
                user: None,
            })
        } else {
            self.local_reminder(&text, &when, true, cx)
        };
        if let Some(input) = input {
            self.apply(input, cx);
        }
    }
}

impl RemindersSource for WorkspaceSession {
    fn reminders(&self) -> Option<&[Reminder]> {
        self.state.reminders.as_deref()
    }

    fn complete(&mut self, id: ReminderId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.apply(Input::CompleteReminder(id), cx));
    }

    fn delete(&mut self, id: ReminderId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.apply(Input::DeleteReminder(id), cx));
    }

    fn add(&mut self, text: String, when: String, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.add_reminder(text, when, cx));
    }

    fn when_ok(&self, when: &str, cx: &App) -> bool {
        if self.state.caps.reminders {
            return domain::is_when(when);
        }
        let zone = self.send_later_settings(cx).zone;
        domain::parse_when(when, &components::now(cx).to_zoned_datetime(zone)).is_some()
    }

    fn local(&self) -> bool {
        !self.state.caps.reminders
    }

    fn time_config(&self, cx: &App) -> SendLaterConfig {
        self.send_later_settings(cx)
    }

    fn snooze(&mut self, id: ReminderId, until: i64, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            s.apply(Input::SnoozeReminder { id, until }, cx)
        });
    }

    fn edit(
        &mut self,
        id: ReminderId,
        text: String,
        when: String,
        repeat: ReminderRepeat,
        cx: &mut App,
    ) {
        // A blank time keeps the reminder's; a new one is read as Add reads it.
        let due = if when.is_empty() {
            None
        } else {
            match self.local_when(&when, cx) {
                Some(d) => Some(d.at),
                None => return,
            }
        };
        let text = Arc::from(text.as_str());
        let now = components::now(cx).as_second();
        self.defer_to(cx, move |s, cx| {
            s.apply(
                Input::EditReminder {
                    id,
                    text,
                    due,
                    repeat,
                    now,
                },
                cx,
            )
        });
    }

    fn when_at(&self, when: &str, cx: &App) -> Option<i64> {
        if self.state.caps.reminders {
            return None;
        }
        self.local_when(when, cx).map(|d| d.at)
    }

    fn member(&self, u: &domain::UserId) -> Option<&domain::Member> {
        self.state.members.get(u)
    }

    fn conversation_label(&self, c: &ConversationId, cx: &App) -> String {
        <Self as components::CollectionsSource>::conversation_label(self, c, cx)
    }
}
