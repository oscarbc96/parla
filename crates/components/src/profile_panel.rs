//! Right-hand panel showing one person's profile: image, names, pronouns,
//! title, status, local time, presence, do-not-disturb, huddle, contact
//! details and custom fields. For the own profile it also edits the status
//! and flips presence.

use std::sync::Arc;

use domain::{DndStatus, PresenceSetting, Profile, UserId};
use gpui::{
    AnyElement, App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, Render, SharedString, Subscription, Window, div, prelude::*,
};
use i18n::{Localizer, Translate as _};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use settings::Clock;
use themes::ActiveTheme;

use crate::controls::ButtonKind;
use crate::send_later::SendLaterConfig;

/// Key context of the panel.
pub const KEY_CONTEXT: &str = "ProfilePanel";

/// What the panel needs from the session, so `components` does not depend on `parla`.
pub trait ProfileSource {
    /// The loaded profile, if any.
    fn profile(&self, u: &UserId) -> Option<&Profile>;
    /// Whether `u` is active now; `None` when presence is not known.
    fn presence(&self, _u: &UserId) -> Option<bool> {
        None
    }
    fn dnd(&self, _u: &UserId) -> Option<&DndStatus> {
        None
    }
    fn in_huddle(&self, _u: &UserId) -> bool {
        false
    }
    /// The person whose profile is open, if any.
    fn shown_profile(&self) -> Option<&UserId>;
    /// Whether `u` is the signed-in user.
    fn is_own(&self, _u: &UserId) -> bool {
        false
    }
    /// Own presence setting (`None` until set).
    fn own_presence(&self) -> Option<PresenceSetting> {
        None
    }
    /// A direct message with `u` exists, so "Message" can open it.
    fn has_dm(&self, _u: &UserId) -> bool {
        false
    }
    /// The provider can start a direct message with anyone, so "Message" also
    /// shows for people we have none with.
    fn can_open_dm(&self) -> bool {
        false
    }
    /// Opens the direct message with `u`, starting it when there is none and
    /// the provider can.
    fn open_dm(&mut self, _u: &UserId, _cx: &mut App) {}
    fn close_profile(&mut self, _cx: &mut App) {}
    fn set_status(&mut self, _s: Option<domain::Status>, _cx: &mut App) {}
    fn set_presence(&mut self, _p: PresenceSetting, _cx: &mut App) {}
    /// The provider keeps statuses: the status can be edited.
    fn can_set_status(&self) -> bool {
        false
    }
    /// The provider keeps a presence setting.
    fn can_set_presence(&self) -> bool {
        false
    }
    /// The clock the times are shown with.
    fn clock(&self, _cx: &App) -> Clock {
        Clock::H24
    }
}

/// The flipped setting: away becomes auto, anything else becomes away.
pub fn next_presence(current: Option<PresenceSetting>) -> PresenceSetting {
    match current {
        Some(PresenceSetting::Away) => PresenceSetting::Auto,
        _ => PresenceSetting::Away,
    }
}

/// Everything the panel shows, as text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileModel {
    pub name: String,
    pub real_name: Option<String>,
    pub pronouns: Option<String>,
    pub title: Option<String>,
    /// Emoji and text; absent when none is set or it has expired.
    pub status: Option<String>,
    /// "14:05, 1 h behind you".
    pub local_time: Option<String>,
    pub presence: Option<String>,
    /// The same as a flag, for the dot: active or away; `None` when unknown.
    pub presence_active: Option<bool>,
    /// "Notifications paused until 15:30".
    pub dnd: Option<String>,
    pub huddle: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub fields: Vec<(String, String)>,
    pub bot: bool,
    pub deleted: bool,
    pub own: bool,
    pub can_message: bool,
}

/// "1 h", "1 h 30 min" or "45 min" for a positive span in minutes.
fn amount(minutes: i64, l: &Localizer) -> String {
    let (h, m) = (minutes / 60, minutes % 60);
    match (h, m) {
        (0, m) => t!(l, "profile-amount-min", m = m).into_owned(),
        (h, 0) => t!(l, "profile-amount-h", h = h).into_owned(),
        (h, m) => t!(l, "profile-amount-hm", h = h, m = m).into_owned(),
    }
}

