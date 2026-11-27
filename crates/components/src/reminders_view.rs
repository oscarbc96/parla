//! The per-workspace Reminders view: upcoming, recurring and completed.

use crate::controls::{
    ButtonKind, button as ui_button, check_box, field_frame, focus_style, on_activate,
};
use domain::{
    ConversationId, Member, MessageId, RemindPreset, Reminder, ReminderId, ReminderRepeat,
};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    Subscription, Window, div, prelude::*,
};
use i18n::Translate as _;
use themes::ActiveTheme;
use themes::metrics::{
    FONT_SIZE_2XS, FONT_SIZE_MD, FONT_SIZE_XL, FONT_SIZE_XS, RADIUS_FULL, RADIUS_LG, RADIUS_SM,
    SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_7, SPACE_8,
};

use crate::send_later::SendLaterConfig;
use crate::text_input::TextInput;

/// Key context of the view.
pub const KEY_CONTEXT: &str = "RemindersView";

/// Shrinks a shared button to a row action.
fn compact(b: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
    b.min_h(SPACE_8)
        .px(SPACE_5)
        .py(SPACE_1)
        .text_size(FONT_SIZE_XS)
}

/// What the view needs from the session, so `components` does not depend on `parla`.
pub trait RemindersSource {
    /// `None` while the list loads.
    fn reminders(&self) -> Option<&[Reminder]>;
    fn complete(&mut self, id: ReminderId, cx: &mut App);
    fn delete(&mut self, id: ReminderId, cx: &mut App);
    /// `when` already passed [`RemindersSource::when_ok`].
    fn add(&mut self, text: String, when: String, cx: &mut App);
    /// Whether `when` can be added now.
    fn when_ok(&self, when: &str, _cx: &App) -> bool {
        domain::is_when(when)
    }
    /// Parla keeps the reminders itself: the provider has none.
    fn local(&self) -> bool {
        false
    }
    /// Zone and clock the times are shown in.
    fn time_config(&self, cx: &App) -> SendLaterConfig;
    /// Moves one of Parla's own reminders to `until` (unix seconds).
    fn snooze(&mut self, _id: ReminderId, _until: i64, _cx: &mut App) {}
    /// Changes one of Parla's own reminders. `when` already passed
    /// [`RemindersSource::when_ok`], or is empty to keep the time.
    fn edit(
        &mut self,
        _id: ReminderId,
        _text: String,
        _when: String,
        _repeat: ReminderRepeat,
        _cx: &mut App,
    ) {
    }
    /// When `when` would fall (unix seconds), for the editor's hint.
    fn when_at(&self, _when: &str, _cx: &App) -> Option<i64> {
        None
    }
    /// The person a linked message is from.
    fn member(&self, _u: &domain::UserId) -> Option<&Member> {
        None
    }
    /// The conversation's name as the sidebar shows it.
    fn conversation_label(&self, _c: &ConversationId, _cx: &App) -> String {
        String::new()
    }
}

/// What a reminder about a message shows of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReminderContext {
    pub message: MessageId,
    /// The author's name; empty when the reminder kept none.
    pub author: String,
    /// The conversation, as the sidebar names it.
    pub place: String,
    /// The message's time of day.
    pub time: String,
    pub excerpt: String,
}

/// The label of a snooze or remind preset; the day presets name their time
/// of day in `zone` ("Tomorrow 09:00").
pub fn preset_label(
    p: RemindPreset,
    zone: &jiff::tz::TimeZone,
    clock: settings::Clock,
    cx: &App,
) -> String {
    let l = cx.localizer();
    let time = || {
        p.at(&crate::now(cx).to_zoned_datetime(zone.clone()))
            .and_then(|at| jiff::Timestamp::from_second(at).ok())
            .map(|t| l.time(t, zone, crate::message_row::hour_cycle(clock)))
            .unwrap_or_default()
    };
    match p {
        RemindPreset::In20Minutes => t!(l, "reminders-preset-in-20-minutes"),
        RemindPreset::In1Hour => t!(l, "reminders-preset-in-1-hour"),
        RemindPreset::In3Hours => t!(l, "reminders-preset-in-3-hours"),
        RemindPreset::Tomorrow => t!(l, "reminders-preset-tomorrow", time = time()),
        RemindPreset::NextWeek => t!(l, "reminders-preset-next-week", time = time()),
    }
    .into_owned()
}

fn repeat_label(r: ReminderRepeat, l: &i18n::Localizer) -> String {
    match r {
        ReminderRepeat::Never => t!(l, "reminders-repeat-never"),
        ReminderRepeat::Daily => t!(l, "reminders-repeat-daily"),
        ReminderRepeat::Weekly => t!(l, "reminders-repeat-weekly"),
    }
    .into_owned()
}

