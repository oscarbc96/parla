//! The channel details panel: a right-hand panel with the About, Members and
//! Settings tabs of the open conversation. It reads what the store holds and
//! writes the notification preferences; with channel admin it asks its host to
//! edit the channel or add people (`ChannelDetailsEvent::Edit`, `Invite`).

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use domain::{
    ContentParser, Conversation, ConversationId, ConversationKind, ConversationPrefs, NotifyLevel,
    PlainParser, UserId, WorkspaceState,
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, Render, Subscription, Window, div, prelude::*,
};
use i18n::{Localizer, Translate as _};
use themes::{ActiveTheme as _, parla_tokens};

use crate::controls::{field_frame, icon_button};
use crate::icon::{Icon, IconName};
use crate::notify_menu::NotifyControls;
use crate::text_input::{TextInput, TextInputEvent};

/// Key context of the panel.
pub const KEY_CONTEXT: &str = "ChannelDetails";

/// Rows the members tab draws at most (a channel can hold thousands of people).
const MEMBER_ROWS_MAX: usize = 200;

/// What the panel needs from the session, so `components` does not depend on `parla`.
pub trait ChannelDetailsSource: NotifyControls {
    /// The workspace the panel reads conversations, members, presence and pins from.
    fn details_state(&self) -> &WorkspaceState;
    /// The conversation the panel describes: the one open in the window.
    fn details_conversation(&self) -> Option<&ConversationId>;
    /// The provider's markup parser, so a topic reads as text, not as markup.
    fn details_content(&self) -> Arc<dyn ContentParser> {
        Arc::new(PlainParser)
    }
    /// Whether the open conversation can be edited and people added to it
    /// (a channel the user is in, with a provider that has channel admin).
    fn details_can_admin(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailsTab {
    #[default]
    About,
    Members,
    Settings,
}

/// What the panel tells its host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelDetailsEvent {
    /// Esc or the close button: the host removes the panel.
    Close,
    /// The About tab's Edit: the host opens the edit dialog.
    Edit,
    /// The Members tab's Add people: the host opens the invite dialog.
    Invite,
}

/// One person in the members tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    pub id: UserId,
    /// The name, with " (you)" for the signed-in user.
    pub name: String,
    /// The job title, when their profile is loaded and has one.
    pub title: Option<String>,
    /// Active (`Some(true)`) or away; `None` while presence is unknown.
    pub active: Option<bool>,
    pub bot: bool,
    /// The person's picture (initials while it loads and when there is none).
    pub avatar: Option<domain::ImageRef>,
}

/// The members tab's people.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemberList {
    pub rows: Vec<MemberRow>,
    /// People before the filter (`rows` after it, in the panel).
    pub total: usize,
    /// The provider did not list the members: these are the people who wrote in
    /// the messages Parla holds.
    pub recent: bool,
}

/// The people of a conversation: the listed members, else the authors of the
/// messages held, newest first (the flag says which).
pub fn conversation_people(state: &WorkspaceState, c: &Conversation) -> (Vec<UserId>, bool) {
    if !c.member_ids.is_empty() {
        return (c.member_ids.to_vec(), false);
    }
    if c.kind == ConversationKind::Direct {
        return (c.peer.iter().cloned().collect(), false);
    }
    let mut seen = HashSet::new();
    let people = state
        .windows
        .get(&c.id)
        .map(|w| {
            w.messages()
                .iter()
                .rev()
                .filter_map(|m| m.author.clone())
                .filter(|a| seen.insert(a.clone()))
                .collect()
        })
        .unwrap_or_default();
    (people, true)
}