/// The person's wall clock and how far it is from `mine`: "14:05, 1 h behind
/// you". `None` when the profile has no known zone.
pub fn local_time_line(
    p: &Profile,
    now: Timestamp,
    mine: &TimeZone,
    clock: Clock,
    l: &Localizer,
) -> Option<String> {
    let tz = TimeZone::get(p.tz.as_deref()?).ok()?;
    let theirs = now.to_zoned_datetime(tz.clone());
    let time = l.time(now, &tz, crate::message_row::hour_cycle(clock));
    let diff =
        (i64::from(theirs.offset().seconds()) - i64::from(mine.to_offset(now).seconds())) / 60;
    let offset = match diff {
        0 => t!(l, "profile-offset-same").into_owned(),
        d if d > 0 => t!(l, "profile-offset-ahead", amount = amount(d, l)).into_owned(),
        d => t!(l, "profile-offset-behind", amount = amount(-d, l)).into_owned(),
    };
    Some(t!(l, "profile-local-time", time = time, offset = offset).into_owned())
}

/// The model of `user`'s profile at `now`, as `mine` sees it.
pub fn profile_model<S: ProfileSource + ?Sized>(
    src: &S,
    user: &UserId,
    now: Timestamp,
    mine: &TimeZone,
    clock: Clock,
    l: &Localizer,
) -> Option<ProfileModel> {
    let p = src.profile(user)?;
    let name = [&p.display_name, &p.real_name]
        .into_iter()
        .find(|n| !n.is_empty())
        .map_or_else(|| t!(l, "author-unknown").into_owned(), |n| n.to_string());
    let own = src.is_own(user);
    let text = |s: &Option<Arc<str>>| s.as_ref().filter(|s| !s.is_empty()).map(|s| s.to_string());
    let status = p
        .status
        .as_ref()
        .filter(|s| domain::status_visible(s, now.as_second()))
        .filter(|s| s.emoji.is_some() || !s.text.is_empty())
        .map(|s| match (&s.emoji, s.text.is_empty()) {
            (Some(e), true) => crate::reactions::glyph(e),
            (Some(e), false) => format!("{} {}", crate::reactions::glyph(e), s.text),
            (None, _) => s.text.to_string(),
        });
    let presence = if own {
        src.own_presence().map(|p| match p {
            PresenceSetting::Away => t!(l, "profile-presence-away").into_owned(),
            PresenceSetting::Auto => t!(l, "profile-presence-active").into_owned(),
        })
    } else {
        src.presence(user).map(|active| {
            if active {
                t!(l, "profile-presence-active").into_owned()
            } else {
                t!(l, "profile-presence-away").into_owned()
            }
        })
    };
    let presence_active = if own {
        src.own_presence().map(|p| p == PresenceSetting::Auto)
    } else {
        src.presence(user)
    };
    let cfg = SendLaterConfig {
        presets: Vec::new(),
        zone: mine.clone(),
        clock,
        server: false,
    };
    let dnd = src
        .dnd(user)
        .and_then(|d| domain::dnd_ends(d, now.as_second(), mine))
        .map(|ends| crate::paused_text(&ends, &cfg, l));
    Some(ProfileModel {
        name,
        real_name: (p.display_name != p.real_name)
            .then(|| text(&Some(p.real_name.clone())))
            .flatten(),
        pronouns: text(&p.pronouns),
        title: text(&p.title),
        status,
        local_time: local_time_line(p, now, mine, clock, l),
        presence,
        presence_active,
        dnd,
        huddle: src
            .in_huddle(user)
            .then(|| t!(l, "profile-huddle").into_owned()),
        email: text(&p.email),
        phone: text(&p.phone),
        fields: p
            .fields
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        bot: p.is_bot,
        deleted: p.deleted,
        own,
        can_message: !own && (src.has_dm(user) || src.can_open_dm()),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfilePanelEvent {
    /// The panel was closed; the host removes it.
    Close,
    /// The own status should be edited (the host opens the dialog).
    EditStatus,
}

pub struct ProfilePanel<S: ProfileSource + 'static> {
    session: Entity<S>,
    focus: FocusHandle,
    _sub: Subscription,
}

impl<S: ProfileSource + 'static> EventEmitter<ProfilePanelEvent> for ProfilePanel<S> {}

impl<S: ProfileSource + 'static> Focusable for ProfilePanel<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: ProfileSource + 'static> ProfilePanel<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            focus: cx.focus_handle(),
            _sub: sub,
        }
    }

    /// What is shown now; `None` while the profile loads.
    pub fn model(&self, cx: &App) -> Option<ProfileModel> {
        let s = self.session.read(cx);
        let user = s.shown_profile()?.clone();
        let mine = crate::time_zone(cx);
        profile_model(s, &user, crate::now(cx), &mine, s.clock(cx), cx.localizer())
    }

    /// Opens the direct message with the person and closes the panel.
    pub fn message(&mut self, cx: &mut Context<Self>) {
        let Some(u) = self.session.read(cx).shown_profile().cloned() else {
            return;
        };
        self.session.update(cx, |s, cx| s.open_dm(&u, cx));
        self.close(cx);
    }

    /// Flips the own presence setting.
    pub fn toggle_presence(&mut self, cx: &mut Context<Self>) {
        let s = self.session.read(cx);
        if !s.can_set_presence() || !s.shown_profile().is_some_and(|u| s.is_own(u)) {
            return;
        }
        let next = next_presence(s.own_presence());
        self.session.update(cx, |s, cx| s.set_presence(next, cx));
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| s.close_profile(cx));
        cx.emit(ProfilePanelEvent::Close);
    }

    /// Asks the host to open the status dialog (own profile only).
    pub fn edit_status(&mut self, cx: &mut Context<Self>) {
        let s = self.session.read(cx);
        if s.can_set_status() && s.shown_profile().is_some_and(|u| s.is_own(u)) {
            cx.emit(ProfilePanelEvent::EditStatus);
        }
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" {
            self.close(cx);
        }
    }
}

