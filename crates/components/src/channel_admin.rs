//! Channel admin dialogs: create a channel, browse and join channels, edit a
//! channel's name, topic and description, invite people, and the in-app
//! confirm for leave and archive. Presentation only: each dialog emits what
//! the person asked for and the host runs it (the session, the provider).

use std::collections::HashSet;
use std::rc::Rc;

use domain::{ChannelName, ConversationId, NameProblem, UserId};
use gpui::{
    App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    Render, SharedString, Stateful, Subscription, Window, div, prelude::*,
};
use i18n::Localizer;
use themes::{ActiveTheme as _, ThemeColors, metrics, parla_tokens};

use crate::controls::{ButtonKind, check_box, field_frame};
use crate::text_input::{TextInput, TextInputEvent};

/// Key context shared by the dialogs.
pub const KEY_CONTEXT: &str = "ChannelAdminDialog";

/// The provider's rule for a typed channel name
/// (`ChatProvider::check_channel_name`): what it would store and what is wrong
/// with it.
pub type NameCheck = Rc<dyn Fn(&str) -> ChannelName>;

/// People rows the invite dialog draws at most (a workspace can hold thousands).
pub const INVITE_ROWS_MAX: usize = 50;

fn single_line(cx: &mut App, placeholder: String) -> Entity<TextInput> {
    cx.new(|cx| {
        let mut i = TextInput::new(cx);
        i.set_single_line(true);
        i.set_placeholder(placeholder);
        i
    })
}

/// The raised card every dialog sits in, a labelled dialog for assistive
/// technology.
fn frame(id: &'static str, label: SharedString, colors: &ThemeColors) -> Stateful<Div> {
    div()
        .id(id)
        .role(gpui::Role::Dialog)
        .aria_label(label)
        .key_context(KEY_CONTEXT)
        .flex()
        .flex_col()
        .gap(metrics::SPACE_5)
        .p(metrics::SPACE_6)
        .bg(colors.elevated_surface_background)
        .border_1()
        .border_color(colors.border)
        .rounded(metrics::RADIUS_LG)
        .shadow_md()
        .text_color(colors.text)
}

fn title(text: String) -> Div {
    div()
        .font_weight(metrics::FONT_WEIGHT_BOLD)
        .text_size(metrics::FONT_SIZE_2XL)
        .child(text)
}

fn label(text: String, colors: &ThemeColors) -> Div {
    div()
        .text_size(metrics::FONT_SIZE_XS)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(colors.text_muted)
        .child(text)
}

fn field(input: &Entity<TextInput>, sel: &'static str, theme: &themes::Theme) -> Div {
    field_frame(false, theme.colors(), parla_tokens(theme).border_control)
        .debug_selector(move || sel.to_string())
        .child(input.clone())
}

/// A one-line note under a field; an error reads in the danger colour
/// (`status.error`, the `danger` token).
fn hint(text: String, error: bool, theme: &themes::Theme) -> Div {
    div()
        .text_size(metrics::FONT_SIZE_XS)
        .text_color(if error {
            theme.status().error
        } else {
            parla_tokens(theme).text_subtle
        })
        .child(text)
}

fn button(
    id: &'static str,
    label: String,
    primary: bool,
    enabled: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let kind = if primary {
        ButtonKind::Primary
    } else {
        ButtonKind::Secondary
    };
    crate::controls::no_focus_on_click(
        crate::controls::button(id, label, kind, enabled, colors)
            .debug_selector(move || id.to_string()),
    )
}

/// The filled danger button (Archive, Leave a private channel).
fn danger(id: &'static str, label: String, theme: &themes::Theme) -> Stateful<Div> {
    crate::controls::no_focus_on_click(
        crate::controls::danger_button(id, label, true, theme)
            .debug_selector(move || id.to_string()),
    )
}

fn footer() -> Div {
    div().flex().flex_none().gap_2().justify_end()
}

/// Why a name cannot be used, in the active language.
pub fn name_problem_text(p: NameProblem, l: &Localizer) -> String {
    match p {
        NameProblem::Empty => t!(l, "channel-name-empty"),
        NameProblem::TooLong { max } => t!(l, "channel-name-too-long", max = max),
        NameProblem::Characters => t!(l, "channel-name-characters"),
    }
    .into_owned()
}

// ---- Create -------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateChannelEvent {
    /// Create `name` (normalized and checked); `private` makes it invite-only.
    Create {
        name: String,
        private: bool,
    },
    Cancel,
}

