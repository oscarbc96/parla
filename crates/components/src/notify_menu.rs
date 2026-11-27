//! The notification menus: "Pause notifications" for a workspace and the
//! per-conversation level. One small view, opened by the host at a point
//! (rail context menu, sidebar row context menu, conversation header).

use std::rc::Rc;

use domain::{ConversationId, ConversationPrefs, DndDay, DndEnds, NotifyExtras, NotifyLevel};
use gpui::{
    AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, Render, SharedString, Window, div, prelude::*, px,
};
use i18n::Translate as _;
use jiff::tz::TimeZone;
use themes::{ActiveTheme as _, parla_tokens};

use crate::conversation_menu::{LEVELS, level_label, with_level};
use crate::dnd_menu::{PauseChoice, PauseRow, parse_custom_minutes, pause_rows};
use crate::emoji_picker::{Confirm, Dismiss, KEY_CONTEXT};
use crate::icon::{Icon, IconName};
use crate::send_later::SendLaterConfig;
use crate::text_input::TextInput;

/// What the menus read from and write to a workspace session.
pub trait NotifyControls {
    /// The provider has do-not-disturb: the pause controls are shown.
    fn can_pause(&self) -> bool {
        false
    }
    /// When the user's own pause ends, if notifications are paused now.
    fn paused(&self, _now: i64, _tz: &TimeZone) -> Option<DndEnds> {
        None
    }
    fn pause(&mut self, _minutes: u32, _cx: &mut App) {}
    fn resume(&mut self, _cx: &mut App) {}
    /// The conversation's level and mute flag (the default when none is set).
    fn conversation_prefs(&self, _c: &ConversationId) -> ConversationPrefs {
        ConversationPrefs::default()
    }
    fn set_conversation_prefs(&mut self, _c: ConversationId, _p: ConversationPrefs, _cx: &mut App) {
    }
    /// Parla's own extras for the conversation: thread replies, sound and a
    /// timed mute (the default when none is set).
    fn notify_extras(&self, _c: &ConversationId) -> NotifyExtras {
        NotifyExtras::default()
    }
    fn set_notify_extras(&mut self, _c: ConversationId, _e: NotifyExtras, _cx: &mut App) {}
    /// The zone and clock the pause times are read and shown in.
    fn notify_config(&self, _cx: &App) -> SendLaterConfig {
        SendLaterConfig {
            presets: Vec::new(),
            zone: TimeZone::system(),
            clock: settings::Clock::H24,
            server: false,
        }
    }
}