/// A labelled row; `copy` adds a copy button.
fn row(
    id: &'static str,
    label: String,
    value: String,
    copy: Option<(String, String)>,
    theme: &themes::Theme,
) -> gpui::Div {
    row_with(id, label, value, None, copy, theme)
}

/// [`row`] with an element before the value (the presence dot).
fn row_with(
    id: &'static str,
    label: String,
    value: String,
    lead: Option<AnyElement>,
    copy: Option<(String, String)>,
    theme: &themes::Theme,
) -> gpui::Div {
    let colors = theme.colors();
    let tokens = themes::parla_tokens(theme);
    let muted = tokens.text_subtle;
    let accent = colors.text_accent;
    let mut line = div()
        .flex()
        .when(lead.is_some(), |d| d.items_center())
        .when(lead.is_none(), |d| d.items_baseline())
        .gap_2()
        .children(lead)
        .child(
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .flex_1()
                .min_w_0()
                .text_size(themes::metrics::FONT_SIZE_MD)
                .child(value),
        );
    if let Some((text, label)) = copy {
        line = line.child(
            div()
                .id(SharedString::from(format!("{id}-copy")))
                .debug_selector(move || format!("{id}-copy"))
                .cursor_pointer()
                .text_xs()
                .text_color(accent)
                .child(label)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                }),
        );
    }
    div()
        .mx(themes::metrics::SPACE_8)
        .py(themes::metrics::SPACE_5)
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(themes::metrics::FONT_SIZE_XS)
                .text_color(muted)
                .child(label),
        )
        .child(line)
}