pub struct CreateChannelDialog {
    name: Entity<TextInput>,
    check: NameCheck,
    private: bool,
    /// The provider's answer to the last try (a taken name, ...).
    error: Option<String>,
    /// A create is on its way; the button waits for the answer.
    pending: bool,
    _sub: Subscription,
}

impl EventEmitter<CreateChannelEvent> for CreateChannelDialog {}

impl Focusable for CreateChannelDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.name.read(cx).focus_handle(cx)
    }
}

impl CreateChannelDialog {
    pub fn new(check: NameCheck, cx: &mut Context<Self>) -> Self {
        let l = i18n::gpui::localizer(cx);
        let name = single_line(cx, t!(l, "channel-create-name-placeholder").into_owned());
        let sub = cx.subscribe(&name, |this, _, _: &TextInputEvent, cx| {
            this.error = None;
            cx.notify();
        });
        Self {
            name,
            check,
            private: false,
            error: None,
            pending: false,
            _sub: sub,
        }
    }

    pub fn set_name(&mut self, text: &str, cx: &mut Context<Self>) {
        self.name.update(cx, |t, cx| t.set_text(text, cx));
        self.error = None;
        cx.notify();
    }

    pub fn set_private(&mut self, private: bool, cx: &mut Context<Self>) {
        self.private = private;
        cx.notify();
    }

    pub fn private(&self) -> bool {
        self.private
    }

    /// The name as it will be created.
    pub fn name(&self, cx: &App) -> String {
        (self.check)(self.name.read(cx).text()).name
    }

    /// What is wrong with the typed name, if anything.
    pub fn problem(&self, cx: &App) -> Option<NameProblem> {
        (self.check)(self.name.read(cx).text()).problem
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The provider refused the name (`Some`) or the host clears the wait.
    pub fn set_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.error = error;
        self.pending = false;
        cx.notify();
    }

    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if self.pending || self.problem(cx).is_some() {
            return;
        }
        self.pending = true;
        self.error = None;
        let name = self.name(cx);
        cx.emit(CreateChannelEvent::Create {
            name,
            private: self.private,
        });
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(CreateChannelEvent::Cancel);
    }
}

impl Render for CreateChannelDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let muted = parla_tokens(&theme).text_subtle;
        let l = i18n::gpui::localizer(cx);
        let typed = self.name.read(cx).text().to_string();
        let ChannelName { name, problem } = (self.check)(&typed);
        // Say what is wrong only once something is typed.
        let line = match (&self.error, problem) {
            (Some(e), _) => Some((e.clone(), true)),
            (None, Some(p)) if !typed.trim().is_empty() => Some((name_problem_text(p, &l), true)),
            (None, None) if name != typed => Some((
                t!(l, "channel-create-as", name = name.clone()).into_owned(),
                false,
            )),
            _ => None,
        };
        let can = problem.is_none() && !self.pending;
        let kind = |id: &'static str, private: bool, title: String, desc: String| {
            let on = self.private == private;
            let aria = SharedString::from(title.clone());
            crate::controls::on_activate(
                crate::settings_controls::radio_card(id, title, desc, on, &colors, muted)
                    .debug_selector(move || id.to_string())
                    .role(gpui::Role::RadioButton)
                    .aria_label(aria)
                    .aria_toggled(if on {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    }),
                cx,
                move |d: &mut Self, _, cx| d.set_private(private, cx),
            )
        };
        let visibility = t!(l, "channel-create-visibility").into_owned();
        frame(
            "create-channel",
            t!(l, "channel-create-title").into_owned().into(),
            &colors,
        )
        .debug_selector(|| "create-channel".into())
        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
            match ev.keystroke.key.as_str() {
                "escape" => this.cancel(cx),
                "enter" => this.submit(cx),
                _ => return,
            }
            cx.stop_propagation();
        }))
        .child(title(t!(l, "channel-create-title").into_owned()))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(metrics::SPACE_2)
                .child(label(t!(l, "channel-field-name").into_owned(), &colors))
                .child(field(&self.name, "create-channel-name", &theme))
                .children(line.map(|(text, err)| {
                    hint(text, err, &theme).debug_selector(|| "create-channel-hint".into())
                })),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(metrics::SPACE_3)
                .id("create-channel-visibility")
                .role(gpui::Role::RadioGroup)
                .aria_label(SharedString::from(visibility.clone()))
                .child(label(visibility, &colors))
                .child(kind(
                    "create-channel-public",
                    false,
                    t!(l, "channel-create-public").into_owned(),
                    t!(l, "channel-create-public-hint").into_owned(),
                ))
                .child(kind(
                    "create-channel-private",
                    true,
                    t!(l, "channel-create-private").into_owned(),
                    t!(l, "channel-create-private-hint").into_owned(),
                )),
        )
        .child(
            footer()
                .child(crate::controls::on_activate(
                    button(
                        "create-channel-cancel",
                        t!(l, "channel-cancel").into_owned(),
                        false,
                        true,
                        &colors,
                    ),
                    cx,
                    |d, _, cx| d.cancel(cx),
                ))
                .child(crate::controls::on_activate(
                    button(
                        "create-channel-submit",
                        t!(l, "channel-create-submit").into_owned(),
                        true,
                        can,
                        &colors,
                    ),
                    cx,
                    |d, _, cx| d.submit(cx),
                )),
        )
    }
}

