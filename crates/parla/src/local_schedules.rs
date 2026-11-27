//! The local schedule runner: one GPUI timer for the next due time, the send
//! of due items, and the prompt for items that are late. The rules (what is
//! due, late, in flight, failed) live in `domain::scheduling`; this file only
//! runs its effects.

use std::sync::Arc;
use std::time::Duration;

use components::{ScheduledSource, SendLaterConfig};
use domain::{
    CallOutput, ConversationId, Input, LocalSchedule, MessageId, MessageTs, ProviderCall,
    ProviderError, ScheduleId, ScheduledMessage, Upload,
};
use gpui::{App, Context, Task};

use crate::session::{SessionEvent, WorkspaceSession, new_client_id};

/// Timer and prompt state of one session.
#[derive(Default)]
pub(crate) struct LocalRunner {
    /// Unix time the single timer is armed for.
    pub(crate) armed: Option<i64>,
    pub(crate) timer: Option<Task<()>>,
    /// Late items waiting for the user's answer.
    pub(crate) late: Option<Vec<ScheduleId>>,
    /// The local items were read from the store (once per session).
    pub(crate) loaded: bool,
}

impl WorkspaceSession {
    /// Unix time of the armed wake-up, if any.
    pub fn armed_at(&self) -> Option<i64> {
        self.local_runner.armed
    }

    /// Late local items waiting for "Send now" or "Keep scheduled".
    pub fn late_prompt(&self) -> Option<&[ScheduleId]> {
        self.local_runner.late.as_deref()
    }