/// Unix seconds of `p` from now in the zone of `cfg`.
pub fn preset_at(p: RemindPreset, cfg: &SendLaterConfig, cx: &App) -> Option<i64> {
    p.at(&crate::now(cx).to_zoned_datetime(cfg.zone.clone()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReminderRow {
    pub id: ReminderId,
    pub text: String,
    /// The due time, formatted; empty when the provider gave none.
    pub time: String,
    pub due: Option<i64>,
    /// Open, not recurring and already past its due time (read when the view
    /// renders; no timer re-renders it).
    pub overdue: bool,
    /// The message the reminder is about.
    pub context: Option<ReminderContext>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReminderGroups {
    pub upcoming: Vec<ReminderRow>,
    pub recurring: Vec<ReminderRow>,
    pub completed: Vec<ReminderRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemindersEvent {
    Close,
    /// Show the message a reminder is about.
    OpenMessage(MessageId),
}

pub struct RemindersView<S: RemindersSource + 'static> {
    session: Entity<S>,
    what: Entity<TextInput>,
    when: Entity<TextInput>,
    invalid: bool,
    completed_open: bool,
    focus: FocusHandle,
    /// The Add button's focus, the last stop of the form's Tab order.
    add_focus: FocusHandle,
    /// The reminder whose snooze menu is open.
    snooze_open: Option<ReminderId>,
    /// The menu's first preset, focused when the menu opens.
    snooze_first: FocusHandle,
    /// Each open reminder's Snooze button, so Escape returns to it.
    snooze_buttons: std::collections::HashMap<ReminderId, FocusHandle>,
    /// An outside click just closed the menu (so the press on Snooze does
    /// not open it again).
    snooze_just_closed: bool,
    /// The reminder being edited inline, and its fields.
    editing: Option<ReminderId>,
    edit_what: Entity<TextInput>,
    edit_when: Entity<TextInput>,
    edit_repeat: ReminderRepeat,
    repeat_open: bool,
    repeat_focus: FocusHandle,
    edit_invalid: bool,
    /// The time was a repeating phrase: Repeat sets how it repeats.
    edit_repeat_hint: bool,
    _subs: Vec<Subscription>,
}

impl<S: RemindersSource + 'static> EventEmitter<RemindersEvent> for RemindersView<S> {}

impl<S: RemindersSource + 'static> RemindersView<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        let (what, when) = (cx.new(TextInput::new), cx.new(TextInput::new));
        let (edit_what, edit_when) = (cx.new(TextInput::new), cx.new(TextInput::new));
        let what_hint = t!(cx, "reminders-add-what").into_owned();
        let when_hint = t!(cx, "reminders-add-when").into_owned();
        what.update(cx, |i, _| i.set_placeholder(what_hint));
        when.update(cx, |i, _| i.set_placeholder(when_hint));
        let subs = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.observe(&what, |_, _, cx| cx.notify()),
            cx.observe(&when, |_, _, cx| cx.notify()),
            cx.observe(&edit_what, |_, _, cx| cx.notify()),
            cx.observe(&edit_when, |_, _, cx| cx.notify()),
        ];
        Self {
            session,
            what,
            when,
            invalid: false,
            completed_open: false,
            focus: cx.focus_handle(),
            add_focus: cx.focus_handle().tab_stop(true),
            snooze_open: None,
            snooze_first: cx.focus_handle().tab_stop(true),
            snooze_buttons: std::collections::HashMap::new(),
            snooze_just_closed: false,
            editing: None,
            edit_what,
            edit_when,
            edit_repeat: ReminderRepeat::Never,
            repeat_open: false,
            repeat_focus: cx.focus_handle().tab_stop(true),
            edit_invalid: false,
            edit_repeat_hint: false,
            _subs: subs,
        }
    }

    /// Upcoming (by due time, none last), recurring and completed (by due
    /// time); `None` while the list loads.
    pub fn groups(&self, cx: &App) -> Option<ReminderGroups> {
        let s = self.session.read(cx);
        let all = s.reminders()?;
        let cfg = s.time_config(cx);
        let hc = crate::message_row::hour_cycle(cfg.clock);
        let l = cx.localizer();
        let now = crate::now_secs(cx);
        let row = |r: &Reminder| {
            // An open one shows when it alerts next (a snoozed occurrence).
            let due = if r.completed { r.due } else { r.next_at() };
            ReminderRow {
                id: r.id.clone(),
                text: if r.about_message() {
                    t!(l, "reminders-about-message").into_owned()
                } else {
                    r.text.to_string()
                },
                time: due
                    .and_then(|d| jiff::Timestamp::from_second(d).ok())
                    .map(|t| {
                        t!(
                            l,
                            "reminders-when",
                            date = l.date(t, &cfg.zone),
                            time = l.time(t, &cfg.zone, hc)
                        )
                        .into_owned()
                    })
                    .unwrap_or_default(),
                due,
                overdue: !r.completed && !r.recurring && due.is_some_and(|d| d < now),
                context: r.link.as_ref().map(|k| ReminderContext {
                    message: k.message.clone(),
                    author: k
                        .author
                        .as_ref()
                        .and_then(|u| s.member(u))
                        .and_then(crate::sidebar::best_name)
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    place: s.conversation_label(&k.message.conversation, cx),
                    time: crate::message_row::format_time(&k.message.ts, cfg.clock, &cfg.zone, l),
                    excerpt: k.excerpt.to_string(),
                }),
            }
        };
        // A missing due time sorts after every real one.
        let key = |r: &ReminderRow| (r.due.is_none(), r.due, r.id.clone());
        let mut out = ReminderGroups::default();
        for r in all {
            let target = if r.completed {
                &mut out.completed
            } else if r.recurring {
                &mut out.recurring
            } else {
                &mut out.upcoming
            };
            target.push(row(r));
        }
        for g in [&mut out.upcoming, &mut out.recurring, &mut out.completed] {
            g.sort_by_key(key);
        }
        Some(out)
    }

    pub fn complete(&mut self, id: ReminderId, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| s.complete(id, cx));
    }

    pub fn delete(&mut self, id: ReminderId, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| s.delete(id, cx));
    }

    /// Fills the add row's fields.
    pub fn set_add(&mut self, what: &str, when: &str, cx: &mut Context<Self>) {
        self.what.update(cx, |i, cx| i.set_text(what, cx));
        self.when.update(cx, |i, cx| i.set_text(when, cx));
    }

    /// Adds the reminder in the add row; false (and an error shown) when the
    /// text is blank or `when` is not a time the source accepts.
    pub fn submit(&mut self, cx: &mut Context<Self>) -> bool {
        let what = self.what.read(cx).text().trim().to_string();
        let when = self.when.read(cx).text().trim().to_string();
        if what.is_empty() || !self.session.read(cx).when_ok(&when, cx) {
            self.invalid = true;
            cx.notify();
            return false;
        }
        self.invalid = false;
        self.set_add("", "", cx);
        self.session.update(cx, |s, cx| s.add(what, when, cx));
        cx.notify();
        true
    }

    /// Whether the add row shows its error.
    pub fn add_invalid(&self) -> bool {
        self.invalid
    }

    pub fn completed_open(&self) -> bool {
        self.completed_open
    }

    pub fn toggle_completed(&mut self, cx: &mut Context<Self>) {
        self.completed_open = !self.completed_open;
        cx.notify();
    }

    /// Snooze and edit are offered: Parla keeps the reminders itself (a
    /// provider's reminders take neither).
    fn editable(&self, cx: &App) -> bool {
        self.session.read(cx).local()
    }

    pub fn snooze_open(&self) -> Option<&ReminderId> {
        self.snooze_open.as_ref()
    }

    /// Opens (or closes) the snooze menu of `id`.
    pub fn toggle_snooze(&mut self, id: ReminderId, cx: &mut Context<Self>) {
        if !self.editable(cx) {
            return;
        }
        self.snooze_open = if self.snooze_open.as_ref() == Some(&id) {
            None
        } else {
            Some(id)
        };
        cx.notify();
    }

    /// Snoozes `id` to `preset` and closes the menu.
    /// Opens (or closes) the snooze menu of `id` from its button: open, the
    /// first preset takes the focus; closed, the button keeps it.
    fn open_snooze(&mut self, id: ReminderId, window: &mut Window, cx: &mut Context<Self>) {
        if self.snooze_open.as_ref() == Some(&id) {
            self.close_snooze(window, cx);
            return;
        }
        self.toggle_snooze(id, cx);
        if self.snooze_open.is_some() {
            window.focus(&self.snooze_first, cx);
        }
    }

    /// Closes the snooze menu with the focus back on its Snooze button.
    fn close_snooze(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.snooze_open.take() else {
            return;
        };
        match self.snooze_buttons.get(&id) {
            Some(f) => window.focus(f, cx),
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// The editor's Repeat menu is open.
    pub fn repeat_open(&self) -> bool {
        self.repeat_open
    }

    /// The last save was refused for a repeating time ("every monday"):
    /// the editor points to Repeat.
    pub fn edit_repeat_hint(&self) -> bool {
        self.edit_repeat_hint
    }

    pub fn snooze(&mut self, id: ReminderId, preset: RemindPreset, cx: &mut Context<Self>) {
        if !self.editable(cx) {
            return;
        }
        self.snooze_open = None;
        let cfg = self.session.read(cx).time_config(cx);
        if let Some(until) = preset_at(preset, &cfg, cx) {
            self.session.update(cx, |s, cx| s.snooze(id, until, cx));
        }
        cx.notify();
    }

    pub fn editing(&self) -> Option<&ReminderId> {
        self.editing.as_ref()
    }

    /// Opens the inline editor of `id`, its text filled in and the time
    /// blank (kept unless a new one is written).
    pub fn begin_edit(&mut self, id: ReminderId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable(cx) {
            return;
        }
        let Some(r) = self
            .session
            .read(cx)
            .reminders()
            .and_then(|all| all.iter().find(|r| r.id == id))
            .cloned()
        else {
            return;
        };
        self.snooze_open = None;
        self.edit_repeat = ReminderRepeat::of(&r);
        self.repeat_open = false;
        self.edit_invalid = false;
        self.edit_repeat_hint = false;
        // A reminder about a message starts from its label.
        let text = if r.about_message() {
            t!(cx, "reminders-about-message").into_owned()
        } else {
            r.text.to_string()
        };
        self.edit_what.update(cx, |i, cx| i.set_text(&text, cx));
        self.edit_when.update(cx, |i, cx| i.set_text("", cx));
        self.editing = Some(id);
        let f = self.edit_what.read(cx).focus_handle(cx);
        window.focus(&f, cx);
        cx.notify();
    }

    /// The editor's text field.
    pub fn edit_text(&self, cx: &App) -> String {
        self.edit_what.read(cx).text().to_string()
    }

    /// Fills the editor's text and time.
    pub fn set_edit(&mut self, what: &str, when: &str, cx: &mut Context<Self>) {
        self.edit_what.update(cx, |i, cx| i.set_text(what, cx));
        self.edit_when.update(cx, |i, cx| i.set_text(when, cx));
    }

    pub fn set_edit_repeat(&mut self, repeat: ReminderRepeat, cx: &mut Context<Self>) {
        self.edit_repeat = repeat;
        self.repeat_open = false;
        cx.notify();
    }

    pub fn edit_invalid(&self) -> bool {
        self.edit_invalid
    }

    /// Saves the editor; false (and an error shown) when the text is blank
    /// or the time is not one the source accepts.
    pub fn save_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = self.editing.clone() else {
            return false;
        };
        let what = self.edit_what.read(cx).text().trim().to_string();
        let when = self.edit_when.read(cx).text().trim().to_string();
        // Repeat decides how it repeats; a repeating time is refused with a
        // pointer to it rather than read two ways.
        if domain::is_repeating_when(&when) {
            self.edit_repeat_hint = true;
            self.edit_invalid = false;
            cx.notify();
            return false;
        }
        let when_ok = when.is_empty() || self.session.read(cx).when_ok(&when, cx);
        if what.is_empty() || !when_ok {
            self.edit_invalid = true;
            self.edit_repeat_hint = false;
            cx.notify();
            return false;
        }
        let repeat = self.edit_repeat;
        // Unchanged, a reminder about a message keeps no text of its own.
        let label = t!(cx, "reminders-about-message").into_owned();
        let about = self
            .session
            .read(cx)
            .reminders()
            .and_then(|all| all.iter().find(|r| r.id == id))
            .is_some_and(Reminder::about_message);
        let what = if about && what == label {
            String::new()
        } else {
            what
        };
        self.editing = None;
        self.edit_invalid = false;
        self.edit_repeat_hint = false;
        self.session
            .update(cx, |s, cx| s.edit(id, what, when, repeat, cx));
        cx.notify();
        true
    }

    pub fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        self.edit_invalid = false;
        self.edit_repeat_hint = false;
        self.repeat_open = false;
        cx.notify();
    }

    /// Shows the message reminder `id` is about.
    pub fn open_link(&mut self, id: ReminderId, cx: &mut Context<Self>) {
        let target = self
            .session
            .read(cx)
            .reminders()
            .and_then(|all| all.iter().find(|r| r.id == id))
            .and_then(|r| r.link.as_ref())
            .map(|k| k.message.clone());
        if let Some(m) = target {
            cx.emit(RemindersEvent::OpenMessage(m));
        }
    }

    fn in_editor(&self, window: &Window, cx: &App) -> bool {
        self.editing.is_some()
            && (self.edit_what.read(cx).focus_handle(cx).is_focused(window)
                || self.edit_when.read(cx).focus_handle(cx).is_focused(window))
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        match ev.keystroke.key.as_str() {
            "tab" if self.edit_tab(m.shift, window, cx) => {}
            "tab" => self.tab(m.shift, window, cx),
            "escape" if self.snooze_open.is_some() => self.close_snooze(window, cx),
            "escape" if self.repeat_open => {
                self.repeat_open = false;
                window.focus(&self.repeat_focus, cx);
                cx.notify();
            }
            "escape" if self.editing.is_some() => {
                self.cancel_edit(cx);
                window.focus(&self.focus, cx);
            }
            "escape" => cx.emit(RemindersEvent::Close),
            "enter" if !m.shift && self.in_editor(window, cx) => {
                if self.save_edit(cx) {
                    window.focus(&self.focus, cx);
                }
            }
            "enter" if !m.shift => {
                self.submit(cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Inside the editor Tab goes text, time, Repeat (then the buttons, in
    /// page order); false when the focus is not in its fields.
    fn edit_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.editing.is_none() {
            return false;
        }
        let what = self.edit_what.read(cx).focus_handle(cx);
        let when = self.edit_when.read(cx).focus_handle(cx);
        let next = if what.is_focused(window) {
            (!back).then_some(when)
        } else if when.is_focused(window) {
            Some(if back {
                what
            } else {
                self.repeat_focus.clone()
            })
        } else if self.repeat_focus.is_focused(window) && back {
            Some(when)
        } else {
            None
        };
        match next {
            Some(f) => {
                window.focus(&f, cx);
                true
            }
            None => false,
        }
    }

    /// Tab order: the text, the time, Add, then the rows' stops; Shift-Tab
    /// from the text leaves the view backwards.
    fn tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let order = [
            self.what.read(cx).focus_handle(cx),
            self.when.read(cx).focus_handle(cx),
            self.add_focus.clone(),
        ];
        let Some(i) = order.iter().position(|h| h.is_focused(window)) else {
            if self.focus.is_focused(window) {
                // Nothing inside has the focus yet: enter the form.
                window.focus(&order[if back { 2 } else { 0 }], cx);
            } else if back {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
            return;
        };
        match (i, back) {
            (2, false) => window.focus_next(cx),
            (0, true) => window.focus_prev(cx),
            (i, false) => window.focus(&order[i + 1], cx),
            (i, true) => window.focus(&order[i - 1], cx),
        }
    }
}

/// The linked message of a reminder: where and when it was posted, and the
/// start of its text, on the sunken ground (the Reminders board). A tab stop
/// with the focus ring; the caller makes it open the message.
fn context_box<V: 'static>(
    c: &ReminderContext,
    id: impl Into<gpui::ElementId>,
    linked: bool,
    theme: &themes::Theme,
    cx: &mut Context<V>,
) -> gpui::Stateful<gpui::Div> {
    let colors = theme.colors();
    let tokens = themes::parla_tokens(theme);
    let l = i18n::gpui::localizer(cx);
    let head = if linked {
        t!(l, "reminders-linked-message", place = c.place.clone())
    } else if c.author.is_empty() {
        std::borrow::Cow::Owned(c.place.clone())
    } else {
        t!(
            l,
            "reminders-context",
            author = c.author.clone(),
            place = c.place.clone()
        )
    };
    let head = if c.time.is_empty() {
        head.into_owned()
    } else {
        format!("{head} \u{b7} {}", c.time)
    };
    div()
        .id(id)
        .debug_selector(|| "reminders-context".to_string())
        .w_full()
        .flex()
        .flex_col()
        .gap(SPACE_1)
        .px(SPACE_5)
        .py(SPACE_4)
        .rounded(themes::metrics::RADIUS_MD)
        .border_1()
        .border_color(gpui::transparent_black())
        .bg(tokens.bg_sunken)
        .cursor_pointer()
        .tab_stop(true)
        .focusable()
        .focus_visible(focus_style(colors))
        .tooltip(crate::reactions::tooltip_view(
            t!(l, "reminders-open-message").into_owned(),
        ))
        .child(
            div()
                .text_size(FONT_SIZE_XS)
                .text_color(tokens.text_subtle)
                .child(head),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(themes::metrics::FONT_SIZE_SM)
                .text_color(colors.text_muted)
                .child(c.excerpt.clone()),
        )
}

impl<S: RemindersSource + 'static> RemindersView<S> {
    /// The inline editor of the reminder being edited (the InlineEditors
    /// board): text, When with the time it reads as, Repeat, the linked
    /// message, then Delete, Cancel and Save.
    fn edit_card(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let id = self.editing.clone()?;
        let row = self
            .groups(cx)
            .into_iter()
            .flat_map(|g| g.upcoming.into_iter().chain(g.recurring).chain(g.completed))
            .find(|r| r.id == id)?;
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let l = i18n::gpui::localizer(cx);
        let label = |text: String| {
            div()
                .text_size(FONT_SIZE_XS)
                .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(colors.text_muted)
                .child(text)
        };
        let field = |input: &Entity<TextInput>, id: &'static str, cx: &App| {
            let focus = input.read(cx).focus_handle(cx);
            let focused = focus.is_focused(window);
            field_frame(focused, colors, tokens.border_control)
                .id(id)
                .w_full()
                .cursor_text()
                .on_click(move |_, window, cx| window.focus(&focus, cx))
                .child(input.clone())
        };
        // Under When: the time the phrase reads as, or which time a blank keeps.
        let when = self.edit_when.read(cx).text().trim().to_string();
        let cfg = self.session.read(cx).time_config(cx);
        let hint = if when.is_empty() {
            (!row.time.is_empty()).then(|| {
                (
                    t!(l, "reminders-edit-when-keep", when = row.time.clone()).into_owned(),
                    tokens.text_subtle,
                )
            })
        } else {
            self.session
                .read(cx)
                .when_at(&when, cx)
                .and_then(|at| jiff::Timestamp::from_second(at).ok())
                .map(|t| {
                    let hc = crate::message_row::hour_cycle(cfg.clock);
                    (
                        t!(
                            l,
                            "reminders-when",
                            date = l.date(t, &cfg.zone),
                            time = l.time(t, &cfg.zone, hc)
                        )
                        .into_owned(),
                        theme.status().success,
                    )
                })
        };
        let repeat_button = on_activate(
            div()
                .id("reminders-edit-repeat")
                .debug_selector(|| "reminders-edit-repeat".to_string())
                .track_focus(&self.repeat_focus)
                .w_full()
                .min_h(themes::metrics::SPACE_10)
                .px(SPACE_5)
                .flex()
                .items_center()
                .justify_between()
                .rounded(themes::metrics::RADIUS_MD)
                .border_1()
                .border_color(tokens.border_control)
                .bg(colors.surface_background)
                .text_size(FONT_SIZE_MD)
                .cursor_pointer()
                .focus_visible(focus_style(colors))
                .child(repeat_label(self.edit_repeat, &l))
                .child(
                    crate::icon::Icon::new(crate::icon::IconName::ChevDown)
                        .size(themes::metrics::ICON_SIZE_SM)
                        .color(colors.text_muted),
                ),
            cx,
            |this, _, cx| {
                this.repeat_open = !this.repeat_open;
                cx.notify();
            },
        );
        let repeat_menu = self.repeat_open.then(|| {
            let mut m = div()
                .mt(SPACE_2)
                .p(SPACE_2)
                .flex()
                .flex_col()
                .rounded(themes::metrics::RADIUS_MD)
                .border_1()
                .border_color(colors.border)
                .bg(colors.elevated_surface_background)
                .text_size(themes::metrics::FONT_SIZE_SM);
            for (i, r) in ReminderRepeat::ALL.into_iter().enumerate() {
                let hover = colors.element_hover;
                let on = r == self.edit_repeat;
                m = m.child(on_activate(
                    div()
                        .id(("reminders-repeat-option", i))
                        .debug_selector(move || format!("reminders-repeat-option-{i}"))
                        .h(themes::metrics::LAYOUT_MENU_ROW)
                        .px(SPACE_4)
                        .flex()
                        .items_center()
                        .rounded(RADIUS_SM)
                        .cursor_pointer()
                        .when(on, |d| d.bg(colors.element_selected))
                        .hover(move |d| d.bg(hover))
                        .tab_stop(true)
                        .focusable()
                        .focus_visible(focus_style(colors))
                        .child(repeat_label(r, &l)),
                    cx,
                    move |this, w, cx| {
                        this.set_edit_repeat(r, cx);
                        w.focus(&this.repeat_focus, cx);
                    },
                ));
            }
            m
        });
        let context = row.context.clone().map(|c| {
            let open = id.clone();
            let b = context_box(&c, "reminders-edit-context", true, &theme, cx);
            on_activate(b, cx, move |this, _, cx| this.open_link(open.clone(), cx))
        });
        let del = id.clone();
        let card = div()
            .id("reminders-edit")
            .debug_selector(|| "reminders-edit".to_string())
            .mb(SPACE_4)
            .p(SPACE_6)
            .flex()
            .flex_col()
            .gap(SPACE_5)
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(colors.border_focused)
            .bg(colors.surface_background)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2)
                    .child(label(t!(l, "reminders-edit-what").into_owned()))
                    .child(field(&self.edit_what, "reminders-edit-what", cx)),
            )
            .child(
                div()
                    .flex()
                    .gap(SPACE_5)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(SPACE_2)
                            .child(label(t!(l, "reminders-edit-when").into_owned()))
                            .child(field(&self.edit_when, "reminders-edit-when", cx))
                            .when_some(hint, |d, (text, color)| {
                                d.child(
                                    div()
                                        .id("reminders-edit-hint")
                                        .text_size(FONT_SIZE_XS)
                                        .text_color(color)
                                        .child(text),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(SPACE_2)
                            .child(label(t!(l, "reminders-edit-repeat").into_owned()))
                            .child(repeat_button)
                            .when_some(repeat_menu, |d, m| d.child(m)),
                    ),
            )
            .when_some(context, |d, c| d.child(c))
            .when(self.edit_invalid, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme.status().error)
                        .child(t!(l, "reminders-edit-error").into_owned()),
                )
            })
            .when(self.edit_repeat_hint, |d| {
                d.child(
                    div()
                        .id("reminders-edit-repeat-hint")
                        .text_xs()
                        .text_color(theme.status().error)
                        .child(t!(l, "reminders-edit-repeat-hint").into_owned()),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .child(compact(on_activate(
                        ui_button(
                            "reminders-edit-delete",
                            t!(l, "reminders-delete").into_owned(),
                            ButtonKind::Secondary,
                            true,
                            colors,
                        )
                        .text_color(theme.status().error),
                        cx,
                        move |this, w, cx| {
                            this.cancel_edit(cx);
                            this.delete(del.clone(), cx);
                            w.focus(&this.focus, cx);
                        },
                    )))
                    .child(div().flex_1())
                    .child(compact(on_activate(
                        ui_button(
                            "reminders-edit-cancel",
                            t!(l, "reminders-edit-cancel").into_owned(),
                            ButtonKind::Secondary,
                            true,
                            colors,
                        ),
                        cx,
                        |this, w, cx| {
                            this.cancel_edit(cx);
                            w.focus(&this.focus, cx);
                        },
                    )))
                    .child(compact(on_activate(
                        ui_button(
                            "reminders-edit-save",
                            t!(l, "reminders-edit-save").into_owned(),
                            ButtonKind::Primary,
                            true,
                            colors,
                        )
                        .debug_selector(|| "reminders-edit-save".to_string()),
                        cx,
                        |this, w, cx| {
                            if this.save_edit(cx) {
                                w.focus(&this.focus, cx);
                            }
                        },
                    ))),
            );
        Some(card.into_any_element())
    }
}