// ---- Browse -------------------------------------------------------------

/// One channel in the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowseRow {
    pub id: ConversationId,
    pub name: SharedString,
    pub private: bool,
    pub purpose: SharedString,
    pub members: Option<u32>,
    pub is_member: bool,
}

/// A browser row's words, built when the rows or the join change, not per frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowseRowText {
    /// "12 members · Joined".
    pub meta: SharedString,
    /// The button's text: Join, Joining… or Open.
    pub action: SharedString,
    /// The button's accessible name: "Join #design".
    pub action_label: SharedString,
}

/// Wait after the last keystroke before the browser asks for a new page.
pub const BROWSE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelBrowserEvent {
    /// The search text settled ([`BROWSE_DEBOUNCE`] after the last
    /// keystroke); the host drops any page on its way and loads the first
    /// one for it.
    Query(String),
    /// The next page for the current text.
    More,
    Join(ConversationId),
    /// A channel the user is in already: go to it.
    Open(ConversationId),
    /// Create a channel instead.
    Create,
    Cancel,
}

pub struct ChannelBrowser {
    query: Entity<TextInput>,
    rows: Vec<BrowseRow>,
    texts: Vec<BrowseRowText>,
    /// The timer for the typed query; a new keystroke drops (cancels) it.
    pending: Option<gpui::Task<()>>,
    private_label: SharedString,
    loading: bool,
    has_more: bool,
    error: Option<String>,
    /// The channel a join is on its way for.
    joining: Option<ConversationId>,
    _sub: Subscription,
}

impl EventEmitter<ChannelBrowserEvent> for ChannelBrowser {}

impl Focusable for ChannelBrowser {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
}

impl ChannelBrowser {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let l = i18n::gpui::localizer(cx);
        let query = single_line(cx, t!(l, "channel-browse-placeholder").into_owned());
        let sub = cx.subscribe(&query, |this, input, _: &TextInputEvent, cx| {
            let text = input.read(cx).text().to_string();
            this.loading = true;
            this.pending = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(BROWSE_DEBOUNCE).await;
                this.update(cx, |this, cx| {
                    this.pending = None;
                    cx.emit(ChannelBrowserEvent::Query(text));
                })
                .ok();
            }));
            cx.notify();
        });
        Self {
            query,
            rows: Vec::new(),
            texts: Vec::new(),
            pending: None,
            private_label: t!(l, "channel-create-private").into_owned().into(),
            loading: true,
            has_more: false,
            error: None,
            joining: None,
            _sub: sub,
        }
    }

    pub fn query(&self, cx: &App) -> String {
        self.query.read(cx).text().to_string()
    }

    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.query.update(cx, |t, cx| t.set_text(text, cx));
    }

    pub fn rows(&self) -> &[BrowseRow] {
        &self.rows
    }

    pub fn row_texts(&self) -> &[BrowseRowText] {
        &self.texts
    }

    /// Rebuilds the rows' words (rows, language or the join changed).
    fn relabel(&mut self, cx: &App) {
        let l = i18n::gpui::localizer(cx);
        self.texts = self
            .rows
            .iter()
            .map(|r| {
                let mut meta = String::new();
                if let Some(n) = r.members {
                    meta.push_str(&t!(l, "channel-browse-members", count = n));
                }
                if r.is_member {
                    if !meta.is_empty() {
                        meta.push_str(" \u{b7} ");
                    }
                    meta.push_str(&t!(l, "channel-browse-joined"));
                }
                let name = r.name.to_string();
                let (action, action_label) = if r.is_member {
                    (
                        t!(l, "channel-browse-open"),
                        t!(l, "channel-browse-open-label", name = name),
                    )
                } else if self.joining.as_ref() == Some(&r.id) {
                    (
                        t!(l, "channel-browse-joining"),
                        t!(l, "channel-browse-join-label", name = name),
                    )
                } else {
                    (
                        t!(l, "channel-browse-join"),
                        t!(l, "channel-browse-join-label", name = name),
                    )
                };
                BrowseRowText {
                    meta: meta.into(),
                    action: action.into_owned().into(),
                    action_label: action_label.into_owned().into(),
                }
            })
            .collect();
    }

    /// A page arrived: the first replaces the list, a later one extends it.
    pub fn set_page(
        &mut self,
        rows: Vec<BrowseRow>,
        first: bool,
        has_more: bool,
        cx: &mut Context<Self>,
    ) {
        if first {
            self.rows = rows;
        } else {
            self.rows.extend(rows);
        }
        self.has_more = has_more;
        self.loading = false;
        self.relabel(cx);
        self.error = None;
        cx.notify();
    }

    pub fn set_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.error = error;
        self.loading = false;
        self.joining = None;
        self.relabel(cx);
        cx.notify();
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn join(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(r) = self.rows.get(ix) else { return };
        if r.is_member {
            cx.emit(ChannelBrowserEvent::Open(r.id.clone()));
        } else if self.joining.is_none() {
            let id = r.id.clone();
            self.joining = Some(id.clone());
            self.relabel(cx);
            cx.emit(ChannelBrowserEvent::Join(id));
            cx.notify();
        }
    }

    fn more(&mut self, cx: &mut Context<Self>) {
        if self.has_more && !self.loading {
            self.loading = true;
            cx.emit(ChannelBrowserEvent::More);
            cx.notify();
        }
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(ChannelBrowserEvent::Cancel);
    }
}

