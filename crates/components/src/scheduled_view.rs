//! The per-workspace Scheduled view: provider-side and local schedules,
//! grouped by day.

use crate::controls::{ButtonKind, button as ui_button, field_frame, on_activate};
use domain::{ConversationId, LocalSchedule, LocalScheduleState, ScheduleId, ScheduledMessage};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    Subscription, Window, div, prelude::*,
};
use i18n::Translate as _;
use themes::ActiveTheme;
use themes::metrics::{
    FONT_SIZE_2XS, FONT_SIZE_MD, FONT_SIZE_XL, FONT_SIZE_XS, FONT_WEIGHT_BOLD,
    FONT_WEIGHT_SEMIBOLD, LAYOUT_FIELD_XS, RADIUS_FULL, RADIUS_LG, RADIUS_MD, SPACE_1, SPACE_2,
    SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_7, SPACE_8, SPACE_12,
};

use crate::datetime_picker::{CustomCheck, check_custom};
use crate::send_later::SendLaterConfig;
use crate::text_input::TextInput;

/// Key context of the view.
pub const KEY_CONTEXT: &str = "ScheduledView";

/// What the view needs from the session, so `components` does not depend on `parla`.
pub trait ScheduledSource {
    fn scheduled(&self) -> &[ScheduledMessage];
    fn local(&self) -> &[LocalSchedule];
    /// Replaces the text and time of a provider-side item (one action).
    fn reschedule(&mut self, item: ScheduledMessage, text: String, post_at: i64, cx: &mut App);
    fn delete(&mut self, id: ScheduleId, cx: &mut App);
    fn send_now(&mut self, id: ScheduleId, cx: &mut App);
    /// Sends a failed local item again.
    fn retry(&mut self, _id: ScheduleId, _cx: &mut App) {}
    fn conversation_label(&self, c: &ConversationId, _cx: &App) -> String {
        c.0.to_string()
    }
    fn send_later_config(&self, cx: &App) -> SendLaterConfig;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledRow {
    pub id: ScheduleId,
    pub local: bool,
    pub conversation: String,
    pub text: String,
    pub time: String,
    /// "local, needs app open", "sending", or the failure.
    pub tag: Option<String>,
    pub failed: bool,
    /// A local item is being sent right now.
    pub sending: bool,
    pub post_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayGroup {
    pub label: String,
    pub rows: Vec<ScheduledRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduledEvent {
    Close,
}

/// An item being edited in place: text, date and time, saved as one reschedule.
struct EditState {
    item: ScheduledMessage,
    text: Entity<TextInput>,
    date: Entity<TextInput>,
    time: Entity<TextInput>,
    invalid: bool,
    /// Tab stops after the fields, in order: Cancel, Save.
    cancel_focus: FocusHandle,
    save_focus: FocusHandle,
    _subs: Vec<Subscription>,
}

pub struct ScheduledView<S: ScheduledSource + 'static> {
    session: Entity<S>,
    edit: Option<EditState>,
    focus: FocusHandle,
    _observe: Subscription,
}

impl<S: ScheduledSource + 'static> EventEmitter<ScheduledEvent> for ScheduledView<S> {}

impl<S: ScheduledSource + 'static> ScheduledView<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            edit: None,
            focus: cx.focus_handle(),
            _observe: observe,
        }
    }

    /// Provider-side and local items by time, grouped by day in the schedule zone.
    pub fn groups(&self, cx: &App) -> Vec<DayGroup> {
        let s = self.session.read(cx);
        let cfg = s.send_later_config(cx);
        let hc = crate::message_row::hour_cycle(cfg.clock);
        let l = cx.localizer();
        let mut all: Vec<ScheduledRow> = Vec::new();
        let stamp = |at: i64| jiff::Timestamp::from_second(at).ok();
        for m in s.scheduled() {
            let Some(t) = stamp(m.post_at) else { continue };
            all.push(ScheduledRow {
                id: m.id.clone(),
                local: false,
                conversation: s.conversation_label(&m.conversation, cx),
                text: m.text.to_string(),
                time: l.time(t, &cfg.zone, hc),
                tag: None,
                failed: false,
                sending: false,
                post_at: m.post_at,
            });
        }
        for m in s.local() {
            let Some(t) = stamp(m.post_at) else { continue };
            let (tag, failed) = match &m.state {
                LocalScheduleState::Pending => (t!(l, "scheduled-local-tag").into_owned(), false),
                LocalScheduleState::Sending => (t!(l, "scheduled-sending").into_owned(), false),
                LocalScheduleState::Failed(why) => (
                    t!(l, "scheduled-failed", reason = why.as_str()).into_owned(),
                    true,
                ),
            };
            all.push(ScheduledRow {
                id: m.id.clone(),
                local: true,
                conversation: s.conversation_label(&m.conversation, cx),
                text: m.text.to_string(),
                time: l.time(t, &cfg.zone, hc),
                tag: Some(tag),
                failed,
                sending: m.state == LocalScheduleState::Sending,
                post_at: m.post_at,
            });
        }
        all.sort_by_key(|r| r.post_at);
        let mut out: Vec<(jiff::civil::Date, DayGroup)> = Vec::new();
        for r in all {
            let Some(t) = stamp(r.post_at) else { continue };
            let day = t.to_zoned_datetime(cfg.zone.clone()).date();
            match out.last_mut() {
                Some((d, g)) if *d == day => g.rows.push(r),
                _ => out.push((
                    day,
                    DayGroup {
                        label: l.date(t, &cfg.zone),
                        rows: vec![r],
                    },
                )),
            }
        }
        out.into_iter().map(|(_, g)| g).collect()
    }

    /// Opens the edit form of a provider-side item.
    pub fn begin_edit(&mut self, id: ScheduleId, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.session.read(cx);
        let Some(item) = s.scheduled().iter().find(|m| m.id == id).cloned() else {
            return;
        };
        let cfg = s.send_later_config(cx);
        let at = jiff::Timestamp::from_second(item.post_at)
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
            .to_zoned_datetime(cfg.zone);
        let date = at.date().to_string();
        let time = format!("{:02}:{:02}", at.hour(), at.minute());
        let (text, d, t) = (
            cx.new(TextInput::new),
            cx.new(TextInput::new),
            cx.new(TextInput::new),
        );
        text.update(cx, |i, cx| i.set_text(&item.text, cx));
        d.update(cx, |i, cx| i.set_text(&date, cx));
        t.update(cx, |i, cx| i.set_text(&time, cx));
        let subs = vec![
            cx.observe(&text, |_, _, cx| cx.notify()),
            cx.observe(&d, |_, _, cx| cx.notify()),
            cx.observe(&t, |_, _, cx| cx.notify()),
        ];
        let focus = text.read(cx).focus_handle(cx);
        self.edit = Some(EditState {
            item,
            text,
            date: d,
            time: t,
            invalid: false,
            cancel_focus: cx.focus_handle().tab_stop(true),
            save_focus: cx.focus_handle().tab_stop(true),
            _subs: subs,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Replaces the edit form's fields.
    pub fn set_edit(&mut self, text: &str, date: &str, time: &str, cx: &mut Context<Self>) {
        if let Some(e) = &self.edit {
            e.text.update(cx, |i, cx| i.set_text(text, cx));
            e.date.update(cx, |i, cx| i.set_text(date, cx));
            e.time.update(cx, |i, cx| i.set_text(time, cx));
        }
    }

    /// Saves the form as one reschedule; a blank text or a time that is not
    /// allowed keeps the form open.
    pub fn save_edit(&mut self, cx: &mut Context<Self>) {
        let Some(e) = &mut self.edit else { return };
        let cfg = self.session.read(cx).send_later_config(cx);
        let now = crate::now(cx).to_zoned_datetime(cfg.zone);
        let text = e.text.read(cx).text().trim().to_string();
        let ok = match check_custom(e.date.read(cx).text(), e.time.read(cx).text(), &now) {
            CustomCheck::Ok(at) if !text.is_empty() => Some(at),
            _ => None,
        };
        let Some(at) = ok else {
            e.invalid = true;
            cx.notify();
            return;
        };
        let item = e.item.clone();
        self.edit = None;
        self.session
            .update(cx, |s, cx| s.reschedule(item, text, at, cx));
        cx.notify();
    }

    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.edit = None;
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        match ev.keystroke.key.as_str() {
            "tab" if self.edit.is_some() => self.tab_in_edit(m.shift, window, cx),
            "escape" if self.edit.is_some() => {
                self.cancel_edit(cx);
                window.focus(&self.focus, cx);
            }
            "escape" => cx.emit(ScheduledEvent::Close),
            "enter" if self.edit.is_some() && !m.shift => {
                self.save_edit(cx);
                if self.edit.is_none() {
                    window.focus(&self.focus, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Tab order of the inline editor: text, date, time, Cancel, Save,
    /// wrapping.
    fn tab_in_edit(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &self.edit else { return };
        let order = [
            e.text.read(cx).focus_handle(cx),
            e.date.read(cx).focus_handle(cx),
            e.time.read(cx).focus_handle(cx),
            e.cancel_focus.clone(),
            e.save_focus.clone(),
        ];
        let n = order.len();
        let next = match order.iter().position(|h| h.is_focused(window)) {
            Some(i) if back => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None if back => n - 1,
            None => 0,
        };
        window.focus(&order[next], cx);
    }
}

impl<S: ScheduledSource + 'static> Focusable for ScheduledView<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

type Handler = Box<dyn Fn(&mut Window, &mut App)>;

impl<S: ScheduledSource + 'static> Render for ScheduledView<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let groups = self.groups(cx);
        let this = cx.entity();
        let mut body = div()
            .id("scheduled-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(SPACE_6)
            .pb(SPACE_6);
        if groups.is_empty() {
            body = body.child(
                crate::controls::empty_state(
                    t!(cx, "scheduled-empty").into_owned(),
                    t!(cx, "scheduled-empty-hint").into_owned(),
                    colors,
                )
                .id("scheduled-empty"),
            );
        } else {
            body = body.child(
                div()
                    .id("scheduled-explainer")
                    .mt(SPACE_6)
                    .px(SPACE_5)
                    .py(SPACE_4)
                    .rounded(RADIUS_MD)
                    .bg(colors.panel_background)
                    .text_xs()
                    .text_color(colors.text_muted)
                    .child(t!(cx, "scheduled-explainer").into_owned()),
            );
        }
        let soft = themes::parla_tokens(&theme);
        let pill = |label: String, bg: gpui::Hsla, fg: gpui::Hsla| {
            div()
                .min_w_0()
                .max_w_full()
                .px(SPACE_3)
                .py(SPACE_1)
                .rounded(RADIUS_FULL)
                .bg(bg)
                .text_color(fg)
                .text_size(FONT_SIZE_2XS)
                .font_weight(FONT_WEIGHT_SEMIBOLD)
                .child(label)
        };
        let mut n = 0usize;
        for g in groups {
            body = body.child(
                div()
                    .mt(SPACE_7)
                    .mb(SPACE_4)
                    .text_size(FONT_SIZE_2XS)
                    .font_weight(FONT_WEIGHT_SEMIBOLD)
                    .text_color(colors.text_muted)
                    .child(g.label.to_uppercase()),
            );
            for r in g.rows {
                n += 1;
                let editing = self.edit.as_ref().filter(|e| e.item.id == r.id).map(|e| {
                    (
                        e.text.clone(),
                        e.date.clone(),
                        e.time.clone(),
                        e.invalid,
                        e.cancel_focus.clone(),
                        e.save_focus.clone(),
                    )
                });
                let title = div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(SPACE_4)
                    .gap_y(SPACE_2)
                    .child(
                        div()
                            .text_size(themes::metrics::FONT_SIZE_SM)
                            .font_weight(FONT_WEIGHT_BOLD)
                            .child(
                                t!(cx, "scheduled-to", conversation = r.conversation.as_str())
                                    .into_owned(),
                            ),
                    )
                    .child(match (&r.tag, r.failed) {
                        (Some(tag), true) => {
                            pill(tag.clone(), soft.danger_soft, theme.status().error)
                        }
                        (Some(tag), false) => {
                            pill(tag.clone(), soft.warning_soft, theme.status().warning)
                        }
                        (None, _) => pill(
                            t!(cx, "scheduled-server-tag").into_owned(),
                            soft.success_soft,
                            theme.status().success,
                        ),
                    });
                let mut row = div()
                    .id(("scheduled-row", n))
                    .debug_selector({
                        let id = r.id.0.to_string();
                        move || format!("scheduled-row-{id}")
                    })
                    .mb(SPACE_4)
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .flex()
                    .rounded(RADIUS_LG)
                    .border_1()
                    .border_color(if editing.is_some() {
                        colors.border_focused
                    } else if r.failed {
                        theme.status().error
                    } else {
                        colors.border
                    })
                    .bg(colors.surface_background);
                let time = div()
                    .flex_none()
                    .text_size(FONT_SIZE_MD)
                    .font_weight(FONT_WEIGHT_BOLD)
                    .child(r.time.clone());
                if let Some((text, date, time_in, invalid, cancel_focus, save_focus)) = editing {
                    let field = |input: Entity<TextInput>, id: &'static str| {
                        let focus = input.read(cx).focus_handle(cx);
                        field_frame(
                            focus.is_focused(window),
                            colors,
                            themes::parla_tokens(&theme).border_control,
                        )
                        .id((id, n))
                        .cursor_text()
                        .on_click(move |_, window, cx| window.focus(&focus, cx))
                        .child(input)
                    };
                    row = row.flex_col().gap(SPACE_4).child(title).child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(SPACE_4)
                            .child(field(text, "scheduled-edit-text").w_full())
                            .child(
                                div()
                                    .flex()
                                    .gap(SPACE_4)
                                    .child(field(date, "scheduled-edit-date").w(LAYOUT_FIELD_XS))
                                    .child(
                                        field(time_in, "scheduled-edit-time").w(SPACE_12 + SPACE_7),
                                    ),
                            )
                            .when(invalid, |d| {
                                d.child(
                                    div()
                                        .text_size(FONT_SIZE_XS)
                                        .text_color(theme.status().error)
                                        .child(t!(cx, "scheduled-edit-error").into_owned()),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap(SPACE_4)
                                    .child(on_activate(
                                        ui_button(
                                            ("scheduled-cancel", n),
                                            t!(cx, "scheduled-cancel").into_owned(),
                                            ButtonKind::Secondary,
                                            true,
                                            colors,
                                        )
                                        .track_focus(&cancel_focus),
                                        cx,
                                        |this, window, cx| {
                                            this.cancel_edit(cx);
                                            window.focus(&this.focus, cx);
                                        },
                                    ))
                                    .child(on_activate(
                                        ui_button(
                                            ("scheduled-save", n),
                                            t!(cx, "scheduled-save").into_owned(),
                                            ButtonKind::Primary,
                                            true,
                                            colors,
                                        )
                                        .track_focus(&save_focus),
                                        cx,
                                        |this, window, cx| {
                                            this.save_edit(cx);
                                            if this.edit.is_none() {
                                                window.focus(&this.focus, cx);
                                            }
                                        },
                                    )),
                            ),
                    );
                } else {
                    let id = r.id.clone();
                    let mut actions = div().flex().flex_wrap().flex_none().gap(SPACE_4);
                    let src = self.session.clone();
                    let add = |actions: gpui::Div,
                               key: &'static str,
                               label: String,
                               f: Handler,
                               cx: &mut Context<Self>| {
                        actions.child(
                            on_activate(
                                ui_button((key, n), label, ButtonKind::Secondary, true, colors),
                                cx,
                                move |_, w, cx| f(w, cx),
                            )
                            .min_h(SPACE_8)
                            .px(SPACE_5)
                            .py(SPACE_1)
                            .text_size(FONT_SIZE_XS),
                        )
                    };
                    if r.failed {
                        let (s1, s2, i1, i2) = (src.clone(), src, id.clone(), id);
                        actions = add(
                            actions,
                            "scheduled-retry",
                            t!(cx, "scheduled-retry").into_owned(),
                            Box::new(move |_, cx| s1.update(cx, |s, cx| s.retry(i1.clone(), cx))),
                            cx,
                        );
                        actions = add(
                            actions,
                            "scheduled-discard",
                            t!(cx, "scheduled-discard").into_owned(),
                            Box::new(move |_, cx| s2.update(cx, |s, cx| s.delete(i2.clone(), cx))),
                            cx,
                        );
                    } else if !r.sending {
                        if !r.local {
                            let (v, i) = (this.clone(), id.clone());
                            actions = add(
                                actions,
                                "scheduled-edit",
                                t!(cx, "scheduled-edit").into_owned(),
                                Box::new(move |w, cx| {
                                    v.update(cx, |v, cx| v.begin_edit(i.clone(), w, cx))
                                }),
                                cx,
                            );
                        }
                        let (s1, s2, i1, i2) = (src.clone(), src, id.clone(), id);
                        actions = add(
                            actions,
                            "scheduled-send-now",
                            t!(cx, "scheduled-send-now").into_owned(),
                            Box::new(move |_, cx| {
                                s1.update(cx, |s, cx| s.send_now(i1.clone(), cx))
                            }),
                            cx,
                        );
                        actions = add(
                            actions,
                            "scheduled-delete",
                            t!(cx, "scheduled-delete").into_owned(),
                            Box::new(move |_, cx| s2.update(cx, |s, cx| s.delete(i2.clone(), cx))),
                            cx,
                        );
                    }
                    row = row
                        .flex_col()
                        .gap(SPACE_3)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(SPACE_5)
                                .child(time)
                                .child(div().flex_1().min_w_0().child(title)),
                        )
                        .child(
                            div()
                                .text_size(FONT_SIZE_MD)
                                .line_clamp(2)
                                .text_ellipsis()
                                .child(r.text.clone()),
                        )
                        .child(actions);
                }
                body = body.child(row);
            }
        }
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.editor_background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .text_size(FONT_SIZE_XL)
                            .font_weight(FONT_WEIGHT_BOLD)
                            .child(t!(cx, "scheduled-title").into_owned()),
                    )
                    .child(crate::controls::icon_button(
                        "scheduled-close",
                        crate::icon::IconName::X,
                        t!(cx, "close-label").into_owned(),
                        colors,
                        {
                            let this = cx.entity();
                            move |_, cx| this.update(cx, |_, cx| cx.emit(ScheduledEvent::Close))
                        },
                    )),
            )
            .child(body)
    }
}