/// At most `limit` people to show as avatars, other than the signed-in user: the
/// listed members, else the latest authors of the messages held, newest first.
/// The flag says the people are recent authors, not the member list. Stops
/// after `limit`, so a channel of thousands costs the same as one of three.
pub fn recent_people(
    state: &WorkspaceState,
    c: &Conversation,
    limit: usize,
) -> (Vec<UserId>, bool) {
    let me = state.self_user.as_ref();
    let usable = |u: &UserId| Some(u) != me && state.members.get(u).is_some_and(|m| !m.deleted);
    if !c.member_ids.is_empty() {
        let people = c
            .member_ids
            .iter()
            .filter(|u| usable(u))
            .take(limit)
            .cloned();
        return (people.collect(), false);
    }
    if c.kind == ConversationKind::Direct {
        return (
            c.peer.iter().filter(|u| usable(u)).cloned().collect(),
            false,
        );
    }
    let mut out: Vec<UserId> = Vec::with_capacity(limit);
    if let Some(w) = state.windows.get(&c.id) {
        for a in w.messages().iter().rev().filter_map(|m| m.author.as_ref()) {
            if out.len() == limit {
                break;
            }
            if usable(a) && !out.contains(a) {
                out.push(a.clone());
            }
        }
    }
    (out, true)
}