impl Render for ChannelBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let tokens = parla_tokens(&theme);
        let l = i18n::gpui::localizer(cx);
        let mut list = div()
            .id("browse-list")
            .debug_selector(|| "browse-list".into())
            .role(gpui::Role::List)
            .flex()
            .flex_col()
            .max_h(metrics::LAYOUT_POPOVER_MAX_HEIGHT)
            .overflow_y_scroll();
        for (ix, (r, text)) in self.rows.iter().zip(&self.texts).enumerate() {
            let busy = self.joining.as_ref() == Some(&r.id);
            let sel = if r.is_member {
                format!("browse-open-{ix}")
            } else {
                format!("browse-join-{ix}")
            };
            let meta = text.meta.clone();
            let action = crate::controls::compact(
                crate::controls::button(
                    ("browse-action", ix),
                    text.action.clone(),
                    if r.is_member {
                        ButtonKind::Secondary
                    } else {
                        ButtonKind::Primary
                    },
                    !busy,
                    &colors,
                )
                .debug_selector(move || sel.clone())
                .role(gpui::Role::Button)
                .aria_label(text.action_label.clone()),
            );
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_5)
                    .px(metrics::SPACE_3)
                    .py(metrics::SPACE_4)
                    .when(ix > 0, |d| {
                        d.border_t_1().border_color(tokens.border_subtle)
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(metrics::SPACE_1)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(metrics::SPACE_2)
                                    .min_w_0()
                                    .text_size(metrics::FONT_SIZE_MD)
                                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                    .map(|d| {
                                        if r.private {
                                            d.child(
                                                div()
                                                    .id(("browse-private", ix))
                                                    .flex_none()
                                                    .role(gpui::Role::Image)
                                                    .aria_label(self.private_label.clone())
                                                    .child(
                                                        crate::icon::Icon::new(
                                                            crate::icon::IconName::Lock,
                                                        )
                                                        .size(metrics::ICON_SIZE_SM)
                                                        .color(colors.text_muted),
                                                    ),
                                            )
                                        } else {
                                            d.child(div().flex_none().child("#"))
                                        }
                                    })
                                    .child(div().min_w_0().truncate().child(r.name.clone())),
                            )
                            .when(!meta.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(metrics::FONT_SIZE_XS)
                                        .text_color(tokens.text_subtle)
                                        .child(meta),
                                )
                            })
                            .when(!r.purpose.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(metrics::FONT_SIZE_SM)
                                        .text_color(colors.text_muted)
                                        .truncate()
                                        .child(r.purpose.clone()),
                                )
                            }),
                    )
                    .child(crate::controls::on_activate(action, cx, move |d, _, cx| {
                        d.join(ix, cx)
                    })),
            );
        }
        let status = if let Some(e) = &self.error {
            Some(hint(e.clone(), true, &theme))
        } else if self.loading && self.rows.is_empty() {
            Some(hint(
                t!(l, "channel-browse-loading").into_owned(),
                false,
                &theme,
            ))
        } else if self.rows.is_empty() {
            Some(hint(
                t!(l, "channel-browse-empty").into_owned(),
                false,
                &theme,
            ))
        } else {
            None
        };
        frame(
            "channel-browser",
            t!(l, "channel-browse-title").into_owned().into(),
            &colors,
        )
        .debug_selector(|| "channel-browser".into())
        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
            if ev.keystroke.key == "escape" {
                this.cancel(cx);
                cx.stop_propagation();
            }
        }))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(metrics::SPACE_4)
                .child(title(t!(l, "channel-browse-title").into_owned()))
                .child(crate::controls::on_activate(
                    button(
                        "browse-create",
                        t!(l, "channel-browse-create").into_owned(),
                        false,
                        true,
                        &colors,
                    ),
                    cx,
                    |_, _, cx| cx.emit(ChannelBrowserEvent::Create),
                )),
        )
        .child(field(&self.query, "browse-query", &theme))
        .children(status.map(|s| s.debug_selector(|| "browse-status".into())))
        .child(list)
        .child(
            footer()
                .when(self.has_more, |d| {
                    d.child(crate::controls::on_activate(
                        button(
                            "browse-more",
                            t!(l, "channel-browse-more").into_owned(),
                            false,
                            !self.loading,
                            &colors,
                        ),
                        cx,
                        |d, _, cx| d.more(cx),
                    ))
                })
                .child(crate::controls::on_activate(
                    button(
                        "browse-close",
                        t!(l, "channel-close").into_owned(),
                        false,
                        true,
                        &colors,
                    ),
                    cx,
                    |d, _, cx| d.cancel(cx),
                )),
        )
    }
}

