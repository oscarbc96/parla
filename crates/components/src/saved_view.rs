//! The per-workspace "Later" view (saved items), plus what the four
//! collection views (pins, bookmarks, Later, Threads) need from the session.

use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::sync::Arc;

use domain::{
    Bookmark, ContentParser, ConversationId, InboxThread, Input, Member, Message, MessageId,
    MessageTs, Page, Pin, PlainParser, SavedItem, UserId,
};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    ScrollHandle, SharedString, Subscription, Window, div, prelude::*, px,
};
use i18n::Translate as _;
use lru::LruCache;
use themes::{ActiveTheme, parla_tokens};

use crate::message_row::{author_name, format_date_time};
use crate::send_later::SendLaterConfig;

/// Key context of the collection views.
pub const KEY_CONTEXT: &str = "CollectionView";

/// Longest preview of a message, in characters.
const PREVIEW_CHARS: usize = 160;

/// What the collection views need from the session, so `components` does not
/// depend on `parla`.
pub trait CollectionsSource {
    /// `None` while the list loads (or the provider has none).
    fn pins(&self, c: &ConversationId) -> Option<&[Pin]>;
    fn bookmarks(&self, c: &ConversationId) -> Option<&[Bookmark]>;
    fn saved(&self) -> Option<&Page<SavedItem>>;
    fn thread_inbox(&self) -> Option<&Page<InboxThread>>;
    /// A message fetched for one of the lists (or held by a window).
    fn referenced(&self, id: &MessageId) -> Option<&Message>;
    fn member(&self, u: &UserId) -> Option<&Member>;
    /// The conversation's name as the sidebar shows it.
    fn conversation_label(&self, _c: &ConversationId, _cx: &App) -> String {
        String::new()
    }
    fn content(&self) -> Arc<dyn ContentParser> {
        Arc::new(PlainParser)
    }
    /// Zone and clock the times are shown in.
    fn time_config(&self, cx: &App) -> SendLaterConfig;
    /// Sends a domain input (`OpenSaved`, `SetPinned`, ...) to the session.
    fn dispatch(&mut self, input: Input, cx: &mut App);
    /// Opens a link outside the app. The caller has checked it is safe.
    fn open_external(&mut self, url: &str, cx: &mut App) {
        cx.open_url(url);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectionEvent {
    Close,
    /// Show the message in its conversation.
    OpenMessage(MessageId),
    /// Show the thread panel under the root.
    OpenThread(MessageId),
}

/// A message as a list row shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub message: MessageId,
    pub author: SharedString,
    pub text: String,
    pub time: String,
    /// The message is still being fetched.
    pub loading: bool,
}

/// Parsed, truncated preview text per `(message, edited)`, bounded like
/// `RenderCache`, so a list re-render does not re-parse every row.
type PreviewKey = (MessageId, Option<MessageTs>);

pub struct PreviewCache {
    text: RefCell<LruCache<PreviewKey, Arc<str>>>,
}

impl Default for PreviewCache {
    fn default() -> Self {
        Self {
            text: RefCell::new(LruCache::new(
                NonZeroUsize::new(crate::render_cache::CAPACITY).expect("non-zero"),
            )),
        }
    }
}

/// The preview of `id`; a placeholder while the message is not held.
pub fn preview<S: CollectionsSource + ?Sized>(
    s: &S,
    id: &MessageId,
    cache: &PreviewCache,
    cx: &App,
) -> Preview {
    let l = cx.localizer();
    let cfg = s.time_config(cx);
    let time = format_date_time(&id.ts, cfg.clock, &cfg.zone, l);
    match s.referenced(id) {
        Some(m) => {
            let key = (id.clone(), m.edited.clone());
            let cached = cache.text.borrow_mut().get(&key).cloned();
            let text = cached.unwrap_or_else(|| {
                let plain = s.content().parse_text(&m.text).plain_text();
                let mut text: String = plain.chars().take(PREVIEW_CHARS).collect();
                if plain.chars().count() > PREVIEW_CHARS {
                    text.push('\u{2026}');
                }
                let text: Arc<str> = text.into();
                cache.text.borrow_mut().put(key, text.clone());
                text
            });
            Preview {
                message: id.clone(),
                author: author_name(m.author.as_ref().and_then(|u| s.member(u)), l),
                text: text.to_string(),
                time,
                loading: false,
            }
        }
        None => Preview {
            message: id.clone(),
            author: SharedString::default(),
            text: t!(l, "collections-loading-message").into_owned(),
            time,
            loading: true,
        },
    }
}

/// One saved item as the Later view shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedRow {
    pub preview: Preview,
    pub conversation: String,
    /// The formatted due date, when the item has one.
    pub due: Option<String>,
    pub done: bool,
}