/// The members tab's rows: you first, then active people, then by name.
pub fn member_rows(state: &WorkspaceState, c: &Conversation, l: &Localizer) -> MemberList {
    let (people, recent) = conversation_people(state, c);
    let mut rows: Vec<MemberRow> = people
        .into_iter()
        .filter_map(|id| {
            let m = state.members.get(&id)?;
            if m.deleted {
                return None;
            }
            let base = crate::sidebar::best_name(m)
                .map(|n| n.to_string())
                .unwrap_or_else(|| t!(l, "author-unknown").into_owned());
            let you = state.self_user.as_ref() == Some(&id);
            let name = if you {
                t!(l, "details-you", name = base).into_owned()
            } else {
                base
            };
            Some(MemberRow {
                title: state
                    .profiles
                    .get(&id)
                    .and_then(|p| p.title.as_deref())
                    .filter(|t| !t.is_empty())
                    .map(str::to_string),
                active: state.presence.get(&id).copied(),
                bot: m.is_bot,
                avatar: m.avatar_for(themes::metrics::SPACE_9.as_f32()).cloned(),
                name,
                id,
            })
        })
        .collect();
    let me = state.self_user.clone();
    rows.sort_by(|a, b| {
        let key = |r: &MemberRow| (Some(&r.id) != me.as_ref(), r.active != Some(true));
        key(a)
            .cmp(&key(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    MemberList {
        total: rows.len(),
        rows,
        recent,
    }
}

/// The rows whose name or title contains `query`, ignoring case; a blank query
/// keeps every row.
pub fn filter_members<'a>(rows: &'a [MemberRow], query: &str) -> Vec<&'a MemberRow> {
    let q = query.trim().to_lowercase();
    rows.iter()
        .filter(|r| {
            q.is_empty()
                || r.name.to_lowercase().contains(&q)
                || r.title
                    .as_ref()
                    .is_some_and(|t| t.to_lowercase().contains(&q))
        })
        .collect()
}

/// What the About tab shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailsAbout {
    pub name: String,
    pub kind: ConversationKind,
    /// The topic as text; `None` when there is none.
    pub topic: Option<String>,
    /// The description; `None` when there is none or it is not known.
    pub purpose: Option<String>,
    /// The Edit and Add people buttons show.
    pub can_edit: bool,
    pub members: Option<u32>,
    /// Pinned messages, when the pins are loaded.
    pub pins: Option<usize>,
}

pub struct ChannelDetailsPanel<S: ChannelDetailsSource + 'static> {
    session: Entity<S>,
    tab: DetailsTab,
    query: String,
    /// The conversation the panel describes; a change resets the tab and query.
    current: Option<ConversationId>,
    /// The rows of the members tab for the current query, built once per
    /// session change (one entry, so the cache is bounded).
    rows: std::cell::RefCell<Option<Rc<MemberList>>>,
    input: Entity<TextInput>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl<S: ChannelDetailsSource + 'static> EventEmitter<ChannelDetailsEvent>
    for ChannelDetailsPanel<S>
{
}

impl<S: ChannelDetailsSource + 'static> Focusable for ChannelDetailsPanel<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: ChannelDetailsSource + 'static> ChannelDetailsPanel<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx);
            i.set_single_line(true);
            i
        });
        let on_input = cx.subscribe(&input, |this, input, _: &TextInputEvent, cx| {
            let text = input.read(cx).text().to_string();
            if text != this.query {
                this.query = text;
                *this.rows.borrow_mut() = None;
                cx.notify();
            }
        });
        let on_session = cx.observe(&session, |this, _, cx| {
            this.session_changed(cx);
            cx.notify();
        });
        let current = session.read(cx).details_conversation().cloned();
        Self {
            session,
            tab: DetailsTab::About,
            query: String::new(),
            current,
            rows: std::cell::RefCell::new(None),
            input,
            focus: cx.focus_handle(),
            _subs: vec![on_input, on_session],
        }
    }

    /// The session changed: the members are rebuilt on the next paint, and a
    /// different conversation starts over on About with an empty filter.
    fn session_changed(&mut self, cx: &mut Context<Self>) {
        *self.rows.borrow_mut() = None;
        let now = self.session.read(cx).details_conversation().cloned();
        if now != self.current {
            self.current = now;
            self.tab = DetailsTab::About;
            if !self.query.is_empty() {
                self.query.clear();
                self.input.update(cx, |i, cx| i.set_text("", cx));
            }
        }
    }

    pub fn tab(&self) -> DetailsTab {
        self.tab
    }

    pub fn set_tab(&mut self, tab: DetailsTab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    /// Sets the members filter (the field is typed into in the app).
    pub fn set_query(&mut self, q: &str, cx: &mut Context<Self>) {
        self.query = q.to_string();
        *self.rows.borrow_mut() = None;
        self.input.update(cx, |i, cx| i.set_text(q, cx));
        cx.notify();
    }

    fn conversation<'a>(&self, cx: &'a App) -> Option<(&'a S, &'a Conversation)> {
        let s = self.session.read(cx);
        let c = s
            .details_state()
            .conversations
            .get(s.details_conversation()?)?;
        Some((s, c))
    }

    /// The members tab as it is painted: the rows the filter keeps. Built once
    /// per session change or query change and kept (one entry).
    fn shown(&self, cx: &App) -> Rc<MemberList> {
        if let Some(list) = self.rows.borrow().as_ref() {
            return list.clone();
        }
        let list = match self.conversation(cx) {
            Some((s, c)) => {
                let all = member_rows(s.details_state(), c, cx.localizer());
                MemberList {
                    total: all.rows.len(),
                    recent: all.recent,
                    rows: filter_members(&all.rows, &self.query)
                        .into_iter()
                        .cloned()
                        .collect(),
                }
            }
            None => MemberList::default(),
        };
        let list = Rc::new(list);
        *self.rows.borrow_mut() = Some(list.clone());
        list
    }

    /// The members the filter keeps.
    pub fn visible_members(&self, cx: &App) -> Vec<MemberRow> {
        self.shown(cx).rows.clone()
    }

    /// What the About tab shows; `None` when no conversation is open.
    pub fn about(&self, cx: &App) -> Option<DetailsAbout> {
        let (s, c) = self.conversation(cx)?;
        let st = s.details_state();
        let l = cx.localizer();
        // The header's own title and count: no walk over the members.
        let info = crate::header::header_info(st, c, l);
        let name = match c.kind {
            ConversationKind::Channel => info.title.strip_prefix("# "),
            ConversationKind::PrivateChannel => info.title.strip_prefix("\u{1F512} "),
            _ => None,
        }
        .unwrap_or(&info.title)
        .to_string();
        let topic = s.details_content().parse_text(c.topic.trim()).plain_text();
        Some(DetailsAbout {
            name,
            kind: c.kind,
            topic: Some(topic.trim().to_string()).filter(|t| !t.is_empty()),
            purpose: Some(c.purpose.trim().to_string()).filter(|t| !t.is_empty()),
            can_edit: s.details_can_admin(),
            members: info.member_count,
            pins: st.pins.get(&c.id).map(Vec::len),
        })
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(ChannelDetailsEvent::Close);
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" {
            self.close(cx);
            cx.stop_propagation();
        }
    }

    fn set_prefs(
        &mut self,
        f: impl Fn(ConversationPrefs) -> ConversationPrefs,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.session.read(cx).details_conversation().cloned() else {
            return;
        };
        let p = self.session.read(cx).conversation_prefs(&id);
        let next = f(p);
        self.session
            .update(cx, |s, cx| s.set_conversation_prefs(id, next, cx));
    }
}