// ---- Edit ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditField {
    Name,
    Topic,
    /// The description: what the channel is for.
    Purpose,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditChannelEvent {
    /// The fields that changed (`None`: unchanged); the name normalized.
    Save {
        name: Option<String>,
        topic: Option<String>,
        purpose: Option<String>,
    },
    Cancel,
}

pub struct EditChannelDialog {
    name: Entity<TextInput>,
    topic: Entity<TextInput>,
    purpose: Entity<TextInput>,
    initial: [String; 3],
    /// Channels can be renamed; a group or direct message cannot.
    can_rename: bool,
    /// The name field was typed in: only then is a rename sent, so a stored
    /// name the rules would spell differently is never rewritten unasked.
    name_dirty: bool,
    check: NameCheck,
    _subs: Vec<Subscription>,
}

impl EventEmitter<EditChannelEvent> for EditChannelDialog {}

impl Focusable for EditChannelDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        let first = if self.can_rename {
            &self.name
        } else {
            &self.topic
        };
        first.read(cx).focus_handle(cx)
    }
}

impl EditChannelDialog {
    pub fn new(
        name: &str,
        topic: &str,
        purpose: &str,
        can_rename: bool,
        check: NameCheck,
        cx: &mut Context<Self>,
    ) -> Self {
        let l = i18n::gpui::localizer(cx);
        let make = |cx: &mut Context<Self>, ph: String, text: &str| {
            let i = single_line(cx, ph);
            i.update(cx, |t, cx| t.set_text(text, cx));
            i
        };
        let n = make(
            cx,
            t!(l, "channel-create-name-placeholder").into_owned(),
            name,
        );
        let t = make(
            cx,
            t!(l, "channel-edit-topic-placeholder").into_owned(),
            topic,
        );
        let p = make(
            cx,
            t!(l, "channel-edit-purpose-placeholder").into_owned(),
            purpose,
        );
        let mut subs: Vec<Subscription> = [&t, &p]
            .into_iter()
            .map(|i| cx.subscribe(i, |_, _, _: &TextInputEvent, cx| cx.notify()))
            .collect();
        subs.push(cx.subscribe(&n, |this, _, _: &TextInputEvent, cx| {
            this.name_dirty = true;
            cx.notify();
        }));
        Self {
            name: n,
            topic: t,
            purpose: p,
            initial: [name.to_string(), topic.to_string(), purpose.to_string()],
            can_rename,
            name_dirty: false,
            check,
            _subs: subs,
        }
    }

    fn input(&self, f: EditField) -> &Entity<TextInput> {
        match f {
            EditField::Name => &self.name,
            EditField::Topic => &self.topic,
            EditField::Purpose => &self.purpose,
        }
    }

    pub fn set_field(&mut self, f: EditField, text: &str, cx: &mut Context<Self>) {
        self.input(f).update(cx, |t, cx| t.set_text(text, cx));
        if f == EditField::Name {
            self.name_dirty = true;
        }
        cx.notify();
    }