/// The row's "Snooze" button (clock, label, chevron, as on the Reminders
/// board); a press or Enter opens its menu with the first preset focused, and
/// closes it again.
fn snooze_button<S: RemindersSource + 'static>(
    n: usize,
    id: ReminderId,
    open: bool,
    focus: Option<&FocusHandle>,
    colors: &themes::ThemeColors,
    cx: &mut Context<RemindersView<S>>,
) -> gpui::Stateful<gpui::Div> {
    let muted = colors.text_muted;
    let icon = |name| {
        crate::icon::Icon::new(name)
            .size(themes::metrics::ICON_SIZE_SM)
            .color(muted)
    };
    let key_id = id.clone();
    compact(crate::controls::button_frame(
        ("reminders-snooze", n),
        ButtonKind::Secondary,
        true,
        colors,
    ))
    .debug_selector(move || format!("reminders-snooze-{n}"))
    .gap(SPACE_2)
    .when(open, |d| d.bg(colors.element_selected))
    .when_some(focus, |d, f| d.track_focus(f))
    .child(icon(crate::icon::IconName::Clock))
    .child(t!(cx, "reminders-snooze").into_owned())
    .child(icon(crate::icon::IconName::ChevDown))
    // On the press, as the composer's emoji button: the outside press that
    // closed the menu must not open it again.
    .on_mouse_down(
        gpui::MouseButton::Left,
        cx.listener(move |this, _, w, cx| {
            if !std::mem::take(&mut this.snooze_just_closed) {
                this.open_snooze(id.clone(), w, cx);
            }
        }),
    )
    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, w, cx| {
        let k = &ev.keystroke;
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
            return;
        }
        if matches!(k.key.as_str(), "enter" | "space") {
            if !k.modifiers.shift && !ev.is_held {
                this.open_snooze(key_id.clone(), w, cx);
            }
            cx.stop_propagation();
        }
    }))
}