/// A grouped card: rows split by hairlines on the surface ground.
fn card(theme: &themes::Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .rounded(themes::metrics::RADIUS_LG)
        .border_1()
        .border_color(theme.colors().border)
        .bg(theme.colors().surface_background)
}

/// A labelled value row of a card.
fn info_row(
    id: &'static str,
    label: String,
    value: String,
    last: bool,
    theme: &themes::Theme,
) -> gpui::Div {
    let tokens = parla_tokens(theme);
    div()
        .flex()
        .flex_col()
        .gap(themes::metrics::SPACE_1)
        .px(themes::metrics::SPACE_6)
        .py(themes::metrics::SPACE_5)
        .when(!last, |d| d.border_b_1().border_color(tokens.border_subtle))
        .child(
            div()
                .text_size(themes::metrics::FONT_SIZE_XS)
                .text_color(tokens.text_subtle)
                .child(label),
        )
        .child(
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .text_size(themes::metrics::FONT_SIZE_MD)
                .child(value),
        )
}

fn kind_label(kind: ConversationKind, l: &Localizer) -> String {
    match kind {
        ConversationKind::Channel => t!(l, "details-kind-channel"),
        ConversationKind::PrivateChannel => t!(l, "details-kind-private"),
        ConversationKind::Group => t!(l, "details-kind-group"),
        ConversationKind::Direct => t!(l, "details-kind-direct"),
    }
    .into_owned()
}

