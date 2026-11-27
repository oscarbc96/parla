//! The per-workspace Drafts view: every saved conversation and thread draft,
//! newest first. A row opens its conversation (or thread) with the draft in
//! the composer; the trash button deletes it.

use domain::DraftKey;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Subscription,
    Window, div, prelude::*,
};
use themes::ActiveTheme;
use themes::metrics::{
    FONT_SIZE_SM, FONT_SIZE_XL, FONT_SIZE_XS, FONT_WEIGHT_BOLD, FONT_WEIGHT_SEMIBOLD,
    ICON_SIZE_BASE, ICON_SIZE_LG, LAYOUT_HEADER, RADIUS_FULL, SPACE_1, SPACE_3, SPACE_4, SPACE_5,
    SPACE_6,
};

use crate::icon::{Icon, IconName};

/// Key context of the view.
pub const KEY_CONTEXT: &str = "DraftsView";

/// Where a draft belongs, for its row icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftKind {
    Channel,
    PrivateChannel,
    Direct,
    Thread,
}

/// One saved draft as the session reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftItem {
    pub key: DraftKey,
    /// The conversation's sidebar label ("#design", "Ana").
    pub conversation: String,
    pub kind: DraftKind,
    pub text: String,
    /// Last edit, Unix seconds; 0 when unknown.
    pub edited: i64,
}

/// A row as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftRow {
    pub key: DraftKey,
    pub kind: DraftKind,
    /// "#design" or "Thread in #design".
    pub place: String,
    /// "Edited 5 minutes ago", empty when the time is unknown.
    pub when: String,
    /// The text on one line.
    pub preview: String,
}

/// What the view needs from the session, so `components` does not depend on `parla`.
pub trait DraftsSource {
    fn draft_items(&self, cx: &App) -> Vec<DraftItem>;
    fn delete_draft(&mut self, key: DraftKey, cx: &mut App);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftsEvent {
    Close,
    /// Open the conversation or thread of this draft, draft in the composer.
    Open(DraftKey),
}

pub struct DraftsView<S: DraftsSource + 'static> {
    session: Entity<S>,
    focus: FocusHandle,
    /// One tab stop per row, grown as rows appear.
    rows_focus: Vec<FocusHandle>,
    _observe: Subscription,
}

impl<S: DraftsSource + 'static> EventEmitter<DraftsEvent> for DraftsView<S> {}

impl<S: DraftsSource + 'static> DraftsView<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            focus: cx.focus_handle(),
            rows_focus: Vec::new(),
            _observe: observe,
        }
    }

    /// Row `n`'s focus handle (Enter on it opens the draft).
    pub fn row_focus(&self, n: usize) -> FocusHandle {
        self.rows_focus[n].clone()
    }

    /// The rows, most recently edited first (ties by conversation, then thread).
    pub fn rows(&self, cx: &App) -> Vec<DraftRow> {
        let l = i18n::Translate::localizer(cx);
        let now = crate::now(cx);
        let mut items = self.session.read(cx).draft_items(cx);
        items.sort_by(|a, b| {
            b.edited
                .cmp(&a.edited)
                .then_with(|| a.key.conversation.cmp(&b.key.conversation))
                .then_with(|| a.key.thread.cmp(&b.key.thread))
        });
        items
            .into_iter()
            .map(|i| {
                let place = match i.kind {
                    DraftKind::Thread => t!(
                        l,
                        "drafts-thread-in",
                        conversation = i.conversation.as_str()
                    )
                    .into_owned(),
                    _ => i.conversation.clone(),
                };
                let when = jiff::Timestamp::from_second(i.edited)
                    .ok()
                    .filter(|_| i.edited > 0)
                    .map(|t| {
                        t!(l, "drafts-edited", when = l.relative(t, now).as_str()).into_owned()
                    })
                    .unwrap_or_default();
                DraftRow {
                    key: i.key,
                    kind: i.kind,
                    place,
                    when,
                    preview: i.text.split_whitespace().collect::<Vec<_>>().join(" "),
                }
            })
            .collect()
    }
}