/// The snooze presets of reminder `id`: a column menu on the elevated ground,
/// anchored by its top right corner under the Snooze button and kept inside
/// the window. A press outside closes it.
fn snooze_menu<S: RemindersSource + 'static>(
    n: usize,
    id: ReminderId,
    first: &FocusHandle,
    cfg: &SendLaterConfig,
    colors: &themes::ThemeColors,
    cx: &mut Context<RemindersView<S>>,
) -> gpui::Deferred {
    let mut menu = div()
        .id(("reminders-snooze-menu", n))
        .debug_selector(|| "reminders-snooze-menu".to_string())
        .mt(SPACE_2)
        .p(SPACE_2)
        .min_w(themes::metrics::LAYOUT_FIELD_MD)
        .flex()
        .flex_col()
        .rounded(themes::metrics::RADIUS_MD)
        .border_1()
        .border_color(colors.border)
        .bg(colors.elevated_surface_background)
        .shadow_md()
        .text_size(themes::metrics::FONT_SIZE_SM)
        .text_color(colors.text)
        .on_mouse_down_out(cx.listener(|this, _, window, cx| {
            if this.snooze_open.take().is_some() {
                this.snooze_just_closed = true;
                let me = cx.entity();
                // The press reaches the Snooze button's handler before this runs.
                window.defer(cx, move |_, cx| {
                    me.update(cx, |v, _| v.snooze_just_closed = false)
                });
                cx.notify();
            }
        }));
    for (i, p) in RemindPreset::ALL.into_iter().enumerate() {
        let target = id.clone();
        let hover = colors.element_hover;
        let mut item = div()
            .id(("reminders-snooze-preset", n * 8 + i))
            .debug_selector(move || format!("reminders-snooze-preset-{i}"))
            .h(themes::metrics::LAYOUT_MENU_ROW)
            .px(SPACE_5)
            .flex()
            .items_center()
            .rounded(RADIUS_SM)
            .cursor_pointer()
            .hover(move |d| d.bg(hover))
            .focus_visible(focus_style(colors))
            .child(preset_label(p, &cfg.zone, cfg.clock, cx));
        item = if i == 0 {
            item.track_focus(first)
        } else {
            item.tab_stop(true).focusable()
        };
        menu = menu.child(on_activate(item, cx, move |this, w, cx| {
            this.snooze(target.clone(), p, cx);
            w.focus(&this.focus, cx);
        }));
    }
    gpui::deferred(
        gpui::anchored()
            .anchor(gpui::Anchor::TopRight)
            .snap_to_window_with_margin(SPACE_2)
            .child(menu),
    )
}