impl<S: ChannelDetailsSource + 'static> ChannelDetailsPanel<S> {
    fn about_body(
        &self,
        about: &DetailsAbout,
        theme: &themes::Theme,
        l: &Localizer,
        cx: &Context<Self>,
    ) -> AnyElement {
        let topic = about
            .topic
            .clone()
            .unwrap_or_else(|| t!(l, "details-no-topic").into_owned());
        let mut rows = vec![
            info_row(
                "details-about-topic",
                t!(l, "details-topic").into_owned(),
                topic,
                false,
                theme,
            ),
            info_row(
                "details-about-kind",
                t!(l, "details-kind").into_owned(),
                kind_label(about.kind, l),
                about.members.is_none() && about.pins.is_none(),
                theme,
            ),
        ];
        // A description belongs to channels; groups and direct messages have none.
        if matches!(
            about.kind,
            ConversationKind::Channel | ConversationKind::PrivateChannel
        ) {
            rows.insert(
                1,
                info_row(
                    "details-about-purpose",
                    t!(l, "details-purpose").into_owned(),
                    about
                        .purpose
                        .clone()
                        .unwrap_or_else(|| t!(l, "details-no-purpose").into_owned()),
                    false,
                    theme,
                ),
            );
        }
        if let Some(n) = about.members {
            rows.push(info_row(
                "details-about-members",
                t!(l, "details-members-label").into_owned(),
                n.to_string(),
                about.pins.is_none(),
                theme,
            ));
        }
        if let Some(n) = about.pins {
            rows.push(info_row(
                "details-about-pins",
                t!(l, "details-pins").into_owned(),
                n.to_string(),
                true,
                theme,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(themes::metrics::SPACE_5)
            .p(themes::metrics::SPACE_7)
            .child(card(theme).children(rows))
            .when(about.can_edit, |d| {
                d.child(
                    div().flex().child(crate::controls::on_activate(
                        crate::controls::button(
                            "details-edit",
                            t!(l, "details-edit").into_owned(),
                            crate::controls::ButtonKind::Secondary,
                            true,
                            theme.colors(),
                        )
                        .debug_selector(|| "details-edit".into()),
                        cx,
                        |_, _, cx| cx.emit(ChannelDetailsEvent::Edit),
                    )),
                )
            })
            .into_any_element()
    }

    fn member_row(&self, r: &MemberRow, theme: &themes::Theme, l: &Localizer) -> AnyElement {
        let colors = theme.colors();
        let tokens = parla_tokens(theme);
        div()
            .debug_selector(|| "details-member".into())
            .flex()
            .items_center()
            .gap(themes::metrics::SPACE_5)
            .h(themes::metrics::LAYOUT_MEMBER_ROW)
            .px(themes::metrics::SPACE_4)
            .rounded(themes::metrics::RADIUS_MD)
            .hover(|d| d.bg(colors.element_hover))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(crate::message_row::avatar_sized(
                        &r.name,
                        theme,
                        themes::metrics::SPACE_9,
                        r.avatar.as_ref(),
                    ))
                    .when_some(r.active, |d, active| {
                        d.child(
                            div()
                                .absolute()
                                .bottom(themes::metrics::SPACE_0)
                                .right(themes::metrics::SPACE_0)
                                .rounded_full()
                                .border_2()
                                .border_color(colors.surface_background)
                                .child(crate::sidebar::presence_dot(
                                    "details-member-presence",
                                    active,
                                    tokens.presence_active,
                                    tokens.presence_away,
                                )),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(themes::metrics::FONT_SIZE_MD)
                            .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                            .child(r.name.clone()),
                    )
                    .when_some(r.title.clone(), |d, t| {
                        d.child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(themes::metrics::FONT_SIZE_XS)
                                .text_color(tokens.text_subtle)
                                .child(t),
                        )
                    }),
            )
            .when(r.bot, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(themes::metrics::FONT_SIZE_XS)
                        .text_color(colors.text_muted)
                        .child(t!(l, "details-bot").into_owned()),
                )
            })
            .into_any_element()
    }

    fn members_body(
        &mut self,
        focused: bool,
        theme: &themes::Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = *theme.colors();
        let tokens = parla_tokens(theme);
        let l = i18n::gpui::localizer(cx);
        let placeholder = t!(&l, "details-members-search").into_owned();
        self.input.update(cx, |i, _| i.set_placeholder(placeholder));
        let list = self.shown(cx);
        let shown = &list.rows;
        let hidden = shown.len().saturating_sub(MEMBER_ROWS_MAX);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(themes::metrics::SPACE_4)
            .p(themes::metrics::SPACE_7)
            .child(
                field_frame(focused, &colors, tokens.border_control)
                    .id("details-members-search")
                    .debug_selector(|| "details-members-search".into())
                    .gap(themes::metrics::SPACE_4)
                    .child(
                        Icon::new(IconName::Search)
                            .size(themes::metrics::ICON_SIZE_MD)
                            .color(tokens.text_subtle),
                    )
                    .child(div().flex_1().min_w_0().child(self.input.clone())),
            );
        if self.session.read(cx).details_can_admin() {
            body = body.child(
                div().flex().child(crate::controls::on_activate(
                    crate::controls::button(
                        "details-add-people",
                        t!(&l, "details-add-people").into_owned(),
                        crate::controls::ButtonKind::Secondary,
                        true,
                        &colors,
                    )
                    .debug_selector(|| "details-add-people".into()),
                    cx,
                    |_, _, cx| cx.emit(ChannelDetailsEvent::Invite),
                )),
            );
        }
        if list.recent && list.total > 0 {
            body = body.child(
                div()
                    .debug_selector(|| "details-members-recent".into())
                    .text_size(themes::metrics::FONT_SIZE_XS)
                    .text_color(tokens.text_subtle)
                    .child(t!(&l, "details-members-recent").into_owned()),
            );
        }
        if shown.is_empty() {
            let text = if list.total == 0 {
                t!(&l, "details-members-none").into_owned()
            } else {
                t!(&l, "details-members-no-match").into_owned()
            };
            body = body.child(
                div()
                    .debug_selector(|| "details-members-empty".into())
                    .text_size(themes::metrics::FONT_SIZE_MD)
                    .text_color(colors.text_muted)
                    .child(text),
            );
        }
        let mut col = div().flex().flex_col();
        for r in shown.iter().take(MEMBER_ROWS_MAX) {
            col = col.child(self.member_row(r, theme, &l));
        }
        body = body.child(col);
        if hidden > 0 {
            body = body.child(
                div()
                    .debug_selector(|| "details-members-more".into())
                    .text_size(themes::metrics::FONT_SIZE_XS)
                    .text_color(tokens.text_subtle)
                    .child(t!(&l, "details-members-more", count = hidden).into_owned()),
            );
        }
        body.into_any_element()
    }

    fn settings_body(&mut self, theme: &themes::Theme, cx: &mut Context<Self>) -> AnyElement {
        let colors = *theme.colors();
        let tokens = parla_tokens(theme);
        let l = i18n::gpui::localizer(cx);
        let prefs = self
            .session
            .read(cx)
            .details_conversation()
            .map(|c| self.session.read(cx).conversation_prefs(c))
            .unwrap_or_default();
        let mut levels = card(theme);
        for (i, level) in crate::conversation_menu::LEVELS.into_iter().enumerate() {
            let last = i + 1 == crate::conversation_menu::LEVELS.len();
            levels = levels.child(check_row(
                format!("details-level-{}", level_key(level)),
                crate::conversation_menu::level_label(level, &l),
                prefs.level == level,
                last,
                &colors,
                tokens.border_subtle,
                cx,
                move |this, _, cx| {
                    this.set_prefs(|p| crate::conversation_menu::with_level(p, level), cx)
                },
            ));
        }
        let mute = card(theme).child(check_row(
            "details-mute".to_string(),
            if prefs.muted {
                t!(&l, "notify-unmute")
            } else {
                t!(&l, "notify-mute")
            }
            .into_owned(),
            prefs.muted,
            true,
            &colors,
            tokens.border_subtle,
            cx,
            |this, _, cx| this.set_prefs(crate::conversation_menu::toggled_mute, cx),
        ));
        div()
            .flex()
            .flex_col()
            .gap(themes::metrics::SPACE_6)
            .p(themes::metrics::SPACE_7)
            .child(
                div()
                    .text_size(themes::metrics::FONT_SIZE_XS)
                    .text_color(tokens.text_subtle)
                    .child(t!(&l, "notify-level-title").into_owned()),
            )
            .child(levels)
            .child(mute)
            .into_any_element()
    }
}