    /// The typed name, when the person changed it.
    fn new_name(&self, cx: &App) -> Option<ChannelName> {
        if !self.can_rename || !self.name_dirty {
            return None;
        }
        let n = (self.check)(self.name.read(cx).text());
        (n.name != self.initial[0]).then_some(n)
    }

    /// What is wrong with a changed name, if anything.
    pub fn problem(&self, cx: &App) -> Option<NameProblem> {
        self.new_name(cx).and_then(|n| n.problem)
    }

    fn changes(&self, cx: &App) -> (Option<String>, Option<String>, Option<String>) {
        let topic = self.topic.read(cx).text().trim().to_string();
        let purpose = self.purpose.read(cx).text().trim().to_string();
        (
            self.new_name(cx).map(|n| n.name),
            (topic != self.initial[1].trim()).then_some(topic),
            (purpose != self.initial[2].trim()).then_some(purpose),
        )
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.problem(cx).is_some() {
            return;
        }
        let (name, topic, purpose) = self.changes(cx);
        if name.is_none() && topic.is_none() && purpose.is_none() {
            cx.emit(EditChannelEvent::Cancel);
            return;
        }
        cx.emit(EditChannelEvent::Save {
            name,
            topic,
            purpose,
        });
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(EditChannelEvent::Cancel);
    }
}

impl Render for EditChannelDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let l = i18n::gpui::localizer(cx);
        let problem = self.problem(cx);
        let section = |text: String, input: &Entity<TextInput>, sel: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap(metrics::SPACE_2)
                .child(label(text, &colors))
                .child(field(input, sel, &theme))
        };
        frame(
            "edit-channel",
            t!(l, "channel-edit-title").into_owned().into(),
            &colors,
        )
        .debug_selector(|| "edit-channel".into())
        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
            match ev.keystroke.key.as_str() {
                "escape" => this.cancel(cx),
                "enter" => this.save(cx),
                _ => return,
            }
            cx.stop_propagation();
        }))
        .child(title(t!(l, "channel-edit-title").into_owned()))
        .when(self.can_rename, |d| {
            d.child(
                section(
                    t!(l, "channel-field-name").into_owned(),
                    &self.name,
                    "edit-channel-name",
                )
                .children(problem.map(|p| hint(name_problem_text(p, &l), true, &theme))),
            )
        })
        .child(section(
            t!(l, "channel-field-topic").into_owned(),
            &self.topic,
            "edit-channel-topic",
        ))
        .child(section(
            t!(l, "channel-field-purpose").into_owned(),
            &self.purpose,
            "edit-channel-purpose",
        ))
        .child(
            footer()
                .child(crate::controls::on_activate(
                    button(
                        "edit-channel-cancel",
                        t!(l, "channel-cancel").into_owned(),
                        false,
                        true,
                        &colors,
                    ),
                    cx,
                    |d, _, cx| d.cancel(cx),
                ))
                .child(crate::controls::on_activate(
                    button(
                        "edit-channel-save",
                        t!(l, "channel-edit-save").into_owned(),
                        true,
                        problem.is_none(),
                        &colors,
                    ),
                    cx,
                    |d, _, cx| d.save(cx),
                )),
        )
    }
}

// ---- Confirm ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmEvent {
    Confirm,
    Cancel,
}

/// "Leave #x?" and "Archive #x?": a title, one line and two buttons.
pub struct ConfirmDialog {
    title: SharedString,
    body: SharedString,
    ok: SharedString,
    /// Archive and leaving a private channel are filled in the danger colour.
    danger: bool,
    /// A quoted preview of the message, used by the message-delete prompt.
    preview: Option<SharedString>,
    /// The Delete key (and Cmd-Backspace on macOS) confirms as well; the
    /// message-delete prompt.
    delete_confirms: bool,
    /// The dialog opens on Cancel: a stray Enter never confirms.
    cancel_focus: FocusHandle,
    ok_focus: FocusHandle,
}

impl EventEmitter<ConfirmEvent> for ConfirmDialog {}

impl Focusable for ConfirmDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.cancel_focus.clone()
    }
}

