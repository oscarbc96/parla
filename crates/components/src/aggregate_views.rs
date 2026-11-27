//! The cross-workspace views: "All unreads", "All mentions" and "All
//! scheduled". Each row names its workspace; opening one is the host's job.

use std::collections::HashMap;

use domain::{ConversationId, LocalScheduleState, MessageId, ScheduleId};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement, KeyDownEvent,
    Render, SharedString, Subscription, Window, div, prelude::*,
};
use i18n::Translate as _;
use settings::WorkspaceKey;
use themes::{ActiveTheme, parla_tokens};

use crate::message_row::PreviewLine;
use crate::saved_view::list_note;
use crate::send_later::SendLaterConfig;

gpui::actions!(workspace, [AllUnreads, AllMentions, AllScheduled]);

/// Key context of the views.
pub const KEY_CONTEXT: &str = "AggregateView";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggregateKind {
    Unreads,
    Mentions,
    Scheduled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AggregateTarget {
    Conversation(ConversationId),
    Message(MessageId),
    Schedule(ScheduleId),
}

/// The workspace a row belongs to.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceTag {
    pub key: WorkspaceKey,
    pub name: SharedString,
    pub accent: Option<Hsla>,
}

/// An unread conversation of some workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct UnreadItem {
    pub workspace: WorkspaceTag,
    pub conversation: ConversationId,
    pub label: SharedString,
    pub unread: bool,
    pub mentions: u32,
    /// Newest message, to mark the conversation read up to it.
    pub latest: Option<domain::MessageTs>,
    /// Where the user read up to (keys the previews).
    pub last_read: Option<domain::MessageTs>,
}

/// A held message that mentions the user.
#[derive(Clone, Debug, PartialEq)]
pub struct MentionLine {
    pub id: MessageId,
    pub author: SharedString,
    pub text: SharedString,
}

/// A conversation with mentions and the held messages that carry them
/// (none for a lean workspace or one whose window was dropped).
#[derive(Clone, Debug, PartialEq)]
pub struct MentionItem {
    pub row: UnreadItem,
    pub messages: Vec<MentionLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledItem {
    pub workspace: WorkspaceTag,
    pub id: ScheduleId,
    pub conversation: SharedString,
    pub text: SharedString,
    pub post_at: i64,
    pub local: bool,
    pub state: Option<LocalScheduleState>,
}

/// What the views need from the manager, so `components` does not depend on `parla`.
pub trait AggregateSource {
    fn unreads(&self, cx: &App) -> Vec<UnreadItem>;
    fn mentions(&self, cx: &App) -> Vec<MentionItem>;
    fn scheduled(&self, cx: &App) -> Vec<ScheduledItem>;
    /// Marks each listed conversation read at its newest message.
    fn mark_all_read(&mut self, cx: &mut App);
    /// Marks one conversation read at `latest`, the newest message its row showed.
    fn mark_read(
        &mut self,
        workspace: &WorkspaceKey,
        conversation: &ConversationId,
        latest: domain::MessageTs,
        cx: &mut App,
    );
    /// The newest held unread messages of a conversation, oldest first and
    /// bounded; none when it is not held. Read only for shown rows, and
    /// again only when the row's newest or read marker changes.
    fn previews(
        &self,
        _workspace: &WorkspaceKey,
        _conversation: &ConversationId,
        _cx: &App,
    ) -> Vec<PreviewLine> {
        Vec::new()
    }
    /// Zone and clock for the times shown.
    fn config(&self, cx: &App) -> SendLaterConfig;
}

/// One row, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct AggregateEntry {
    pub workspace: WorkspaceTag,
    pub target: AggregateTarget,
    pub title: String,
    pub unread: bool,
    pub mentions: u32,
    /// Held mention texts, or the time of a scheduled item.
    pub lines: Vec<String>,
    pub tag: Option<String>,
    pub failed: bool,
    /// Unread message previews ("All unreads" only, shown rows only).
    pub previews: Vec<PreviewLine>,
    /// Newest message and read marker of the row's conversation.
    pub latest: Option<domain::MessageTs>,
    pub last_read: Option<domain::MessageTs>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AggregateEvent {
    /// Show this row in its workspace.
    Open {
        workspace: WorkspaceKey,
        target: AggregateTarget,
    },
    Close,
}

/// Rows drawn at first and added by each "Show more": a large account has
/// hundreds of unread conversations and laying them all out every frame is slow.
pub const PAGE: usize = 100;

type PreviewKey = (
    WorkspaceKey,
    ConversationId,
    Option<domain::MessageTs>,
    Option<domain::MessageTs>,
);

pub struct AggregateView<S: AggregateSource + 'static> {
    kind: AggregateKind,
    shown: usize,
    /// The rows as last read from the source; the view redraws only when they change.
    rows: Vec<AggregateEntry>,
    session: Entity<S>,
    /// Previews of the shown rows, keyed by row and by the newest message and
    /// read marker they were read at; only shown rows are kept.
    previews: HashMap<PreviewKey, Vec<PreviewLine>>,
    confirming: bool,
    focus: FocusHandle,
    _observe: Subscription,
}