/// A settings row with a check mark when `on`: a tab stop that Enter, Space and
/// a click activate.
#[allow(clippy::too_many_arguments)]
fn check_row<S: ChannelDetailsSource + 'static>(
    id: String,
    label: String,
    on: bool,
    last: bool,
    colors: &themes::ThemeColors,
    separator: gpui::Hsla,
    cx: &mut Context<ChannelDetailsPanel<S>>,
    f: impl Fn(&mut ChannelDetailsPanel<S>, &mut Window, &mut Context<ChannelDetailsPanel<S>>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let hover = colors.element_hover;
    let accent = colors.text_accent;
    let selector = id.clone();
    crate::controls::on_activate(
        div()
            .id(gpui::SharedString::from(id))
            .debug_selector(move || selector.clone())
            .flex()
            .items_center()
            .gap(themes::metrics::SPACE_5)
            .h(themes::metrics::LAYOUT_SETTING_ROW)
            .px(themes::metrics::SPACE_6)
            .cursor_pointer()
            .tab_stop(true)
            .focusable()
            .focus_visible(crate::controls::focus_style(colors))
            .hover(move |d| d.bg(hover))
            .when(!last, |d| d.border_b_1().border_color(separator))
            .text_size(themes::metrics::FONT_SIZE_MD)
            .child(div().flex_1().child(label))
            .child(div().size(themes::metrics::ICON_SIZE_MD).when(on, |d| {
                d.child(
                    Icon::new(IconName::Check)
                        .size(themes::metrics::ICON_SIZE_MD)
                        .color(accent),
                )
            })),
        cx,
        f,
    )
}

fn level_key(level: NotifyLevel) -> &'static str {
    match level {
        NotifyLevel::Default => "default",
        NotifyLevel::All => "all",
        NotifyLevel::Mentions => "mentions",
        NotifyLevel::Nothing => "nothing",
    }
}