impl ConfirmDialog {
    pub fn new(title: String, body: String, ok: String, cx: &mut Context<Self>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            ok: ok.into(),
            danger: false,
            preview: None,
            delete_confirms: false,
            cancel_focus: cx.focus_handle().tab_stop(true),
            ok_focus: cx.focus_handle().tab_stop(true),
        }
    }

    /// Fills the confirm button in the danger colour.
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    /// Shows `text` in a quoted frame between the title and the body.
    pub fn preview(mut self, text: String) -> Self {
        self.preview = (!text.is_empty()).then(|| text.into());
        self
    }

    /// Lets the Delete key (Cmd-Backspace on macOS) confirm. The host focuses
    /// the confirm button so Enter confirms too; Escape and Cancel still
    /// decline.
    pub fn delete_confirms(mut self) -> Self {
        self.delete_confirms = true;
        self
    }

    pub fn is_danger(&self) -> bool {
        self.danger
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Focus handle of the confirm button, for hosts that open on Delete.
    pub fn confirm_focus(&self) -> FocusHandle {
        self.ok_focus.clone()
    }

    /// The quoted preview, when the dialog has one.
    pub fn preview_str(&self) -> Option<&str> {
        self.preview.as_deref()
    }
}

/// Delete, or Cmd-Backspace on macOS (the shortcut that opens the prompt).
fn is_delete_key(k: &gpui::Keystroke) -> bool {
    let m = &k.modifiers;
    (k.key == "delete" && !m.modified())
        || (cfg!(target_os = "macos")
            && k.key == "backspace"
            && m.platform
            && !m.control
            && !m.alt
            && !m.shift)
}

impl Render for ConfirmDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let l = i18n::gpui::localizer(cx);
        let ok = if self.danger {
            danger("confirm-ok", self.ok.to_string(), &theme)
        } else {
            button("confirm-ok", self.ok.to_string(), true, true, &colors)
        };
        // Enter reaches only the focused button (Cancel first); held and
        // modified Enter are ignored there.
        let preview = self.preview.as_ref().map(|text| {
            div()
                .debug_selector(|| "confirm-preview".into())
                .flex_none()
                .rounded(metrics::RADIUS_MD)
                .border_1()
                .border_color(colors.border)
                .bg(colors.surface_background)
                .p(metrics::SPACE_4)
                .text_size(metrics::FONT_SIZE_MD)
                .text_color(colors.text)
                .line_clamp(3)
                .child(text.clone())
        });
        frame("confirm-dialog", self.title.clone(), &colors)
            .debug_selector(|| "confirm-dialog".into())
            .role(gpui::Role::AlertDialog)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    cx.emit(ConfirmEvent::Cancel);
                    cx.stop_propagation();
                } else if this.delete_confirms && !ev.is_held && is_delete_key(&ev.keystroke) {
                    cx.emit(ConfirmEvent::Confirm);
                    cx.stop_propagation();
                }
            }))
            .child(title(self.title.to_string()))
            .children(preview)
            .children((!self.body.is_empty()).then(|| {
                div()
                    .text_size(metrics::FONT_SIZE_MD)
                    .text_color(colors.text_muted)
                    .child(self.body.clone())
            }))
            .child(
                footer()
                    .child(crate::controls::on_activate(
                        button(
                            "confirm-cancel",
                            t!(l, "channel-cancel").into_owned(),
                            false,
                            true,
                            &colors,
                        )
                        .track_focus(&self.cancel_focus),
                        cx,
                        |_, _, cx| cx.emit(ConfirmEvent::Cancel),
                    ))
                    .child(crate::controls::on_activate(
                        ok.track_focus(&self.ok_focus),
                        cx,
                        |_, _, cx| cx.emit(ConfirmEvent::Confirm),
                    )),
            )
    }
}

// ---- Invite -------------------------------------------------------------

/// One person who can be invited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteRow {
    pub id: UserId,
    pub name: SharedString,
    /// A second line: the handle, when it differs from the name.
    pub detail: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InviteEvent {
    Invite(Vec<UserId>),
    Cancel,
}

pub struct InviteDialog {
    title: SharedString,
    query: Entity<TextInput>,
    all: Vec<InviteRow>,
    /// Indices into `all` the filter keeps, at most [`INVITE_ROWS_MAX`].
    shown: Vec<usize>,
    picked: Vec<UserId>,
    _sub: Subscription,
}

impl EventEmitter<InviteEvent> for InviteDialog {}

impl Focusable for InviteDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
}

impl InviteDialog {
    /// `people` are those not in the channel yet, in the order to show them.
    pub fn new(title: String, people: Vec<InviteRow>, cx: &mut Context<Self>) -> Self {
        let l = i18n::gpui::localizer(cx);
        let query = single_line(cx, t!(l, "channel-invite-placeholder").into_owned());
        let sub = cx.subscribe(&query, |this, input, _: &TextInputEvent, cx| {
            let q = input.read(cx).text().to_string();
            this.filter(&q);
            cx.notify();
        });
        let mut d = Self {
            title: title.into(),
            query,
            all: people,
            shown: Vec::new(),
            picked: Vec::new(),
            _sub: sub,
        };
        d.filter("");
        d
    }