    /// Reads the local schedules from the store, once, after the first
    /// connection: items left in flight by a crash become failed, and due
    /// ones are sent or asked about by the first tick.
    pub(crate) fn load_local_schedules(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.local_runner.loaded, true) {
            return;
        }
        let (st, ws) = (self.store_handle(), self.state.workspace.clone());
        self.run(
            cx,
            async move { st.local_schedules(&ws).await },
            |s, r, cx| s.local_schedules_read(r, cx),
        );
    }

    /// The store's answer. A failed read is logged and leaves the session
    /// unloaded, so the next connection tries again.
    #[doc(hidden)]
    pub fn local_schedules_read(
        &mut self,
        r: Result<Vec<LocalSchedule>, store::StoreError>,
        cx: &mut Context<Self>,
    ) {
        match r {
            Ok(all) => {
                self.apply(Input::LocalSchedulesLoaded(all), cx);
                self.local_tick(cx);
            }
            Err(e) => {
                self.local_runner.loaded = false;
                crate::log::warn(&format!("local schedules load failed: {e}"));
            }
        }
    }

    /// Whether the local schedules were read from the store.
    #[doc(hidden)]
    pub fn local_schedules_loaded(&self) -> bool {
        self.local_runner.loaded
    }

    /// Arms the one timer for `at` (unix seconds), replacing any earlier one;
    /// `None` cancels it.
    pub(crate) fn wake_at(&mut self, at: Option<i64>, cx: &mut Context<Self>) {
        self.local_runner.timer = None;
        self.local_runner.armed = at;
        let Some(at) = at else { return };
        let secs = (at - components::now_secs(cx)).max(0) as u64;
        self.local_runner.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(secs))
                .await;
            this.update(cx, |s, cx| {
                s.local_runner.timer = None;
                s.local_runner.armed = None;
                s.local_tick(cx);
            })
            .ok();
        }));
    }

    /// Checks the local schedules against the clock now (timer, startup,
    /// resume from sleep).
    pub fn local_tick(&mut self, cx: &mut Context<Self>) {
        if self.state.local_schedules.is_empty() {
            return;
        }
        let now = components::now_secs(cx);
        self.apply(Input::LocalTick { now }, cx);
    }

    /// Remembers the late items and tells the window to ask.
    pub(crate) fn ask_late(&mut self, ids: Vec<ScheduleId>, cx: &mut Context<Self>) {
        self.local_runner
            .late
            .get_or_insert_with(Vec::new)
            .extend(ids);
        cx.emit(SessionEvent::LatePrompt);
        cx.notify();
    }

    /// Answers the late prompt. `true` sends the late items now. `false`
    /// keeps them scheduled: they stay in the Scheduled view (where each can be
    /// sent or deleted), nothing is re-armed for them, and the next launch asks
    /// again.
    pub fn answer_late(&mut self, send: bool, cx: &mut Context<Self>) {
        let Some(ids) = self.local_runner.late.take() else {
            return;
        };
        if send {
            self.apply(Input::SendLateAnswer { ids, send }, cx);
        } else {
            cx.notify();
        }
    }

    /// Sends a local item (its files, then its text) and feeds the answer back.
    pub(crate) fn send_local(&mut self, l: LocalSchedule, cx: &mut Context<Self>) {
        let p = self.provider.clone();
        let id = l.id.clone();
        let client = new_client_id(cx);
        self.run(
            cx,
            async move {
                let text = l.text.to_string();
                if l.files.is_empty() {
                    p.send(&l.conversation, &text, l.thread.clone(), client)
                        .await
                        .map(|_| ())
                } else {
                    let text = (!text.is_empty()).then_some(text);
                    p.upload(
                        &l.conversation,
                        l.thread.clone(),
                        l.files,
                        text.as_deref(),
                        client,
                    )
                    .await
                    .map(|_| ())
                }
            },
            move |s, result: Result<(), ProviderError>, cx| {
                s.apply(Input::LocalSent { id, result }, cx)
            },
        );
    }

    /// Schedules `text` (and `files`) for `post_at` unix seconds. The provider
    /// keeps a text-only one when it can; files, or a provider that cannot,
    /// keep it here until the time comes.
    pub fn schedule(
        &mut self,
        conversation: ConversationId,
        thread: Option<MessageTs>,
        text: String,
        files: Vec<Upload>,
        post_at: i64,
        cx: &mut Context<Self>,
    ) {
        let now_ms = components::now(cx).as_millisecond();
        let local_id = ScheduleId(Arc::from(format!("local-{}", new_client_id(cx).0)));
        self.apply(
            Input::ScheduleSend {
                conversation,
                thread,
                text: Arc::from(text),
                files,
                post_at,
                local_id,
                now_ms,
            },
            cx,
        );
    }

    /// Removes a scheduled item, provider-side or local.
    pub fn delete_schedule(&mut self, id: ScheduleId, cx: &mut Context<Self>) {
        self.apply(Input::DeleteSchedule(id), cx);
    }

    /// Replaces a provider-side item's text and time.
    pub fn reschedule(
        &mut self,
        item: ScheduledMessage,
        text: String,
        post_at: i64,
        cx: &mut Context<Self>,
    ) {
        self.apply(
            Input::Reschedule {
                item,
                text: Arc::from(text),
                post_at,
            },
            cx,
        );
    }

    /// Sends a scheduled item now: a local one goes out as is; a provider-side
    /// one is removed and posted as an ordinary message.
    pub fn send_schedule_now(&mut self, id: ScheduleId, cx: &mut Context<Self>) {
        if self.state.local_schedules.iter().any(|l| l.id == id) {
            self.apply(
                Input::SendLateAnswer {
                    ids: vec![id],
                    send: true,
                },
                cx,
            );
        } else if let Some(m) = self.state.scheduled.iter().find(|m| m.id == id).cloned() {
            // Post only once the provider confirmed the delete: a failed delete
            // would otherwise send the message twice (it is still scheduled).
            let p = self.provider.clone();
            let item = m.clone();
            self.run(
                cx,
                async move { p.delete_scheduled(&item).await.map(|()| CallOutput::None) },
                move |s, result, cx| {
                    let deleted = result.is_ok();
                    s.apply(
                        Input::CallFinished {
                            call: ProviderCall::DeleteScheduled(m.clone()),
                            result,
                        },
                        cx,
                    );
                    if deleted {
                        s.post_text(m.conversation, m.thread, m.text.to_string(), cx);
                    }
                },
            );
        }
    }

    /// Sends a failed local item again.
    pub fn retry_schedule(&mut self, id: ScheduleId, cx: &mut Context<Self>) {
        let Some(l) = self
            .state
            .local_schedules
            .iter()
            .find(|l| l.id == id)
            .cloned()
        else {
            return;
        };
        self.apply(Input::DeleteSchedule(id), cx);
        if l.files.is_empty() {
            self.post_text(l.conversation, l.thread, l.text.to_string(), cx);
        } else {
            let now = components::now_secs(cx);
            self.schedule(
                l.conversation,
                l.thread,
                l.text.to_string(),
                l.files,
                now,
                cx,
            );
        }
    }

    fn post_text(
        &mut self,
        c: ConversationId,
        thread: Option<MessageTs>,
        text: String,
        cx: &mut Context<Self>,
    ) {
        match thread {
            None => self.send(c, text, cx),
            Some(ts) => self.reply(
                MessageId {
                    conversation: c,
                    ts,
                },
                text,
                false,
                cx,
            ),
        }
    }

    /// Presets, zone, clock and capability for the Send later menu.
    pub(crate) fn send_later_settings(&self, cx: &App) -> SendLaterConfig {
        let st = self.settings(cx);
        SendLaterConfig {
            presets: st.send_later.presets.clone(),
            zone: domain::state_schedule_zone(
                &self.state,
                st.send_later.timezone.as_deref(),
                jiff::tz::TimeZone::system(),
            ),
            clock: st.messages.clock,
            server: self.provider.capabilities().scheduled_send,
        }
    }

    /// Items waiting to go out (the sidebar count).
    pub(crate) fn scheduled_total(&self) -> usize {
        self.state.scheduled.len() + self.state.local_schedules.len()
    }
}

impl ScheduledSource for WorkspaceSession {
    fn scheduled(&self) -> &[ScheduledMessage] {
        &self.state.scheduled
    }

    fn local(&self) -> &[LocalSchedule] {
        &self.state.local_schedules
    }

    fn reschedule(&mut self, item: ScheduledMessage, text: String, post_at: i64, cx: &mut App) {
        self.defer_to(cx, move |s, cx| {
            WorkspaceSession::reschedule(s, item, text, post_at, cx)
        });
    }

    fn delete(&mut self, id: ScheduleId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.delete_schedule(id, cx));
    }

    fn send_now(&mut self, id: ScheduleId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.send_schedule_now(id, cx));
    }

    fn retry(&mut self, id: ScheduleId, cx: &mut App) {
        self.defer_to(cx, move |s, cx| s.retry_schedule(id, cx));
    }

    fn conversation_label(&self, c: &ConversationId, cx: &App) -> String {
        self.label_of(c, cx)
    }

    fn send_later_config(&self, cx: &App) -> SendLaterConfig {
        self.send_later_settings(cx)
    }
}