impl<S: ChannelDetailsSource + 'static> Render for ChannelDetailsPanel<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let tokens = parla_tokens(&theme);
        let l = i18n::gpui::localizer(cx);
        let about = self.about(cx);
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let count = about.as_ref().and_then(|a| a.members);
        let (kind_icon, name) = match &about {
            Some(a) => (
                match a.kind {
                    ConversationKind::Channel => Some(IconName::Hash),
                    ConversationKind::PrivateChannel => Some(IconName::Lock),
                    _ => None,
                },
                a.name.clone(),
            ),
            None => (None, String::new()),
        };
        let body = match (&about, self.tab) {
            (None, _) => div()
                .p(themes::metrics::SPACE_7)
                .text_size(themes::metrics::FONT_SIZE_MD)
                .text_color(colors.text_muted)
                .child(t!(&l, "details-none").into_owned())
                .into_any_element(),
            (Some(a), DetailsTab::About) => self.about_body(a, &theme, &l, cx),
            (Some(_), DetailsTab::Members) => self.members_body(focused, &theme, cx),
            (Some(_), DetailsTab::Settings) => self.settings_body(&theme, cx),
        };
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .debug_selector(|| "channel-details".into())
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.surface_background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(themes::metrics::SPACE_3)
                    .flex_none()
                    .h(themes::metrics::LAYOUT_HEADER)
                    .pl(themes::metrics::SPACE_7)
                    .pr(themes::metrics::SPACE_5)
                    .border_b_1()
                    .border_color(tokens.border_subtle)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(themes::metrics::SPACE_2)
                            .when_some(kind_icon, |d, i| {
                                d.child(
                                    Icon::new(i)
                                        .size(themes::metrics::ICON_SIZE_MD)
                                        .color(colors.text),
                                )
                            })
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(themes::metrics::FONT_SIZE_LG)
                                    .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                                    .child(if name.is_empty() {
                                        t!(&l, "details-title").into_owned()
                                    } else {
                                        name
                                    }),
                            ),
                    )
                    .child(
                        icon_button(
                            "details-close",
                            IconName::X,
                            t!(&l, "details-close").into_owned(),
                            &colors,
                            {
                                let this = cx.entity();
                                move |_, cx| this.update(cx, |this, cx| this.close(cx))
                            },
                        )
                        .debug_selector(|| "details-close".into()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .px(themes::metrics::SPACE_7)
                    .border_b_1()
                    .border_color(tokens.border_subtle)
                    .child(tab_button(
                        "details-tab-about",
                        t!(&l, "details-tab-about").into_owned(),
                        None,
                        self.tab == DetailsTab::About,
                        DetailsTab::About,
                        &theme,
                        cx,
                    ))
                    .child(tab_button(
                        "details-tab-members",
                        t!(&l, "details-tab-members").into_owned(),
                        count,
                        self.tab == DetailsTab::Members,
                        DetailsTab::Members,
                        &theme,
                        cx,
                    ))
                    .child(tab_button(
                        "details-tab-settings",
                        t!(&l, "details-tab-settings").into_owned(),
                        None,
                        self.tab == DetailsTab::Settings,
                        DetailsTab::Settings,
                        &theme,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("details-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

/// One tab of the strip: underlined in the signal colour while selected.
fn tab_button<S: ChannelDetailsSource + 'static>(
    id: &'static str,
    label: String,
    chip: Option<u32>,
    on: bool,
    to: DetailsTab,
    theme: &themes::Theme,
    cx: &mut Context<ChannelDetailsPanel<S>>,
) -> gpui::Stateful<gpui::Div> {
    let colors = *theme.colors();
    crate::controls::on_activate(
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .flex()
            .items_center()
            .gap(themes::metrics::SPACE_3)
            .h(themes::metrics::LAYOUT_DETAILS_TAB_HEIGHT)
            .px(themes::metrics::SPACE_1)
            .mr(themes::metrics::SPACE_6)
            .border_b_2()
            .border_color(if on {
                parla_tokens(theme).signal
            } else {
                gpui::transparent_black()
            })
            .text_size(themes::metrics::FONT_SIZE_MD)
            .font_weight(if on {
                themes::metrics::FONT_WEIGHT_SEMIBOLD
            } else {
                themes::metrics::FONT_WEIGHT_MEDIUM
            })
            .text_color(if on { colors.text } else { colors.text_muted })
            .cursor_pointer()
            .tab_stop(true)
            .focusable()
            .focus_visible(crate::controls::focus_style(&colors))
            .child(label)
            .when_some(chip, |d, n| {
                d.child(
                    div()
                        .px(themes::metrics::SPACE_2)
                        .rounded_full()
                        .bg(colors.element_hover)
                        .text_size(themes::metrics::FONT_SIZE_XS)
                        .text_color(colors.text_muted)
                        .child(n.to_string()),
                )
            }),
        cx,
        move |this, _, cx| this.set_tab(to, cx),
    )
}