pub struct SavedView<S: CollectionsSource + 'static> {
    session: Entity<S>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    previews: PreviewCache,
    _observe: Subscription,
}

impl<S: CollectionsSource + 'static> EventEmitter<CollectionEvent> for SavedView<S> {}

impl<S: CollectionsSource + 'static> SavedView<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&session, |_, _, cx| cx.notify());
        session.update(cx, |s, cx| s.dispatch(Input::OpenSaved, cx));
        Self {
            session,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            previews: PreviewCache::default(),
            _observe: observe,
        }
    }

    /// Open items first (newest save first), then done ones; `None` while loading.
    pub fn rows(&self, cx: &App) -> Option<Vec<SavedRow>> {
        let s = self.session.read(cx);
        let page = s.saved()?;
        let cfg = s.time_config(cx);
        let l = cx.localizer();
        let mut rows: Vec<(i64, SavedRow)> = page
            .items
            .iter()
            .map(|i| {
                let due = i
                    .due
                    .and_then(|d| jiff::Timestamp::from_second(d).ok())
                    .map(|t| t!(l, "saved-due", date = l.date(t, &cfg.zone)).into_owned());
                (
                    i.saved_at,
                    SavedRow {
                        preview: preview(s, &i.message, &self.previews, cx),
                        conversation: s.conversation_label(&i.message.conversation, cx),
                        due,
                        done: i.done,
                    },
                )
            })
            .collect();
        rows.sort_by_key(|(at, r)| (r.done, std::cmp::Reverse(*at)));
        Some(rows.into_iter().map(|(_, r)| r).collect())
    }

    /// More items exist beyond the loaded pages.
    pub fn has_more(&self, cx: &App) -> bool {
        self.session
            .read(cx)
            .saved()
            .is_some_and(|p| p.next.is_some())
    }

    /// Loads the next page, when there is one.
    pub fn more(&mut self, cx: &mut Context<Self>) {
        if self.has_more(cx) {
            self.session
                .update(cx, |s, cx| s.dispatch(Input::MoreSaved, cx));
        }
    }

    /// Takes the message off the saved list.
    pub fn remove(&mut self, id: MessageId, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| {
            s.dispatch(Input::SetSaved { id, saved: false }, cx)
        });
    }

    pub fn open(&mut self, id: MessageId, cx: &mut Context<Self>) {
        cx.emit(CollectionEvent::OpenMessage(id));
    }

    fn near_bottom(&self) -> bool {
        let max = self.scroll.max_offset().y;
        max > px(0.0) && self.scroll.offset().y <= -(max - px(48.0))
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" {
            cx.emit(CollectionEvent::Close);
        }
    }
}