    fn filter(&mut self, q: &str) {
        let q = q.trim().to_lowercase();
        self.shown = self
            .all
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                q.is_empty()
                    || r.name.to_lowercase().contains(&q)
                    || r.detail.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .take(INVITE_ROWS_MAX)
            .collect();
    }

    pub fn set_query(&mut self, q: &str, cx: &mut Context<Self>) {
        self.query.update(cx, |t, cx| t.set_text(q, cx));
        self.filter(q);
        cx.notify();
    }

    /// The people the filter keeps.
    pub fn shown(&self) -> Vec<&InviteRow> {
        self.shown.iter().map(|&i| &self.all[i]).collect()
    }

    pub fn picked(&self) -> &[UserId] {
        &self.picked
    }

    pub fn toggle(&mut self, id: &UserId, cx: &mut Context<Self>) {
        match self.picked.iter().position(|u| u == id) {
            Some(i) => {
                self.picked.remove(i);
            }
            None => self.picked.push(id.clone()),
        }
        cx.notify();
    }

    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.picked.is_empty() {
            cx.emit(InviteEvent::Invite(self.picked.clone()));
        }
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(InviteEvent::Cancel);
    }
}

impl Render for InviteDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let tokens = parla_tokens(&theme);
        let l = i18n::gpui::localizer(cx);
        let picked: HashSet<&UserId> = self.picked.iter().collect();
        let hover = colors.element_hover;
        let mut list = div()
            .id("invite-list")
            .role(gpui::Role::List)
            .flex()
            .flex_col()
            .max_h(metrics::LAYOUT_POPOVER_MAX_HEIGHT)
            .overflow_y_scroll();
        for (ix, &i) in self.shown.iter().enumerate() {
            let r = &self.all[i];
            let on = picked.contains(&r.id);
            let id = r.id.clone();
            list = list.child(crate::controls::on_activate(
                div()
                    .id(("invite-row", ix))
                    .debug_selector(move || format!("invite-row-{ix}"))
                    .role(gpui::Role::CheckBox)
                    .aria_label(r.name.clone())
                    .aria_toggled(if on {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_5)
                    .px(metrics::SPACE_3)
                    .py(metrics::SPACE_3)
                    .rounded(metrics::RADIUS_MD)
                    .cursor_pointer()
                    .hover(move |d| d.bg(hover))
                    .tab_stop(true)
                    .focusable()
                    .focus_visible(crate::controls::focus_style(&colors))
                    .child(check_box(on, &colors, tokens.border_control))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(metrics::FONT_SIZE_MD)
                                    .truncate()
                                    .child(r.name.clone()),
                            )
                            .when(!r.detail.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(metrics::FONT_SIZE_XS)
                                        .text_color(tokens.text_subtle)
                                        .child(r.detail.clone()),
                                )
                            }),
                    ),
                cx,
                move |d, _, cx| d.toggle(&id, cx),
            ));
        }
        let n = self.picked.len();
        frame("invite-dialog", self.title.clone(), &colors)
            .debug_selector(|| "invite-dialog".into())
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let k = &ev.keystroke;
                match k.key.as_str() {
                    "escape" => this.cancel(cx),
                    // Enter in the search field sends the picked people.
                    "enter"
                        if !ev.is_held
                            && !k.modifiers.platform
                            && !k.modifiers.control
                            && !k.modifiers.alt
                            && !k.modifiers.shift =>
                    {
                        this.submit(cx)
                    }
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(title(self.title.to_string()))
            .child(field(&self.query, "invite-query", &theme))
            .when(self.shown.is_empty(), |d| {
                d.child(hint(
                    t!(l, "channel-invite-none").into_owned(),
                    false,
                    &theme,
                ))
            })
            .child(list)
            .child(
                footer()
                    .child(crate::controls::on_activate(
                        button(
                            "invite-cancel",
                            t!(l, "channel-cancel").into_owned(),
                            false,
                            true,
                            &colors,
                        ),
                        cx,
                        |d, _, cx| d.cancel(cx),
                    ))
                    .child(crate::controls::on_activate(
                        button(
                            "invite-submit",
                            t!(l, "channel-invite-submit", count = n).into_owned(),
                            true,
                            n > 0,
                            &colors,
                        ),
                        cx,
                        |d, _, cx| d.submit(cx),
                    )),
            )
    }
}
