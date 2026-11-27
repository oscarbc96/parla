//! The pins popover under the header: message previews, unpin, click to jump.

use domain::{ConversationId, Input, MessageId};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    Subscription, Window, div, prelude::*,
};
use themes::{ActiveTheme, parla_tokens};

use crate::saved_view::{
    CollectionEvent, CollectionsSource, Preview, PreviewCache, list_note, preview, row_shell,
};

pub struct PinsPopover<S: CollectionsSource + 'static> {
    session: Entity<S>,
    conversation: ConversationId,
    focus: FocusHandle,
    previews: PreviewCache,
    _observe: Subscription,
}

impl<S: CollectionsSource + 'static> EventEmitter<CollectionEvent> for PinsPopover<S> {}

impl<S: CollectionsSource + 'static> PinsPopover<S> {
    pub fn new(session: Entity<S>, conversation: ConversationId, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&session, |_, _, cx| cx.notify());
        let c = conversation.clone();
        session.update(cx, |s, cx| s.dispatch(Input::OpenPins(c), cx));
        Self {
            session,
            conversation,
            focus: cx.focus_handle(),
            previews: PreviewCache::default(),
            _observe: observe,
        }
    }

    pub fn conversation(&self) -> &ConversationId {
        &self.conversation
    }

    /// Newest pin first; `None` while the list loads.
    pub fn rows(&self, cx: &App) -> Option<Vec<Preview>> {
        let s = self.session.read(cx);
        let pins = s.pins(&self.conversation)?;
        let mut sorted: Vec<_> = pins.iter().collect();
        sorted.sort_by_key(|p| std::cmp::Reverse(p.at));
        Some(
            sorted
                .iter()
                .map(|p| preview(s, &p.message, &self.previews, cx))
                .collect(),
        )
    }

    pub fn unpin(&mut self, id: MessageId, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            s.dispatch(Input::SetPinned { id, pinned: false }, cx)
        });
    }

    /// Jumps to the message and closes the popover.
    pub fn jump(&mut self, id: MessageId, cx: &mut Context<Self>) {
        cx.emit(CollectionEvent::OpenMessage(id));
        cx.emit(CollectionEvent::Close);
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" {
            cx.emit(CollectionEvent::Close);
        }
    }
}

impl<S: CollectionsSource + 'static> Focusable for PinsPopover<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: CollectionsSource + 'static> Render for PinsPopover<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let tokens = parla_tokens(&theme);
        let hover = colors.element_hover;
        let mut body = div()
            .id("pins-list")
            .debug_selector(|| "pins-list".into())
            .flex()
            .flex_col()
            .gap(themes::metrics::SPACE_4)
            .p(themes::metrics::SPACE_5)
            .max_h(themes::metrics::LAYOUT_POPOVER_MAX_HEIGHT)
            .overflow_y_scroll()
            .rounded_b(themes::metrics::RADIUS_LG)
            .bg(colors.background);
        match self.rows(cx) {
            None => {
                body = body.child(list_note(
                    t!(cx, "collections-loading").into_owned(),
                    colors,
                ))
            }
            Some(rows) if rows.is_empty() => {
                body = body.child(
                    list_note(t!(cx, "pins-empty").into_owned(), colors)
                        .debug_selector(|| "pins-empty".into()),
                )
            }
            Some(rows) => {
                for (ix, p) in rows.into_iter().enumerate() {
                    let id = p.message.clone();
                    let unpin_id = id.clone();
                    body = body.child(
                        div()
                            .id(("pins-row", ix))
                            .debug_selector(move || format!("pins-row-{ix}"))
                            .flex()
                            .gap(themes::metrics::SPACE_4)
                            .justify_between()
                            .p(themes::metrics::SPACE_5)
                            .rounded(themes::metrics::RADIUS_MD)
                            .border_1()
                            .border_color(tokens.border_subtle)
                            .bg(colors.surface_background)
                            .cursor_pointer()
                            .hover(move |d| d.bg(hover))
                            .child(div().flex_1().min_w_0().child(row_shell(
                                &p,
                                None,
                                colors,
                                tokens.text_subtle,
                            )))
                            .child(
                                div()
                                    .id(("pins-unpin", ix))
                                    .debug_selector(move || format!("pins-unpin-{ix}"))
                                    .flex_none()
                                    .text_size(themes::metrics::FONT_SIZE_XS)
                                    .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                                    .text_color(colors.text_accent)
                                    .child(t!(cx, "pins-unpin").into_owned())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.unpin(unpin_id.clone(), cx)
                                    })),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.jump(id.clone(), cx))),
                    );
                }
            }
        }
        div()
            .key_context(crate::saved_view::KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .debug_selector(|| "pins-popover".into())
            .w(themes::metrics::LAYOUT_POPOVER_LG)
            .flex()
            .flex_col()
            .rounded(themes::metrics::RADIUS_LG)
            .overflow_hidden()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .shadow_md()
            .child(
                div()
                    .px(themes::metrics::SPACE_6)
                    .pt(themes::metrics::SPACE_5)
                    .pb(themes::metrics::SPACE_4)
                    .rounded_t(themes::metrics::RADIUS_LG)
                    .border_b_1()
                    .border_color(tokens.border_subtle)
                    .text_size(themes::metrics::FONT_SIZE_LG)
                    .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                    .child(t!(cx, "pins-title").into_owned()),
            )
            .child(body)
    }
}
