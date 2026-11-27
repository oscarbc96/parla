//! The per-workspace Threads view: the threads the user follows, newest
//! activity first, with their unread replies.

use domain::{Input, MessageId};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    ScrollHandle, Subscription, Window, div, prelude::*, px,
};
use i18n::Translate as _;
use themes::ActiveTheme;

use crate::saved_view::{
    CollectionEvent, CollectionsSource, Preview, PreviewCache, list_ground, list_note, preview,
    row_card, row_shell, title_bar,
};

/// A followed thread as the view shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadRow {
    pub root: Preview,
    pub conversation: String,
    pub unread_replies: u32,
    pub mentions: u32,
}

pub struct ThreadsView<S: CollectionsSource + 'static> {
    session: Entity<S>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    previews: PreviewCache,
    _observe: Subscription,
}

impl<S: CollectionsSource + 'static> EventEmitter<CollectionEvent> for ThreadsView<S> {}

impl<S: CollectionsSource + 'static> ThreadsView<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&session, |_, _, cx| cx.notify());
        session.update(cx, |s, cx| s.dispatch(Input::OpenThreadInbox, cx));
        Self {
            session,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            previews: PreviewCache::default(),
            _observe: observe,
        }
    }

    /// Threads with unread replies first, then by latest activity; `None` while loading.
    pub fn rows(&self, cx: &App) -> Option<Vec<ThreadRow>> {
        let s = self.session.read(cx);
        let page = s.thread_inbox()?;
        let mut items: Vec<_> = page.items.iter().collect();
        items.sort_by(|a, b| {
            (b.unread_replies > 0)
                .cmp(&(a.unread_replies > 0))
                .then_with(|| b.latest.0.cmp(&a.latest.0))
        });
        Some(
            items
                .into_iter()
                .map(|t| ThreadRow {
                    root: preview(s, &t.root, &self.previews, cx),
                    conversation: s.conversation_label(&t.root.conversation, cx),
                    unread_replies: t.unread_replies,
                    mentions: t.mentions,
                })
                .collect(),
        )
    }

    /// More threads exist beyond the loaded pages.
    pub fn has_more(&self, cx: &App) -> bool {
        self.session
            .read(cx)
            .thread_inbox()
            .is_some_and(|p| p.next.is_some())
    }

    /// Loads the next page, when there is one.
    pub fn more(&mut self, cx: &mut Context<Self>) {
        if self.has_more(cx) {
            self.session
                .update(cx, |s, cx| s.dispatch(Input::MoreThreadInbox, cx));
        }
    }

    fn near_bottom(&self) -> bool {
        let max = self.scroll.max_offset().y;
        max > px(0.0) && self.scroll.offset().y <= -(max - px(48.0))
    }

    /// Opens the thread panel on `root`.
    pub fn open(&mut self, root: MessageId, cx: &mut Context<Self>) {
        cx.emit(CollectionEvent::OpenThread(root));
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" {
            cx.emit(CollectionEvent::Close);
        }
    }
}

impl<S: CollectionsSource + 'static> Focusable for ThreadsView<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: CollectionsSource + 'static> Render for ThreadsView<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let subtle = themes::parla_tokens(&theme).text_subtle;
        let l = cx.localizer();
        let mut body = list_ground("threads-list", colors)
            .debug_selector(|| "threads-list".into())
            .track_scroll(&self.scroll)
            .on_scroll(cx.listener(|this, _, _, cx| {
                if this.near_bottom() {
                    this.more(cx);
                }
            }));
        let has_more = self.has_more(cx);
        match self.rows(cx) {
            None => {
                body = body.child(list_note(t!(l, "collections-loading").into_owned(), colors));
            }
            Some(rows) if rows.is_empty() => {
                body = body.child(
                    list_note(t!(l, "threads-empty").into_owned(), colors)
                        .debug_selector(|| "threads-empty".into()),
                );
            }
            Some(rows) => {
                for (ix, r) in rows.into_iter().enumerate() {
                    let root = r.root.message.clone();
                    let unread = (r.unread_replies > 0)
                        .then(|| t!(l, "threads-unread", count = r.unread_replies).into_owned());
                    let meta = [
                        Some(r.conversation.clone()).filter(|c| !c.is_empty()),
                        unread,
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" \u{b7} ");
                    body = body.child(
                        row_card(("threads-row", ix), &theme)
                            .debug_selector(move || format!("threads-row-{ix}"))
                            .child(row_shell(
                                &r.root,
                                Some(meta).filter(|m| !m.is_empty()),
                                colors,
                                subtle,
                            ))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.open(root.clone(), cx)),
                            ),
                    );
                }
                if has_more {
                    body = body.child(
                        div()
                            .id("threads-more")
                            .debug_selector(|| "threads-more".into())
                            .px(themes::metrics::SPACE_7)
                            .pt(themes::metrics::SPACE_5)
                            .text_size(themes::metrics::FONT_SIZE_SM)
                            .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                            .text_color(colors.text_accent)
                            .cursor_pointer()
                            .child(t!(l, "collections-more").into_owned())
                            .on_click(cx.listener(|this, _, _, cx| this.more(cx))),
                    );
                }
            }
        }
        div()
            .key_context(crate::saved_view::KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .flex()
            .flex_col()
            .text_color(colors.text)
            .child(title_bar(
                "threads-close",
                t!(l, "threads-title").into_owned(),
                t!(l, "close-label").into_owned(),
                &theme,
                {
                    let this = cx.entity();
                    move |_, cx| this.update(cx, |_, cx| cx.emit(CollectionEvent::Close))
                },
            ))
            .child(body)
    }
}