impl<S: AggregateSource + 'static> EventEmitter<AggregateEvent> for AggregateView<S> {}

impl<S: AggregateSource + 'static> AggregateView<S> {
    pub fn new(kind: AggregateKind, session: Entity<S>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&session, |this, _, cx| this.refresh(cx));
        let rows = Vec::new();
        let mut this = Self {
            kind,
            session,
            shown: PAGE,
            rows,
            previews: HashMap::new(),
            confirming: false,
            focus: cx.focus_handle(),
            _observe: observe,
        };
        let mut rows = this.entries(cx);
        this.fill_previews(&mut rows, cx);
        this.rows = rows;
        this
    }

    /// The rows as drawn, previews included.
    pub fn rows(&self) -> &[AggregateEntry] {
        &self.rows
    }

    /// Puts the previews on the shown unread rows: kept ones when the row's
    /// newest message and read marker are unchanged, else read from the source.
    fn fill_previews(&mut self, rows: &mut [AggregateEntry], cx: &App) {
        if self.kind != AggregateKind::Unreads {
            return;
        }
        let s = self.session.read(cx);
        let mut kept = HashMap::new();
        for e in rows.iter_mut().take(self.shown) {
            let AggregateTarget::Conversation(c) = &e.target else {
                continue;
            };
            let key = (
                e.workspace.key.clone(),
                c.clone(),
                e.latest.clone(),
                e.last_read.clone(),
            );
            let lines = match self.previews.remove(&key) {
                Some(l) => l,
                None => s.previews(&e.workspace.key, c, cx),
            };
            e.previews = lines.clone();
            kept.insert(key, lines);
        }
        self.previews = kept;
    }

    /// Reads the rows again; redraws only when they differ.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let mut rows = self.entries(cx);
        self.fill_previews(&mut rows, cx);
        if rows != self.rows {
            self.rows = rows;
            cx.notify();
        }
    }

    pub fn kind(&self) -> AggregateKind {
        self.kind
    }

    /// How many rows are drawn.
    pub fn shown(&self) -> usize {
        self.shown
    }

    /// Draws one more page of rows.
    pub fn show_more(&mut self, cx: &mut Context<Self>) {
        self.shown += PAGE;
        let mut rows = std::mem::take(&mut self.rows);
        self.fill_previews(&mut rows, cx);
        self.rows = rows;
        cx.notify();
    }

    /// The rows, in the order the source gives them (the aggregates sort them).
    pub fn entries(&self, cx: &App) -> Vec<AggregateEntry> {
        let s = self.session.read(cx);
        let l = cx.localizer();
        match self.kind {
            AggregateKind::Unreads => s
                .unreads(cx)
                .into_iter()
                .map(|r| AggregateEntry {
                    target: AggregateTarget::Conversation(r.conversation),
                    workspace: r.workspace,
                    title: r.label.to_string(),
                    unread: r.unread,
                    mentions: r.mentions,
                    lines: Vec::new(),
                    tag: None,
                    failed: false,
                    previews: Vec::new(),
                    latest: r.latest,
                    last_read: r.last_read,
                })
                .collect(),
            AggregateKind::Mentions => s
                .mentions(cx)
                .into_iter()
                .map(|m| {
                    let held = m.messages.first().map(|f| f.id.clone());
                    let lines: Vec<String> = m
                        .messages
                        .iter()
                        .map(|x| format!("{}: {}", x.author, x.text))
                        .collect();
                    let load = lines.is_empty();
                    AggregateEntry {
                        target: match held {
                            Some(id) => AggregateTarget::Message(id),
                            None => AggregateTarget::Conversation(m.row.conversation),
                        },
                        workspace: m.row.workspace,
                        title: m.row.label.to_string(),
                        unread: m.row.unread,
                        mentions: m.row.mentions,
                        lines,
                        tag: load.then(|| t!(l, "aggregate-open-to-load").into_owned()),
                        failed: false,
                        previews: Vec::new(),
                        latest: m.row.latest,
                        last_read: m.row.last_read,
                    }
                })
                .collect(),
            AggregateKind::Scheduled => {
                let cfg = s.config(cx);
                let hc = crate::message_row::hour_cycle(cfg.clock);
                let mut items = s.scheduled(cx);
                items.sort_by_key(|i| i.post_at);
                items
                    .into_iter()
                    .map(|i| {
                        let when = jiff::Timestamp::from_second(i.post_at)
                            .map(|t| {
                                format!("{} {}", l.date(t, &cfg.zone), l.time(t, &cfg.zone, hc))
                            })
                            .unwrap_or_default();
                        let (tag, failed) = match &i.state {
                            None => (None, false),
                            Some(LocalScheduleState::Pending) => {
                                (Some(t!(l, "scheduled-local-tag").into_owned()), false)
                            }
                            Some(LocalScheduleState::Sending) => {
                                (Some(t!(l, "scheduled-sending").into_owned()), false)
                            }
                            Some(LocalScheduleState::Failed(why)) => (
                                Some(t!(l, "scheduled-failed", reason = why.as_str()).into_owned()),
                                true,
                            ),
                        };
                        AggregateEntry {
                            workspace: i.workspace,
                            target: AggregateTarget::Schedule(i.id),
                            title: t!(l, "scheduled-to", conversation = i.conversation.as_str())
                                .into_owned(),
                            unread: false,
                            mentions: 0,
                            lines: vec![i.text.to_string(), when],
                            tag,
                            failed,
                            previews: Vec::new(),
                            latest: None,
                            last_read: None,
                        }
                    })
                    .collect()
            }
        }
    }

    /// Asks the host to show row `index` in its workspace.
    pub fn open(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(e) = self.rows.get(index).cloned() {
            self.open_target(e.workspace.key, e.target, cx);
        }
    }

    /// Opens a row by what it points at, so a refresh between paint and
    /// click cannot open a different row.
    fn open_target(
        &mut self,
        workspace: WorkspaceKey,
        target: AggregateTarget,
        cx: &mut Context<Self>,
    ) {
        cx.emit(AggregateEvent::Open { workspace, target });
    }

    /// "Mark read" on a row: marks its conversation read at `latest`, the
    /// newest message the row showed (what the user saw, not what came since).
    pub fn mark_read(
        &mut self,
        workspace: WorkspaceKey,
        conversation: ConversationId,
        latest: domain::MessageTs,
        cx: &mut Context<Self>,
    ) {
        self.session.update(cx, |s, cx| {
            s.mark_read(&workspace, &conversation, latest, cx)
        });
    }

    pub fn confirming(&self) -> bool {
        self.confirming
    }

    /// "Mark all read": asks first.
    pub fn request_mark_all_read(&mut self, cx: &mut Context<Self>) {
        if self.kind == AggregateKind::Unreads && !self.rows.is_empty() {
            self.confirming = true;
            cx.notify();
        }
    }

    pub fn confirm_mark_all_read(&mut self, cx: &mut Context<Self>) {
        if !self.confirming {
            return;
        }
        self.confirming = false;
        self.session.update(cx, |s, cx| s.mark_all_read(cx));
        cx.notify();
    }

    pub fn cancel_mark_all_read(&mut self, cx: &mut Context<Self>) {
        self.confirming = false;
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" {
            if self.confirming {
                self.cancel_mark_all_read(cx);
            } else {
                cx.emit(AggregateEvent::Close);
            }
        }
    }
}