/// "Paused until 15:30", "... until tomorrow 09:00", or with the date.
pub fn paused_text(ends: &DndEnds, cfg: &SendLaterConfig, l: &i18n::Localizer) -> String {
    let hc = crate::message_row::hour_cycle(cfg.clock);
    let time = l.time(ends.at.timestamp(), &cfg.zone, hc);
    match ends.day {
        DndDay::Today => t!(l, "notify-paused-today", time = time).into_owned(),
        DndDay::Tomorrow => t!(l, "notify-paused-tomorrow", time = time).into_owned(),
        DndDay::On(_) => {
            let when = l.date_time(ends.at.timestamp(), &cfg.zone, hc);
            t!(l, "notify-paused-date", when = when).into_owned()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuKind {
    /// The pause controls (and the host's extra entries).
    Workspace,
    Conversation(ConversationId),
    /// Only the host's entries (details, pop out): the conversation's More menu.
    Actions(ConversationId),
    /// A sidebar row's menu: the host's entries around the notification rows
    /// (Notifications opens the levels, then Mute).
    Row(ConversationId),
}

/// What a host entry does when chosen.
pub type ExtraAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// A keymap binding ("cmd-shift-o") for a menu hint.
pub fn display_binding(binding: &str, mac: bool) -> String {
    let parts: Vec<&str> = binding.split('-').collect();
    let (key, mods) = parts.split_last().map_or(("", &[][..]), |(k, m)| (*k, m));
    let key = if key.chars().count() == 1 {
        key.to_uppercase()
    } else {
        key.to_string()
    };
    let name = |m: &str| -> String {
        match (mac, m) {
            (true, "cmd") => crate::keycap::COMMAND.into(),
            (true, "shift") => crate::keycap::SHIFT.into(),
            (true, "alt") => crate::keycap::OPTION.into(),
            (true, "ctrl") => crate::keycap::CONTROL.into(),
            // Off macOS the keymap's own words, capitalised ("ctrl" is "Ctrl").
            (_, other) => {
                let mut cs = other.chars();
                cs.next()
                    .map(|f| f.to_uppercase().chain(cs).collect())
                    .unwrap_or_default()
            }
        }
    };
    let mods: Vec<String> = mods.iter().map(|m| name(m)).collect();
    if mac {
        format!("{}{key}", mods.concat())
    } else {
        mods.into_iter().chain([key]).collect::<Vec<_>>().join("+")
    }
}

/// An entry the host adds below the notification controls.
#[derive(Clone)]
pub struct MenuExtra {
    pub label: SharedString,
    pub action: ExtraAction,
    /// Drawn before the label (an icon column instead of the tick column).
    pub icon: Option<IconName>,
    /// In a [`MenuKind::Row`] menu: below the notification rows, not above.
    pub after: bool,
    /// The entry's key binding, shown right-aligned (only a real one).
    pub shortcut: Option<SharedString>,
    /// A step that cannot easily be undone (leave, archive): danger text.
    pub danger: bool,
    /// A divider above the entry, to set it apart.
    pub divider_before: bool,
}

impl MenuExtra {
    pub fn new(label: impl Into<SharedString>, action: ExtraAction) -> Self {
        Self {
            label: label.into(),
            action,
            icon: None,
            after: false,
            shortcut: None,
            danger: false,
            divider_before: false,
        }
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn divider_before(mut self) -> Self {
        self.divider_before = true;
        self
    }

    /// The binding as the keymap writes it ("cmd-shift-o"); shown as glyphs
    /// on macOS (⌘⇧O) and as words elsewhere (Ctrl+Shift+O).
    pub fn shortcut(mut self, binding: Option<SharedString>) -> Self {
        self.shortcut = binding.map(|b| display_binding(&b, cfg!(target_os = "macos")).into());
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn after_notifications(mut self) -> Self {
        self.after = true;
        self
    }
}

pub struct NotifyMenu<S: NotifyControls + 'static> {
    session: Entity<S>,
    kind: MenuKind,
    extra: Vec<MenuExtra>,
    custom: Option<Entity<TextInput>>,
    /// A row menu showing the notification levels instead of its entries.
    levels_open: bool,
    focus: FocusHandle,
    _sub: Option<gpui::Subscription>,
    /// Redraws the switches when the session applies a change.
    _observe: gpui::Subscription,
}

/// The length of "Mute for 1 hour".
pub const MUTE_FOR_SECS: i64 = 3600;

impl<S: NotifyControls + 'static> EventEmitter<DismissEvent> for NotifyMenu<S> {}

impl<S: NotifyControls + 'static> Focusable for NotifyMenu<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: NotifyControls + 'static> NotifyMenu<S> {
    pub fn new(
        session: Entity<S>,
        kind: MenuKind,
        extra: Vec<MenuExtra>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&session, |_, _, cx| cx.notify());
        let this = Self {
            session,
            kind,
            extra,
            custom: None,
            levels_open: false,
            focus: cx.focus_handle(),
            _sub: None,
            _observe: observe,
        };
        window.focus(&this.focus, cx);
        this
    }

    pub fn kind(&self) -> &MenuKind {
        &self.kind
    }

    /// The extra entries' labels, in order.
    pub fn extra_labels(&self) -> Vec<SharedString> {
        self.extra.iter().map(|e| e.label.clone()).collect()
    }

    fn rows(&self, cx: &App) -> Vec<PauseRow> {
        let cfg = self.session.read(cx).notify_config(cx);
        let now = crate::now(cx).to_zoned_datetime(cfg.zone);
        pause_rows(&now, cfg.clock, cx.localizer())
    }

    /// The paused state to show, if notifications are paused now.
    pub fn paused(&self, cx: &App) -> Option<DndEnds> {
        let s = self.session.read(cx);
        let cfg = s.notify_config(cx);
        s.paused(crate::now_secs(cx), &cfg.zone)
    }

    /// Chooses row `ix` of the pause menu: pauses, or opens the custom field.
    pub fn choose_pause(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows(cx).into_iter().nth(ix) else {
            return;
        };
        match (row.choice, row.minutes) {
            (PauseChoice::Custom, _) => self.begin_custom(window, cx),
            (_, Some(m)) => {
                self.session.update(cx, |s, cx| s.pause(m, cx));
                cx.emit(DismissEvent);
            }
            _ => {}
        }
    }

    fn begin_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(TextInput::new);
        let l = cx.localizer();
        let ph = t!(l, "notify-pause-custom-placeholder").into_owned();
        input.update(cx, |i, _| i.set_placeholder(ph));
        self._sub = Some(cx.observe(&input, |_, _, cx| cx.notify()));
        let handle = input.read(cx).focus_handle(cx);
        self.custom = Some(input);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// The custom duration typed so far, in minutes, if it is valid.
    pub fn custom_minutes(&self, cx: &App) -> Option<u32> {
        parse_custom_minutes(self.custom.as_ref()?.read(cx).text())
    }

    /// Sets the custom field (a test hook; the field is typed into in the app).
    pub fn set_custom(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(i) = &self.custom {
            i.update(cx, |i, cx| i.set_text(text, cx));
        }
    }

    /// Pauses for the custom duration when it is valid.
    pub fn submit_custom(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.custom_minutes(cx) {
            self.session.update(cx, |s, cx| s.pause(m, cx));
            cx.emit(DismissEvent);
        }
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| s.resume(cx));
        cx.emit(DismissEvent);
    }

    fn prefs(&self, cx: &App) -> Option<(ConversationId, ConversationPrefs)> {
        let (MenuKind::Conversation(c) | MenuKind::Row(c)) = &self.kind else {
            return None;
        };
        Some((c.clone(), self.session.read(cx).conversation_prefs(c)))
    }

    /// Sets the conversation's level (the mute flag is kept).
    pub fn choose_level(&mut self, level: NotifyLevel, cx: &mut Context<Self>) {
        if let Some((c, p)) = self.prefs(cx) {
            let next = with_level(p, level);
            self.session
                .update(cx, |s, cx| s.set_conversation_prefs(c, next, cx));
        }
        cx.emit(DismissEvent);
    }

    /// The conversation's Parla-side extras.
    fn extras(&self, cx: &App) -> Option<(ConversationId, NotifyExtras)> {
        let (MenuKind::Conversation(c) | MenuKind::Row(c)) = &self.kind else {
            return None;
        };
        Some((c.clone(), self.session.read(cx).notify_extras(c)))
    }

    /// Muted now: the flag, or a timed mute that has not run out.
    pub fn muted_now(&self, cx: &App) -> bool {
        let flag = self.prefs(cx).is_some_and(|(_, p)| p.muted);
        let timed = self
            .extras(cx)
            .is_some_and(|(_, e)| e.muted_at(crate::now_secs(cx)));
        flag || timed
    }

    /// Flips the mute (the level is kept): unmutes when muted by the flag or
    /// a timed mute, mutes otherwise.
    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        let was_muted = self.muted_now(cx);
        if let Some((c, p)) = self.prefs(cx) {
            let next = ConversationPrefs {
                muted: !was_muted,
                ..p
            };
            // A timed mute alone leaves the flag as it is: no write then.
            if next.muted != p.muted {
                self.session
                    .update(cx, |s, cx| s.set_conversation_prefs(c, next, cx));
            }
        }
        if was_muted && let Some((c, e)) = self.extras(cx).filter(|(_, e)| e.mute_until.is_some()) {
            let next = NotifyExtras {
                mute_until: None,
                ..e
            };
            self.session
                .update(cx, |s, cx| s.set_notify_extras(c, next, cx));
        }
        cx.emit(DismissEvent);
    }

    /// Mutes the conversation for an hour: a stored deadline, one timer.
    pub fn mute_for_hour(&mut self, cx: &mut Context<Self>) {
        if let Some((c, e)) = self.extras(cx) {
            let next = NotifyExtras {
                mute_until: Some(crate::now_secs(cx) + MUTE_FOR_SECS),
                ..e
            };
            self.session
                .update(cx, |s, cx| s.set_notify_extras(c, next, cx));
        }
        cx.emit(DismissEvent);
    }

    /// Turns replies in followed threads on or off for the conversation.
    pub fn toggle_thread_replies(&mut self, cx: &mut Context<Self>) {
        if let Some((c, e)) = self.extras(cx) {
            let next = NotifyExtras {
                threads_off: !e.threads_off,
                ..e
            };
            self.session
                .update(cx, |s, cx| s.set_notify_extras(c, next, cx));
        }
    }

    /// Turns the conversation's notification sound on or off.
    pub fn toggle_sound(&mut self, cx: &mut Context<Self>) {
        if let Some((c, e)) = self.extras(cx) {
            let next = NotifyExtras {
                sound_off: !e.sound_off,
                ..e
            };
            self.session
                .update(cx, |s, cx| s.set_notify_extras(c, next, cx));
        }
    }

    /// One host entry, with its icon when it has one, and the divider above
    /// it when it asks for one.
    fn extra_rows(&self, ix: usize, e: &MenuExtra, out: &mut Vec<AnyElement>, cx: &Context<Self>) {
        if e.divider_before {
            out.push(
                div()
                    .debug_selector(move || format!("notify-extra-divider-{ix}"))
                    .child(separator(cx))
                    .into_any_element(),
            );
        }
        let id = format!("notify-extra-{ix}");
        let label = e.label.to_string();
        let danger = e.danger.then(|| cx.theme().status().error);
        let row = match e.icon {
            Some(icon) => icon_item_tinted(
                id,
                icon,
                label,
                e.shortcut.as_ref().map(|s| s.to_string()),
                false,
                danger,
                cx,
            ),
            None => item(id, label, false, cx).when_some(danger, |d, c| d.text_color(c)),
        };
        out.push(
            row.on_click(cx.listener(move |this, _, window, cx| this.run_extra(ix, window, cx)))
                .into_any_element(),
        );
    }

    /// A row menu: switches to the notification levels (`true`) or back.
    pub fn show_levels(&mut self, open: bool, cx: &mut Context<Self>) {
        self.levels_open = open;
        cx.notify();
    }

    /// True while a row menu shows the notification levels.
    pub fn levels_open(&self) -> bool {
        self.levels_open
    }

    fn run_extra(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(e) = self.extra.get(ix).cloned() {
            (e.action)(window, cx);
        }
        cx.emit(DismissEvent);
    }
}

/// One selectable row: 34 px, 6 px radius, the hover fill on pointer; the
/// chosen one sits on the soft accent with a check.
fn item(
    id: impl Into<SharedString>,
    label: String,
    checked: bool,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme();
    let colors = theme.colors();
    let tokens = parla_tokens(theme);
    // On the raised ground `element_hover` equals the ground in Parla Dark and
    // `element_selected` equals `accent_soft` in Parla Light, so the hover
    // uses the active fill.
    let hover = colors.element_active;
    let id: SharedString = id.into();
    let sel = id.to_string();
    div()
        .id(id)
        .debug_selector(move || sel)
        .flex()
        .items_center()
        .gap(themes::metrics::SPACE_5)
        .px(themes::metrics::SPACE_4)
        .h(themes::metrics::LAYOUT_ROW_COZY)
        .rounded(themes::metrics::RADIUS_SM)
        .cursor_pointer()
        .when(!checked, |d| d.hover(move |d| d.bg(hover)))
        .when(checked, |d| {
            d.bg(colors.accent_soft)
                .text_color(tokens.accent_soft_text)
                .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
        })
        .child(
            div()
                .w(themes::metrics::SPACE_5)
                .text_color(if checked {
                    tokens.accent_soft_text
                } else {
                    colors.text_accent
                })
                .children(checked.then(|| {
                    Icon::new(IconName::Check)
                        .size(themes::metrics::ICON_SIZE_SM)
                        .color(tokens.accent_soft_text)
                })),
        )
        .child(div().flex_1().child(label))
}

/// An entry with an icon column (the row menu's layout); `trailing` is a
/// muted value at the right, `chevron` marks a drill-in.
fn icon_item(
    id: impl Into<SharedString>,
    icon: IconName,
    label: String,
    trailing: Option<String>,
    chevron: bool,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    icon_item_tinted(id, icon, label, trailing, chevron, None, cx)
}

/// [`icon_item`] with the label and icon in `tint` (danger entries).
pub(crate) fn icon_item_tinted(
    id: impl Into<SharedString>,
    icon: IconName,
    label: String,
    trailing: Option<String>,
    chevron: bool,
    tint: Option<gpui::Hsla>,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let colors = cx.theme().colors();
    let tokens = parla_tokens(cx.theme());
    let hover = colors.element_active;
    let id: SharedString = id.into();
    let sel = id.to_string();
    div()
        .id(id)
        .debug_selector(move || sel)
        .flex()
        .items_center()
        .gap(themes::metrics::SPACE_5)
        .px(themes::metrics::SPACE_4)
        .h(themes::metrics::LAYOUT_ROW_COZY)
        .rounded(themes::metrics::RADIUS_SM)
        .cursor_pointer()
        .hover(move |d| d.bg(hover))
        .when_some(tint, |d, c| d.text_color(c))
        .child(Icon::new(icon).color(tint.unwrap_or(colors.text_muted)))
        .child(div().flex_1().child(label))
        .children(trailing.map(|t| {
            div()
                .text_size(themes::metrics::FONT_SIZE_XS)
                .text_color(tokens.text_subtle)
                .child(crate::keycap::glyph_row(
                    &t.into(),
                    themes::metrics::ICON_SIZE_SM,
                    tokens.text_subtle,
                    cx,
                ))
        }))
        .when(chevron, |d| {
            d.child(
                Icon::new(IconName::ChevRight)
                    .size(themes::metrics::ICON_SIZE_SM)
                    .color(tokens.text_subtle),
            )
        })
}

/// A row with an icon, a label (and a hint under it) and an on/off switch.
fn switch_item(
    id: &'static str,
    icon: IconName,
    label: String,
    hint: Option<String>,
    on: bool,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let colors = cx.theme().colors();
    let tokens = parla_tokens(cx.theme());
    let hover = colors.element_active;
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .flex()
        .items_center()
        .gap(themes::metrics::SPACE_5)
        .px(themes::metrics::SPACE_4)
        .min_h(themes::metrics::LAYOUT_ROW_COZY)
        .py(themes::metrics::SPACE_2)
        .rounded(themes::metrics::RADIUS_SM)
        .cursor_pointer()
        .hover(move |d| d.bg(hover))
        .child(Icon::new(icon).color(colors.text_muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(label)
                .children(hint.map(|h| {
                    div()
                        .text_size(themes::metrics::FONT_SIZE_XS)
                        .text_color(tokens.text_subtle)
                        .child(h)
                })),
        )
        .child(crate::settings_controls::switch(
            format!("{id}-switch"),
            on,
            colors,
        ))
}

/// A small caption over a group of rows.
fn heading(label: String, cx: &App) -> AnyElement {
    let tokens = parla_tokens(cx.theme());
    div()
        .px(themes::metrics::SPACE_4)
        .pt(themes::metrics::SPACE_4)
        .pb(themes::metrics::SPACE_2)
        .text_size(themes::metrics::FONT_SIZE_2XS)
        .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(tokens.text_subtle)
        .child(label.to_uppercase())
        .into_any_element()
}

fn separator(cx: &App) -> AnyElement {
    div()
        .my(themes::metrics::SPACE_2)
        .mx(themes::metrics::SPACE_3)
        .h(px(1.0))
        .bg(parla_tokens(cx.theme()).border_subtle)
        .into_any_element()
}

impl<S: NotifyControls + 'static> Render for NotifyMenu<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let l = cx.localizer();
        let mut out: Vec<AnyElement> = Vec::new();
        match self.kind.clone() {
            MenuKind::Workspace => {
                if self.session.read(cx).can_pause() {
                    if let Some(ends) = self.paused(cx) {
                        let cfg = self.session.read(cx).notify_config(cx);
                        out.push(heading(paused_text(&ends, &cfg, l), cx));
                        out.push(
                            item(
                                "notify-resume",
                                t!(l, "notify-resume").into_owned(),
                                false,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.resume(cx)))
                            .into_any_element(),
                        );
                    } else {
                        out.push(heading(t!(l, "notify-pause-title").into_owned(), cx));
                        for (ix, row) in self.rows(cx).into_iter().enumerate() {
                            out.push(
                                item(format!("notify-pause-{ix}"), row.label, false, cx)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.choose_pause(ix, window, cx)
                                    }))
                                    .into_any_element(),
                            );
                        }
                        if let Some(input) = &self.custom {
                            let ok = self.custom_minutes(cx).is_some();
                            out.push(
                                div()
                                    .px(themes::metrics::SPACE_4)
                                    .py(themes::metrics::SPACE_2)
                                    .flex()
                                    .flex_col()
                                    .gap(themes::metrics::SPACE_2)
                                    .child(
                                        crate::controls::field_frame(
                                            false,
                                            colors,
                                            themes::parla_tokens(cx.theme()).border_control,
                                        )
                                        .debug_selector(|| "notify-custom-field".into())
                                        .when(!ok, |d| d.border_color(cx.theme().status().error))
                                        .child(input.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(themes::metrics::FONT_SIZE_XS)
                                            .text_color(colors.text_muted)
                                            .child(t!(l, "notify-pause-custom-hint").into_owned()),
                                    )
                                    .into_any_element(),
                            );
                        }
                    }
                }
            }
            MenuKind::Actions(_) => {}
            MenuKind::Row(_) if self.levels_open => {
                let prefs = self.prefs(cx).map(|(_, p)| p).unwrap_or_default();
                out.push(
                    icon_item(
                        "notify-back",
                        IconName::ChevLeft,
                        t!(l, "notify-back").into_owned(),
                        None,
                        false,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.show_levels(false, cx)))
                    .into_any_element(),
                );
                out.push(heading(t!(l, "notify-level-title").into_owned(), cx));
                for (ix, level) in LEVELS.into_iter().enumerate() {
                    out.push(
                        item(
                            format!("notify-level-{ix}"),
                            level_label(level, l),
                            prefs.level == level,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.choose_level(level, cx)))
                        .into_any_element(),
                    );
                }
            }
            MenuKind::Row(_) => {
                let prefs = self.prefs(cx).map(|(_, p)| p).unwrap_or_default();
                for (ix, e) in self.extra.iter().enumerate().filter(|(_, e)| !e.after) {
                    self.extra_rows(ix, e, &mut out, cx);
                }
                out.push(
                    icon_item(
                        "notify-levels",
                        IconName::Bell,
                        t!(l, "notify-menu-notifications").into_owned(),
                        Some(level_label(prefs.level, l)),
                        true,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.show_levels(true, cx)))
                    .into_any_element(),
                );
                let (icon, label) = if prefs.muted {
                    (IconName::Bell, t!(l, "notify-unmute").into_owned())
                } else {
                    (IconName::BellOff, t!(l, "notify-mute").into_owned())
                };
                out.push(
                    icon_item("notify-mute", icon, label, None, false, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx)))
                        .into_any_element(),
                );
                if self.extra.iter().any(|e| e.after) {
                    out.push(separator(cx));
                }
                for (ix, e) in self.extra.iter().enumerate().filter(|(_, e)| e.after) {
                    self.extra_rows(ix, e, &mut out, cx);
                }
            }
            MenuKind::Conversation(_) => {
                let prefs = self.prefs(cx).map(|(_, p)| p).unwrap_or_default();
                out.push(heading(t!(l, "notify-level-title").into_owned(), cx));
                for (ix, level) in LEVELS.into_iter().enumerate() {
                    out.push(
                        item(
                            format!("notify-level-{ix}"),
                            level_label(level, l),
                            prefs.level == level,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.choose_level(level, cx)))
                        .into_any_element(),
                    );
                }
                out.push(separator(cx));
                let extras = self.extras(cx).map(|(_, e)| e).unwrap_or_default();
                out.push(
                    switch_item(
                        "notify-threads",
                        IconName::Threads,
                        t!(l, "notify-threads").into_owned(),
                        Some(t!(l, "notify-threads-hint").into_owned()),
                        !extras.threads_off,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_thread_replies(cx)))
                    .into_any_element(),
                );
                out.push(
                    switch_item(
                        "notify-sound",
                        IconName::Volume,
                        t!(l, "notify-sound").into_owned(),
                        None,
                        !extras.sound_off,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sound(cx)))
                    .into_any_element(),
                );
                out.push(separator(cx));
                let muted = self.muted_now(cx);
                if !muted {
                    out.push(
                        icon_item(
                            "notify-mute-hour",
                            IconName::BellOff,
                            t!(l, "notify-mute-hour").into_owned(),
                            None,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.mute_for_hour(cx)))
                        .into_any_element(),
                    );
                }
                let label = if muted {
                    t!(l, "notify-unmute").into_owned()
                } else {
                    t!(l, "notify-mute").into_owned()
                };
                out.push(
                    item("notify-mute", label, muted, cx)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx)))
                        .into_any_element(),
                );
            }
        }
        if !self.extra.is_empty() && !matches!(self.kind, MenuKind::Row(_)) {
            if !out.is_empty() {
                out.push(separator(cx));
            }
            for (ix, e) in self.extra.iter().enumerate() {
                self.extra_rows(ix, e, &mut out, cx);
            }
        }
        div()
            .id("notify-menu")
            .debug_selector(|| "notify-menu".into())
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.submit_custom(cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| {
                // Escape steps back out of the levels first, then closes.
                if this.levels_open {
                    this.show_levels(false, cx);
                } else {
                    cx.emit(DismissEvent);
                }
            }))
            .occlude()
            .w(themes::metrics::LAYOUT_POPOVER_SM)
            .p(themes::metrics::SPACE_3)
            .flex()
            .flex_col()
            .rounded(themes::metrics::RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .text_size(themes::metrics::FONT_SIZE_SM)
            .shadow_md()
            .children(out)
    }
}