impl<S: DraftsSource + 'static> Focusable for DraftsView<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: DraftsSource + 'static> Render for DraftsView<S> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let rows = self.rows(cx);
        let count = rows.len();
        while self.rows_focus.len() < count {
            self.rows_focus.push(cx.focus_handle().tab_stop(true));
        }
        let delete_label = t!(cx, "drafts-delete").into_owned();
        let mut list = div()
            .id("drafts-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if rows.is_empty() {
            list = list.child(
                crate::controls::empty_state(
                    t!(cx, "drafts-empty-title").into_owned(),
                    t!(cx, "drafts-empty").into_owned(),
                    colors,
                )
                .id("drafts-empty")
                .debug_selector(|| "drafts-empty".to_string()),
            );
        }
        for (n, r) in rows.into_iter().enumerate() {
            let icon = match r.kind {
                DraftKind::Channel => IconName::Hash,
                DraftKind::PrivateChannel => IconName::Lock,
                DraftKind::Direct => IconName::User,
                DraftKind::Thread => IconName::Threads,
            };
            let (open, open_key, delete) = (r.key.clone(), r.key.clone(), r.key);
            let session = self.session.clone();
            list = list.child(
                div()
                    .id(("drafts-row", n))
                    .debug_selector(move || format!("drafts-row-{n}"))
                    .flex()
                    .gap(SPACE_4)
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .border_b_1()
                    .border_color(tokens.border_subtle)
                    .cursor_pointer()
                    .track_focus(&self.rows_focus[n])
                    .focus_visible(crate::controls::focus_style(colors))
                    .hover(|h| h.bg(colors.element_hover))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(DraftsEvent::Open(open.clone()));
                    }))
                    .on_key_down(cx.listener(move |_, ev: &gpui::KeyDownEvent, _, cx| {
                        let k = &ev.keystroke;
                        if !(k.modifiers.platform || k.modifiers.control || k.modifiers.alt)
                            && matches!(k.key.as_str(), "enter" | "space")
                        {
                            cx.stop_propagation();
                            cx.emit(DraftsEvent::Open(open_key.clone()));
                        }
                    }))
                    .child(
                        div()
                            .flex_none()
                            .mt(SPACE_1)
                            .child(Icon::new(icon).size(ICON_SIZE_BASE)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(SPACE_1)
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(SPACE_4)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(FONT_SIZE_SM)
                                            .font_weight(FONT_WEIGHT_BOLD)
                                            .text_color(colors.text)
                                            .child(r.place),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_size(FONT_SIZE_XS)
                                            .text_color(tokens.text_subtle)
                                            .child(r.when),
                                    ),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(FONT_SIZE_SM)
                                    .text_color(colors.text_muted)
                                    .child(r.preview),
                            ),
                    )
                    .child(
                        crate::controls::icon_button_sized(
                            ("drafts-delete", n),
                            IconName::Trash,
                            delete_label.clone(),
                            crate::controls::IconButtonSize::Large,
                            colors,
                            move |_, cx| {
                                // The row's click must not open the draft too.
                                cx.stop_propagation();
                                session.update(cx, |s, cx| s.delete_draft(delete.clone(), cx));
                            },
                        )
                        .debug_selector(move || format!("drafts-delete-{n}")),
                    ),
            );
        }
        if count > 0 {
            list = list.child(
                div()
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .text_size(FONT_SIZE_XS)
                    .text_color(tokens.text_subtle)
                    .child(t!(cx, "drafts-note").into_owned()),
            );
        }
        div()
            .id("drafts-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, ev: &gpui::KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    cx.stop_propagation();
                    cx.emit(DraftsEvent::Close);
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.surface_background)
            .text_color(colors.text)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(SPACE_4)
                    .h(LAYOUT_HEADER)
                    .px(SPACE_6)
                    .border_b_1()
                    .border_color(tokens.border_subtle)
                    .child(Icon::new(IconName::Pencil).size(ICON_SIZE_LG))
                    .child(
                        div()
                            .flex_1()
                            .text_size(FONT_SIZE_XL)
                            .font_weight(FONT_WEIGHT_BOLD)
                            .child(t!(cx, "drafts-title").into_owned()),
                    )
                    .when(count > 0, |d| {
                        d.child(
                            div()
                                .debug_selector(|| "drafts-count".to_string())
                                .px(SPACE_3)
                                .rounded(RADIUS_FULL)
                                .bg(colors.element_selected)
                                .text_size(FONT_SIZE_XS)
                                .font_weight(FONT_WEIGHT_SEMIBOLD)
                                .text_color(colors.text_muted)
                                .child(count.to_string()),
                        )
                    })
                    .child(crate::controls::icon_button(
                        "drafts-close",
                        IconName::X,
                        t!(cx, "close-label").into_owned(),
                        colors,
                        {
                            let this = cx.entity();
                            move |_, cx| this.update(cx, |_, cx| cx.emit(DraftsEvent::Close))
                        },
                    )),
            )
            .child(list)
    }
}