impl<S: AggregateSource + 'static> Focusable for AggregateView<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// A small text button.
pub(crate) fn button(
    id: impl Into<gpui::ElementId>,
    label: String,
    color: Hsla,
    on: impl Fn(&mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .cursor_pointer()
        .text_xs()
        .text_color(color)
        .child(label)
        .on_click(move |_, w, cx| on(w, cx))
}

impl<S: AggregateSource + 'static> Render for AggregateView<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use themes::metrics as m;
        let theme = cx.theme().clone();
        let colors = theme.colors();
        // Read once, outside the row loop: a long list reads nothing per row.
        let tokens = parla_tokens(&theme);
        let error = theme.status().error;
        let count = self.rows.len();
        let entries: Vec<AggregateEntry> = self.rows.iter().take(self.shown).cloned().collect();
        let this = cx.entity();
        let (title, empty) = match self.kind {
            AggregateKind::Unreads => (
                t!(cx, "aggregate-unreads-title").into_owned(),
                t!(cx, "aggregate-unreads-empty").into_owned(),
            ),
            AggregateKind::Mentions => (
                t!(cx, "aggregate-mentions-title").into_owned(),
                t!(cx, "aggregate-mentions-empty").into_owned(),
            ),
            AggregateKind::Scheduled => (
                t!(cx, "aggregate-scheduled-title").into_owned(),
                t!(cx, "scheduled-empty").into_owned(),
            ),
        };
        let mut body = div()
            .id("aggregate-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb(m::SPACE_5)
            .bg(colors.background);
        if count == 0 {
            body = body.child(list_note(empty, colors));
        }
        let more = count.saturating_sub(self.shown);
        let hover = colors.element_hover;
        let unreads = self.kind == AggregateKind::Unreads;
        let mark_label: SharedString = t!(cx, "aggregate-mark-read").into_owned().into();
        for (i, e) in entries.into_iter().enumerate() {
            let v = this.clone();
            let (open_ws, open_target) = (e.workspace.key.clone(), e.target.clone());
            let accent = e.workspace.accent.unwrap_or(colors.text_accent);
            let emphasised = e.unread || e.mentions > 0;
            let has_body = !e.lines.is_empty() || e.tag.is_some() || !e.previews.is_empty();
            let painted = match (&e.target, &e.latest) {
                (AggregateTarget::Conversation(c), Some(t)) if unreads => {
                    Some((e.workspace.key.clone(), c.clone(), t.clone()))
                }
                _ => None,
            };
            let mark = painted.map(|(ws, c, latest)| {
                let button = crate::controls::no_focus_on_click(
                    crate::controls::button_small(
                        ("aggregate-mark-read", i),
                        mark_label.clone(),
                        crate::controls::ButtonKind::Secondary,
                        colors,
                    )
                    .flex_none()
                    // Icon first, as on the board: the label is added first.
                    .flex_row_reverse()
                    .gap(m::SPACE_2)
                    .debug_selector(move || format!("aggregate-mark-read-{i}")),
                )
                .child(
                    crate::icon::Icon::new(crate::icon::IconName::Check)
                        .size(m::ICON_SIZE_SM)
                        .color(colors.text_muted),
                );
                // The row opens on a click; the button must not.
                crate::controls::on_activate(button, cx, move |v, _, cx| {
                    cx.stop_propagation();
                    v.mark_read(ws.clone(), c.clone(), latest.clone(), cx);
                })
            });
            let mut row = div()
                .id(("aggregate-row", i))
                .debug_selector(move || format!("aggregate-row-{i}"))
                .flex()
                .flex_col()
                .mx(m::SPACE_7)
                .mt(m::SPACE_5)
                .rounded(m::RADIUS_LG)
                .border_1()
                .border_color(colors.border)
                .bg(colors.surface_background)
                .cursor_pointer()
                .hover(move |d| d.bg(hover))
                .on_click(move |_, _, cx| {
                    let (ws, target) = (open_ws.clone(), open_target.clone());
                    v.update(cx, |v, cx| v.open_target(ws, target, cx))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(m::SPACE_4)
                        .min_h(m::LAYOUT_ROW_COZY)
                        .px(m::SPACE_5)
                        .py(m::SPACE_2)
                        .when(has_body, |d| {
                            d.border_b_1().border_color(tokens.border_subtle)
                        })
                        .child(div().flex_none().size(m::SPACE_4).rounded_full().bg(accent))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(m::FONT_SIZE_MD)
                                .font_weight(if emphasised {
                                    m::FONT_WEIGHT_BOLD
                                } else {
                                    m::FONT_WEIGHT_MEDIUM
                                })
                                .child(e.title.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(m::FONT_SIZE_XS)
                                .text_color(tokens.text_subtle)
                                .child(e.workspace.name.clone()),
                        )
                        .child(div().flex_1())
                        .when(e.mentions > 0, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .min_w(m::SPACE_8)
                                    .px(m::SPACE_3)
                                    .flex()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(error)
                                    .text_color(tokens.danger_text)
                                    .text_size(m::FONT_SIZE_2XS)
                                    .font_weight(m::FONT_WEIGHT_BOLD)
                                    .child(e.mentions.to_string()),
                            )
                        })
                        .children(mark),
                );
            if has_body {
                let mut block = div()
                    .flex()
                    .flex_col()
                    .gap(m::SPACE_1)
                    .px(m::SPACE_5)
                    .py(m::SPACE_4);
                for (n, p) in e.previews.into_iter().enumerate() {
                    block = block.child(
                        div()
                            .debug_selector(move || format!("aggregate-preview-{i}-{n}"))
                            .flex()
                            .gap(m::SPACE_4)
                            .min_w_0()
                            .child(div().flex_none().mt(m::SPACE_1).child(
                                crate::message_row::avatar_sized(
                                    &p.author,
                                    &theme,
                                    m::LAYOUT_AVATAR_XS,
                                    p.avatar.as_ref(),
                                ),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex()
                                            .items_baseline()
                                            .gap(m::SPACE_4)
                                            .child(
                                                div()
                                                    .text_size(m::FONT_SIZE_SM)
                                                    .font_weight(m::FONT_WEIGHT_SEMIBOLD)
                                                    .child(p.author),
                                            )
                                            .child(
                                                div()
                                                    .text_size(m::FONT_SIZE_XS)
                                                    .text_color(tokens.text_subtle)
                                                    .child(p.time),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(m::FONT_SIZE_SM)
                                            .text_color(colors.text)
                                            .truncate()
                                            .child(p.text),
                                    ),
                            ),
                    );
                }
                for (n, line) in e.lines.into_iter().enumerate() {
                    block = block.child(if n == 0 {
                        div()
                            .text_size(m::FONT_SIZE_SM)
                            .text_color(colors.text)
                            .child(line)
                    } else {
                        div()
                            .text_size(m::FONT_SIZE_XS)
                            .text_color(colors.text_muted)
                            .child(line)
                    });
                }
                if let Some(tag) = e.tag {
                    let color = if e.failed { error } else { colors.text_muted };
                    block = block.child(
                        div()
                            .text_size(m::FONT_SIZE_XS)
                            .text_color(color)
                            .child(tag),
                    );
                }
                row = row.child(block);
            }
            body = body.child(row);
        }
        if more > 0 {
            let v = this.clone();
            body = body.child(
                div()
                    .debug_selector(|| "aggregate-show-more".into())
                    .px(m::SPACE_7)
                    .pt(m::SPACE_5)
                    .child(button(
                        "aggregate-show-more",
                        t!(cx, "aggregate-show-more", count = more).into_owned(),
                        colors.text_accent,
                        move |_, cx| v.update(cx, |v, cx| v.show_more(cx)),
                    )),
            );
        }
        let this_req = this.clone();
        let (t_yes, t_no) = (this.clone(), this);
        let mark =
            (self.kind == AggregateKind::Unreads && count > 0 && !self.confirming).then(|| {
                button(
                    "aggregate-mark-all",
                    t!(cx, "aggregate-mark-all-read").into_owned(),
                    colors.text_accent,
                    move |_, cx| this_req.update(cx, |v, cx| v.request_mark_all_read(cx)),
                )
            });
        let confirm = self.confirming.then(|| {
            div()
                .debug_selector(|| "aggregate-confirm".into())
                .flex()
                .items_center()
                .gap(m::SPACE_5)
                .px(m::SPACE_7)
                .py(m::SPACE_4)
                .bg(colors.surface_background)
                .border_b_1()
                .border_color(tokens.border_subtle)
                .text_size(m::FONT_SIZE_SM)
                .child(t!(cx, "aggregate-confirm-mark", count = count).into_owned())
                .child(button(
                    "aggregate-confirm-yes",
                    t!(cx, "aggregate-confirm").into_owned(),
                    colors.text_accent,
                    move |_, cx| t_yes.update(cx, |v, cx| v.confirm_mark_all_read(cx)),
                ))
                .child(button(
                    "aggregate-confirm-no",
                    t!(cx, "scheduled-cancel").into_owned(),
                    colors.text_muted,
                    move |_, cx| t_no.update(cx, |v, cx| v.cancel_mark_all_read(cx)),
                ))
        });
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_between()
                    .items_center()
                    .gap(m::SPACE_5)
                    .h(m::LAYOUT_HEADER)
                    .px(m::SPACE_7)
                    .bg(colors.surface_background)
                    .border_b_1()
                    .border_color(tokens.border_subtle)
                    .child(
                        div()
                            .flex_1()
                            .text_size(m::FONT_SIZE_XL)
                            .font_weight(m::FONT_WEIGHT_BOLD)
                            .child(title),
                    )
                    .children(mark)
                    .child(crate::controls::icon_button(
                        "aggregate-close",
                        crate::icon::IconName::X,
                        t!(cx, "close-label").into_owned(),
                        colors,
                        {
                            let this = cx.entity();
                            move |_, cx| this.update(cx, |_, cx| cx.emit(AggregateEvent::Close))
                        },
                    )),
            )
            .children(confirm)
            .child(body)
    }
}