impl<S: CollectionsSource + 'static> Focusable for SavedView<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The title strip with a close button, shared by Later and Threads: the
/// board's 56 px header on the surface ground.
pub(crate) fn title_bar(
    id: &'static str,
    title: String,
    close_label: String,
    theme: &themes::Theme,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> gpui::Div {
    let colors = theme.colors();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .h(themes::metrics::LAYOUT_HEADER)
        .px(themes::metrics::SPACE_7)
        .bg(colors.surface_background)
        .border_b_1()
        .border_color(parla_tokens(theme).border_subtle)
        .child(
            div()
                .text_size(themes::metrics::FONT_SIZE_XL)
                .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                .child(title),
        )
        .child(
            crate::controls::icon_button(
                id,
                crate::icon::IconName::X,
                close_label,
                colors,
                on_close,
            )
            .debug_selector(move || id.to_string()),
        )
}

/// The ground of a list under a [`title_bar`]: the app background, with the
/// row cards on it.
pub(crate) fn list_ground(
    id: &'static str,
    colors: &themes::ThemeColors,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .pb(themes::metrics::SPACE_5)
        .bg(colors.background)
}

/// One row card of a list: surface fill, hairline border, large radius, the
/// hover fill on pointer.
pub(crate) fn row_card(
    id: impl Into<gpui::ElementId>,
    theme: &themes::Theme,
) -> gpui::Stateful<gpui::Div> {
    let colors = theme.colors();
    let hover = colors.element_hover;
    div()
        .id(id)
        .flex()
        .gap(themes::metrics::SPACE_4)
        .justify_between()
        .mx(themes::metrics::SPACE_7)
        .mt(themes::metrics::SPACE_5)
        .px(themes::metrics::SPACE_6)
        .py(themes::metrics::SPACE_5)
        .rounded(themes::metrics::RADIUS_LG)
        .border_1()
        .border_color(colors.border)
        .bg(colors.surface_background)
        .cursor_pointer()
        .hover(move |d| d.bg(hover))
}

/// A list's empty or loading line, centred on the ground.
pub(crate) fn list_note(text: String, colors: &themes::ThemeColors) -> gpui::Div {
    div()
        .flex()
        .justify_center()
        .px(themes::metrics::SPACE_7)
        .py(themes::metrics::SPACE_9)
        .text_size(themes::metrics::FONT_SIZE_SM)
        .text_color(colors.text_muted)
        .child(text)
}

/// A row: the author and time, then the text, with the muted `meta` line.
pub(crate) fn row_shell(
    p: &Preview,
    meta: Option<String>,
    colors: &themes::ThemeColors,
    subtle: gpui::Hsla,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(themes::metrics::SPACE_1)
        .child(
            div()
                .flex()
                .gap(themes::metrics::SPACE_4)
                .items_baseline()
                .child(
                    div()
                        .text_size(themes::metrics::FONT_SIZE_SM)
                        .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                        .child(p.author.clone()),
                )
                .child(
                    div()
                        .text_size(themes::metrics::FONT_SIZE_XS)
                        .text_color(subtle)
                        .child(p.time.clone()),
                ),
        )
        .child(
            div()
                .text_size(themes::metrics::FONT_SIZE_SM)
                .text_color(if p.loading {
                    colors.text_muted
                } else {
                    colors.text
                })
                .child(p.text.clone()),
        )
        .children(meta.map(|m| {
            div()
                .text_size(themes::metrics::FONT_SIZE_XS)
                .text_color(colors.text_muted)
                .child(m)
        }))
}

impl<S: CollectionsSource + 'static> Render for SavedView<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let subtle = parla_tokens(&theme).text_subtle;
        let rows = self.rows(cx);
        let has_more = self.has_more(cx);
        let mut body = list_ground("saved-list", colors)
            .debug_selector(|| "saved-list".into())
            .track_scroll(&self.scroll)
            .on_scroll(cx.listener(|this, _, _, cx| {
                if this.near_bottom() {
                    this.more(cx);
                }
            }));
        match rows {
            None => {
                body = body.child(list_note(
                    t!(cx, "collections-loading").into_owned(),
                    colors,
                ));
            }
            Some(rows) if rows.is_empty() => {
                body = body.child(
                    list_note(t!(cx, "saved-empty").into_owned(), colors)
                        .debug_selector(|| "saved-empty".into()),
                );
            }
            Some(rows) => {
                for (ix, r) in rows.into_iter().enumerate() {
                    let id = r.preview.message.clone();
                    let remove_id = id.clone();
                    let meta = [
                        Some(r.conversation.clone()).filter(|c| !c.is_empty()),
                        r.due.clone(),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" \u{b7} ");
                    body = body.child(
                        row_card(("saved-row", ix), &theme)
                            .debug_selector(move || format!("saved-row-{ix}"))
                            .when(r.done, |d| d.opacity(0.6))
                            .child(div().flex_1().min_w_0().child(row_shell(
                                &r.preview,
                                Some(meta).filter(|m| !m.is_empty()),
                                colors,
                                subtle,
                            )))
                            .child(
                                div()
                                    .id(("saved-remove", ix))
                                    .debug_selector(move || format!("saved-remove-{ix}"))
                                    .flex_none()
                                    .text_xs()
                                    .text_color(colors.text_accent)
                                    .child(t!(cx, "saved-remove").into_owned())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.remove(remove_id.clone(), cx)
                                    })),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.open(id.clone(), cx))),
                    );
                }
                if has_more {
                    body = body.child(
                        div()
                            .id("saved-more")
                            .debug_selector(|| "saved-more".into())
                            .px(themes::metrics::SPACE_7)
                            .pt(themes::metrics::SPACE_5)
                            .text_size(themes::metrics::FONT_SIZE_SM)
                            .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                            .text_color(colors.text_accent)
                            .cursor_pointer()
                            .child(t!(cx, "collections-more").into_owned())
                            .on_click(cx.listener(|this, _, _, cx| this.more(cx))),
                    );
                }
            }
        }
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .flex()
            .flex_col()
            .text_color(colors.text)
            .child(title_bar(
                "saved-close",
                t!(cx, "saved-title").into_owned(),
                t!(cx, "close-label").into_owned(),
                &theme,
                {
                    let this = cx.entity();
                    move |_, cx| this.update(cx, |_, cx| cx.emit(CollectionEvent::Close))
                },
            ))
            .child(body)
    }
}