impl<S: ProfileSource + 'static> Render for ProfilePanel<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let l = i18n::gpui::localizer(cx);
        let model = self.model(cx);
        let image = {
            let s = self.session.read(cx);
            s.shown_profile()
                .and_then(|u| s.profile(u))
                .and_then(|p| p.image.clone())
        };
        let can_status = self.session.read(cx).can_set_status();
        let can_presence = self.session.read(cx).can_set_presence();
        let mut body = div()
            .id("profile-body")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        match model {
            None => {
                body = body.child(
                    div()
                        .id("profile-loading")
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(colors.text_muted)
                        .child(t!(l, "profile-loading").into_owned()),
                );
            }
            Some(m) => {
                let mut head = div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(themes::metrics::SPACE_2)
                    .px(themes::metrics::SPACE_8)
                    .pt(themes::metrics::SPACE_8)
                    .pb(themes::metrics::SPACE_6)
                    .child(crate::workspace_badge(&m.name, image))
                    .child(
                        div()
                            .id("profile-name")
                            .debug_selector(|| "profile-name".into())
                            .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                            .text_size(themes::metrics::FONT_SIZE_2XL)
                            .child(m.name.clone()),
                    );
                let subtitle = [m.real_name.clone(), m.pronouns.clone()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" \u{b7} ");
                if !subtitle.is_empty() {
                    head = head.child(
                        div()
                            .id("profile-subtitle")
                            .text_sm()
                            .text_color(colors.text_muted)
                            .child(subtitle),
                    );
                }
                if let Some(t) = &m.title {
                    head = head.child(
                        div()
                            .id("profile-title")
                            .debug_selector(|| "profile-title".into())
                            .text_sm()
                            .child(t.clone()),
                    );
                }
                if m.bot {
                    head = head.child(
                        div()
                            .id("profile-bot")
                            .text_xs()
                            .text_color(colors.text_muted)
                            .child(t!(l, "profile-bot").into_owned()),
                    );
                }
                if m.deleted {
                    head = head.child(
                        div()
                            .id("profile-deactivated")
                            .text_xs()
                            .text_color(theme.status().error)
                            .child(t!(l, "profile-deactivated").into_owned()),
                    );
                }
                body = body.child(head);
                let mut rows: Vec<gpui::Div> = Vec::new();
                let copy_label = t!(l, "profile-copy").into_owned();
                if let Some(v) = m.status.clone() {
                    rows.push(row(
                        "profile-status",
                        t!(l, "profile-status").into_owned(),
                        v,
                        None,
                        &theme,
                    ));
                }
                if let Some(v) = m.local_time.clone() {
                    rows.push(row(
                        "profile-local-time",
                        t!(l, "profile-local-time-label").into_owned(),
                        v,
                        None,
                        &theme,
                    ));
                }
                if let Some(v) = m.presence.clone() {
                    let tokens = themes::parla_tokens(&theme);
                    let dot = m.presence_active.map(|active| {
                        crate::sidebar::presence_dot(
                            "profile-presence-dot",
                            active,
                            tokens.presence_active,
                            tokens.presence_away,
                        )
                        .into_any_element()
                    });
                    rows.push(row_with(
                        "profile-presence",
                        t!(l, "profile-presence").into_owned(),
                        v,
                        dot,
                        None,
                        &theme,
                    ));
                }
                if let Some(v) = m.dnd.clone() {
                    rows.push(row(
                        "profile-dnd",
                        t!(l, "profile-dnd").into_owned(),
                        v,
                        None,
                        &theme,
                    ));
                }
                if let Some(v) = m.huddle.clone() {
                    rows.push(row(
                        "profile-huddle",
                        t!(l, "profile-huddle-label").into_owned(),
                        v,
                        None,
                        &theme,
                    ));
                }
                if let Some(v) = m.email.clone() {
                    rows.push(row(
                        "profile-email",
                        t!(l, "profile-email").into_owned(),
                        v.clone(),
                        Some((v, copy_label.clone())),
                        &theme,
                    ));
                }
                if let Some(v) = m.phone.clone() {
                    rows.push(row(
                        "profile-phone",
                        t!(l, "profile-phone").into_owned(),
                        v.clone(),
                        Some((v, copy_label)),
                        &theme,
                    ));
                }
                for (k, v) in m.fields.clone() {
                    rows.push(row("profile-field", k, v, None, &theme));
                }
                let mut actions = div()
                    .flex()
                    .gap(themes::metrics::SPACE_4)
                    .px(themes::metrics::SPACE_8)
                    .py(themes::metrics::SPACE_6);
                let button = |id: &'static str, label: String, kind: ButtonKind| {
                    crate::controls::no_focus_on_click(
                        crate::controls::button(id, label, kind, true, &colors)
                            .debug_selector(move || id.to_string())
                            .flex_1(),
                    )
                };
                if m.can_message {
                    actions = actions.child(crate::controls::on_activate(
                        button(
                            "profile-message",
                            t!(l, "profile-message").into_owned(),
                            ButtonKind::Primary,
                        ),
                        cx,
                        |this, _, cx| this.message(cx),
                    ));
                }
                if m.own && can_status {
                    actions = actions.child(crate::controls::on_activate(
                        button(
                            "profile-edit-status",
                            t!(l, "profile-edit-status").into_owned(),
                            ButtonKind::Secondary,
                        ),
                        cx,
                        |this, _, cx| this.edit_status(cx),
                    ));
                }
                if m.own && can_presence {
                    let away = self.session.read(cx).own_presence() == Some(PresenceSetting::Away);
                    let label = if away {
                        t!(l, "profile-set-active")
                    } else {
                        t!(l, "profile-set-away")
                    }
                    .into_owned();
                    actions = actions.child(crate::controls::on_activate(
                        button("profile-toggle-presence", label, ButtonKind::Secondary),
                        cx,
                        |this, _, cx| this.toggle_presence(cx),
                    ));
                }
                body = body.child(actions);
                let border = themes::parla_tokens(&theme).border_subtle;
                let n = rows.len();
                for (ix, r) in rows.into_iter().enumerate() {
                    body = body.child(r.when(ix + 1 < n, |d| d.border_b_1().border_color(border)));
                }
            }
        }
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.surface_background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .flex_none()
                    .h(themes::metrics::LAYOUT_HEADER)
                    .pl(themes::metrics::SPACE_6)
                    .pr(themes::metrics::SPACE_5)
                    .border_b_1()
                    .border_color(themes::parla_tokens(&theme).border_subtle)
                    .child(
                        div()
                            .text_size(themes::metrics::FONT_SIZE_LG)
                            .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                            .child(t!(l, "profile-title").into_owned()),
                    )
                    .child(
                        crate::controls::icon_button(
                            "profile-close",
                            crate::icon::IconName::X,
                            t!(cx, "close-label").into_owned(),
                            &colors,
                            {
                                let this = cx.entity();
                                move |_, cx| this.update(cx, |this, cx| this.close(cx))
                            },
                        )
                        .debug_selector(|| "profile-close".into()),
                    ),
            )
            .child(body)
    }
}