impl<S: RemindersSource + 'static> Focusable for RemindersView<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: RemindersSource + 'static> Render for RemindersView<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let groups = self.groups(cx);
        let local = self.session.read(cx).local();
        let editing = self.editing.clone();
        let snooze_open = self.snooze_open.clone();
        let snooze_first = self.snooze_first.clone();
        // One focus handle per open local reminder's Snooze button, held only
        // while the reminder is listed.
        if let Some(all) = self.session.read(cx).reminders() {
            let open: std::collections::HashSet<&ReminderId> =
                all.iter().filter(|r| !r.completed).map(|r| &r.id).collect();
            self.snooze_buttons.retain(|k, _| open.contains(k));
            for id in open {
                if !self.snooze_buttons.contains_key(id) {
                    self.snooze_buttons
                        .insert(id.clone(), cx.focus_handle().tab_stop(true));
                }
            }
        }
        let snooze_buttons = self.snooze_buttons.clone();
        let cfg = self.session.read(cx).time_config(cx);
        let mut edit_card = self.edit_card(window, cx);
        let mut body = div()
            .id("reminders-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(SPACE_6)
            .pb(SPACE_6);
        // Section caption: small caps, with the count; `tone` colours it.
        let caption = |label: String, count: usize, tone: gpui::Hsla| {
            div()
                .mt(SPACE_7)
                .mb(SPACE_4)
                .text_size(FONT_SIZE_2XS)
                .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(tone)
                .child(format!("{} \u{b7} {count}", label.to_uppercase()))
        };
        match groups {
            None => {
                body = body.child(
                    div()
                        .id("reminders-loading")
                        .py(SPACE_7)
                        .text_sm()
                        .text_color(colors.text_muted)
                        .child(t!(cx, "reminders-loading").into_owned()),
                );
            }
            Some(g) => {
                let empty =
                    g.upcoming.is_empty() && g.recurring.is_empty() && g.completed.is_empty();
                if empty {
                    body = body.child(
                        crate::controls::empty_state(
                            t!(cx, "reminders-empty").into_owned(),
                            t!(cx, "reminders-empty-hint").into_owned(),
                            colors,
                        )
                        .id("reminders-empty"),
                    );
                }
                let mut n = 0usize;
                let mut section = |body: gpui::Stateful<gpui::Div>,
                                   head: Option<gpui::Div>,
                                   rows: Vec<ReminderRow>,
                                   done: bool,
                                   cx: &mut Context<Self>| {
                    let mut body = body.when_some(head, |b, h| b.child(h));
                    for r in rows {
                        n += 1;
                        if editing.as_ref() == Some(&r.id)
                            && let Some(card) = edit_card.take()
                        {
                            body = body.child(card);
                            continue;
                        }
                        let (i1, i2) = (r.id.clone(), r.id.clone());
                        let mut actions = div().flex().flex_none().items_center().gap(SPACE_2);
                        if local && !done {
                            let edit_id = r.id.clone();
                            let this_e = cx.entity();
                            let open = snooze_open.as_ref() == Some(&r.id);
                            let snooze = snooze_button(
                                n,
                                r.id.clone(),
                                open,
                                snooze_buttons.get(&r.id),
                                colors,
                                cx,
                            );
                            // The presets, a menu anchored under the button's
                            // right edge (the Reminders board).
                            let popover = open.then(|| {
                                snooze_menu(n, r.id.clone(), &snooze_first, &cfg, colors, cx)
                            });
                            actions = actions
                                .child(
                                    // The menu hangs from the button's bottom right corner.
                                    div().relative().child(snooze).when_some(popover, |d, p| {
                                        d.child(div().absolute().right_0().top(SPACE_8).child(p))
                                    }),
                                )
                                .child(
                                    crate::controls::icon_button(
                                        ("reminders-edit", n),
                                        crate::icon::IconName::Pencil,
                                        t!(cx, "reminders-edit").into_owned(),
                                        colors,
                                        move |w, cx| {
                                            this_e.update(cx, |this, cx| {
                                                this.begin_edit(edit_id.clone(), w, cx)
                                            })
                                        },
                                    )
                                    .debug_selector(move || format!("reminders-edit-{n}")),
                                );
                        }
                        actions = actions.child(compact(on_activate(
                            ui_button(
                                ("reminders-delete", n),
                                t!(cx, "reminders-delete").into_owned(),
                                ButtonKind::Secondary,
                                true,
                                colors,
                            ),
                            cx,
                            move |this, window, cx| {
                                this.delete(i2.clone(), cx);
                                window.focus(&this.focus, cx);
                            },
                        )));
                        // The message the reminder is about: a tab stop that opens it.
                        let context = r.context.clone().map(|c| {
                            let open = r.id.clone();
                            let b = context_box(&c, ("reminders-context", n), false, &theme, cx);
                            on_activate(b, cx, move |this, _, cx| this.open_link(open.clone(), cx))
                        });
                        // The check box completes an open reminder; a done one
                        // shows it ticked.
                        let tick = if done {
                            div()
                                .p(SPACE_1)
                                .border_1()
                                .border_color(gpui::transparent_black())
                                .child(check_box(true, colors, colors.element_background))
                        } else {
                            let b = div()
                                .id(("reminders-complete", n))
                                .debug_selector(move || format!("reminders-complete-{n}"))
                                .rounded(RADIUS_SM)
                                .border_1()
                                .border_color(gpui::transparent_black())
                                // The inset focus ring shows in this padding,
                                // outside the opaque box.
                                .p(SPACE_1)
                                .cursor_pointer()
                                .tooltip(crate::reactions::tooltip_view(
                                    t!(cx, "reminders-mark-complete").into_owned(),
                                ))
                                .tab_stop(true)
                                .focusable()
                                .focus_visible(focus_style(colors))
                                .child(check_box(
                                    false,
                                    colors,
                                    themes::parla_tokens(&theme).border_control,
                                ));
                            div().child(on_activate(b, cx, move |this, window, cx| {
                                this.complete(i1.clone(), cx);
                                window.focus(&this.focus, cx);
                            }))
                        };
                        let id = r.id.0.to_string();
                        let text_color = if done { colors.text_muted } else { colors.text };
                        // The due-time pill: danger when overdue.
                        let pill = (!r.time.is_empty()).then(|| {
                            let (bg, fg) = if r.overdue {
                                (
                                    themes::parla_tokens(&theme).danger_soft,
                                    theme.status().error,
                                )
                            } else {
                                (colors.panel_background, colors.text_muted)
                            };
                            div()
                                .flex_none()
                                .px(SPACE_4)
                                .py(SPACE_1)
                                .rounded(RADIUS_FULL)
                                .bg(bg)
                                .text_color(fg)
                                .text_size(FONT_SIZE_XS)
                                .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                                .child(r.time.clone())
                        });
                        body = body.child(
                            div()
                                .id(("reminders-row", n))
                                .debug_selector(move || format!("reminders-row-{id}"))
                                .mb(SPACE_4)
                                .px(SPACE_6)
                                .py(SPACE_5)
                                .flex()
                                .flex_col()
                                .rounded(RADIUS_LG)
                                .border_1()
                                .border_color(if r.overdue {
                                    theme.status().error
                                } else {
                                    colors.border
                                })
                                .when(!done, |d| d.bg(colors.surface_background))
                                .child(
                                    div()
                                        .w_full()
                                        .flex()
                                        .items_start()
                                        .gap(SPACE_5)
                                        .child(tick)
                                        .child(
                                            div().flex_1().min_w_0().flex().flex_col().child(
                                                div()
                                                    .flex()
                                                    .flex_wrap()
                                                    .items_center()
                                                    .gap_x(SPACE_5)
                                                    .gap_y(SPACE_3)
                                                    .child(
                                                        div()
                                                            .min_w_0()
                                                            .max_w_full()
                                                            .text_size(FONT_SIZE_MD)
                                                            .when(!done, |d| {
                                                                d.font_weight(
                                                        themes::metrics::FONT_WEIGHT_SEMIBOLD,
                                                    )
                                                            })
                                                            .when(done, |d| d.line_through())
                                                            .text_color(text_color)
                                                            .child(r.text),
                                                    )
                                                    .when_some(pill, |d, p| d.child(p)),
                                            ),
                                        )
                                        .child(actions),
                                )
                                // Under the row, the full width: the message it is about.
                                .when_some(context, |d, c| d.child(c.mt(SPACE_4))),
                        );
                    }
                    body
                };
                let (overdue, upcoming): (Vec<_>, Vec<_>) =
                    g.upcoming.into_iter().partition(|r| r.overdue);
                if !overdue.is_empty() {
                    let h = caption(
                        t!(cx, "reminders-overdue").into_owned(),
                        overdue.len(),
                        theme.status().error,
                    );
                    body = section(body, Some(h), overdue, false, cx);
                }
                if !upcoming.is_empty() {
                    let h = caption(
                        t!(cx, "reminders-upcoming").into_owned(),
                        upcoming.len(),
                        colors.text_muted,
                    );
                    body = section(body, Some(h), upcoming, false, cx);
                }
                if !g.recurring.is_empty() {
                    let h = caption(
                        t!(cx, "reminders-recurring").into_owned(),
                        g.recurring.len(),
                        colors.text_muted,
                    );
                    body = section(body, Some(h), g.recurring, false, cx);
                }
                if !g.completed.is_empty() {
                    let count = g.completed.len();
                    let open = self.completed_open;
                    body = body.child(
                        div()
                            .mt(SPACE_7)
                            .mb(SPACE_4)
                            .flex()
                            .child(compact(on_activate(
                                ui_button(
                                    "reminders-completed-toggle",
                                    t!(cx, "reminders-completed", count = count).into_owned(),
                                    ButtonKind::Secondary,
                                    true,
                                    colors,
                                )
                                .debug_selector(|| "reminders-completed-toggle".to_string()),
                                cx,
                                |this, _, cx| this.toggle_completed(cx),
                            ))),
                    );
                    if open {
                        body = section(body, None, g.completed, true, cx);
                    }
                }
            }
        }
        let field = |input: Entity<TextInput>, id: &'static str| {
            let focus = input.read(cx).focus_handle(cx);
            let focused = focus.is_focused(window);
            field_frame(focused, colors, themes::parla_tokens(&theme).border_control)
                .id(id)
                .w_full()
                .cursor_text()
                .on_click(move |_, window, cx| window.focus(&focus, cx))
                .child(input)
        };
        // The add card sits above the list, as in the design.
        let add = div()
            .id("reminders-add")
            .mx(SPACE_6)
            .mb(SPACE_2)
            .p(SPACE_5)
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface_background)
            .child(field(self.what.clone(), "reminders-what"))
            .child(field(self.when.clone(), "reminders-when"))
            .when(self.invalid, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme.status().error)
                        .child(t!(cx, "reminders-add-error").into_owned()),
                )
            })
            .child(
                div().flex().justify_end().child(on_activate(
                    ui_button(
                        "reminders-add-button",
                        t!(cx, "reminders-add").into_owned(),
                        ButtonKind::Primary,
                        true,
                        colors,
                    )
                    .track_focus(&self.add_focus),
                    cx,
                    |this, _, cx| {
                        this.submit(cx);
                    },
                )),
            );
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
                            .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                            .child(t!(cx, "reminders-title").into_owned()),
                    )
                    .child(crate::controls::icon_button(
                        "reminders-close",
                        crate::icon::IconName::X,
                        t!(cx, "close-label").into_owned(),
                        colors,
                        {
                            let this = cx.entity();
                            move |_, cx| this.update(cx, |_, cx| cx.emit(RemindersEvent::Close))
                        },
                    )),
            )
            .when(local, |d| {
                d.child(
                    div()
                        .id("reminders-local-note")
                        .mx(SPACE_6)
                        .mt(SPACE_5)
                        .px(SPACE_5)
                        .py(SPACE_4)
                        .rounded(themes::metrics::RADIUS_MD)
                        .bg(colors.panel_background)
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(t!(cx, "reminders-local-note").into_owned()),
                )
            })
            .child(div().pt(SPACE_6).child(add))
            .child(body)
    }
}
