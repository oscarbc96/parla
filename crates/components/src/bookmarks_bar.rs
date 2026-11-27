//! The bookmarks bar under the header: link chips, add and remove.
//!
//! Only `http`, `https` and `mailto` links open (and can be added); anything
//! else stays visible but inert.

use domain::{BookmarkId, ConversationId, Input};
use gpui::{
    App, Context, Entity, IntoElement, KeyDownEvent, Render, SharedString, Subscription, Window,
    div, prelude::*,
};
use themes::{ActiveTheme, parla_tokens};

use crate::links::{LinkAction, open_link};
use crate::saved_view::CollectionsSource;
use crate::text_input::TextInput;

/// One bookmark chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub id: BookmarkId,
    pub title: SharedString,
    pub link: String,
    /// The link may be opened.
    pub safe: bool,
}

pub struct BookmarksBar<S: CollectionsSource + 'static> {
    session: Entity<S>,
    conversation: ConversationId,
    adding: bool,
    invalid: bool,
    title: Entity<TextInput>,
    link: Entity<TextInput>,
    _subs: Vec<Subscription>,
}

impl<S: CollectionsSource + 'static> BookmarksBar<S> {
    pub fn new(session: Entity<S>, conversation: ConversationId, cx: &mut Context<Self>) -> Self {
        let (title, link) = (cx.new(TextInput::new), cx.new(TextInput::new));
        let (th, lh) = (
            t!(cx, "bookmarks-add-title").into_owned(),
            t!(cx, "bookmarks-add-link").into_owned(),
        );
        title.update(cx, |i, _| i.set_placeholder(th));
        link.update(cx, |i, _| i.set_placeholder(lh));
        let subs = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.observe(&title, |_, _, cx| cx.notify()),
            cx.observe(&link, |_, _, cx| cx.notify()),
        ];
        let c = conversation.clone();
        session.update(cx, |s, cx| s.dispatch(Input::OpenBookmarks(c), cx));
        Self {
            session,
            conversation,
            adding: false,
            invalid: false,
            title,
            link,
            _subs: subs,
        }
    }

    /// Shows another conversation's bookmarks.
    pub fn set_conversation(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        if self.conversation == c {
            return;
        }
        self.conversation = c.clone();
        self.adding = false;
        self.invalid = false;
        self.session
            .update(cx, |s, cx| s.dispatch(Input::OpenBookmarks(c), cx));
        cx.notify();
    }

    /// The chips in provider order; `None` while loading.
    pub fn chips(&self, cx: &App) -> Option<Vec<Chip>> {
        let list = self.session.read(cx).bookmarks(&self.conversation)?;
        Some(
            list.iter()
                .map(|b| Chip {
                    id: b.id.clone(),
                    title: SharedString::from(b.title.to_string()),
                    link: b.link.to_string(),
                    safe: matches!(open_link(&b.link), LinkAction::OpenExternal(_)),
                })
                .collect(),
        )
    }

    /// Opens the bookmark's link; false (nothing opens) for an unsafe scheme.
    pub fn open(&mut self, id: &BookmarkId, cx: &mut Context<Self>) -> bool {
        let Some(link) = self
            .session
            .read(cx)
            .bookmarks(&self.conversation)
            .and_then(|l| l.iter().find(|b| &b.id == id))
            .map(|b| b.link.clone())
        else {
            return false;
        };
        match open_link(&link) {
            LinkAction::OpenExternal(url) => {
                self.session.update(cx, |s, cx| s.open_external(&url, cx));
                true
            }
            LinkAction::Refuse => false,
        }
    }

    pub fn remove(&mut self, id: BookmarkId, cx: &mut Context<Self>) {
        let conversation = self.conversation.clone();
        self.session.update(cx, |s, cx| {
            s.dispatch(Input::RemoveBookmark { conversation, id }, cx)
        });
    }

    /// Adds a bookmark; false (and the error shown) when the title is blank
    /// or the link is not a safe one.
    pub fn add(&mut self, title: &str, link: &str, cx: &mut Context<Self>) -> bool {
        let (title, link) = (title.trim(), link.trim());
        if title.is_empty() || !matches!(open_link(link), LinkAction::OpenExternal(_)) {
            self.invalid = true;
            cx.notify();
            return false;
        }
        self.invalid = false;
        self.adding = false;
        let conversation = self.conversation.clone();
        let (title, link) = (title.into(), link.into());
        self.session.update(cx, |s, cx| {
            s.dispatch(
                Input::AddBookmark {
                    conversation,
                    title,
                    link,
                },
                cx,
            )
        });
        self.title.update(cx, |i, cx| i.set_text("", cx));
        self.link.update(cx, |i, cx| i.set_text("", cx));
        cx.notify();
        true
    }

    pub fn add_invalid(&self) -> bool {
        self.invalid
    }

    pub fn toggle_add(&mut self, cx: &mut Context<Self>) {
        self.adding = !self.adding;
        self.invalid = false;
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let (t, l) = (
            self.title.read(cx).text().to_string(),
            self.link.read(cx).text().to_string(),
        );
        self.add(&t, &l, cx);
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.adding {
            return;
        }
        match ev.keystroke.key.as_str() {
            "escape" => self.toggle_add(cx),
            "enter" => self.submit(cx),
            _ => {}
        }
    }
}

impl<S: CollectionsSource + 'static> Render for BookmarksBar<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let hover = colors.element_hover;
        let mut row = div()
            .debug_selector(|| "bookmarks-bar".into())
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(themes::metrics::SPACE_4)
            .w_full()
            .px(themes::metrics::SPACE_7)
            .py(themes::metrics::SPACE_3)
            .bg(colors.surface_background)
            .border_b_1()
            .border_color(parla_tokens(&theme).border_subtle)
            .text_size(themes::metrics::FONT_SIZE_XS)
            .on_key_down(cx.listener(Self::on_key_down));
        for (ix, chip) in self.chips(cx).unwrap_or_default().into_iter().enumerate() {
            let (open_id, remove_id) = (chip.id.clone(), chip.id.clone());
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(themes::metrics::SPACE_2)
                    .h(themes::metrics::LAYOUT_AVATAR_SM)
                    .px(themes::metrics::SPACE_4)
                    .rounded(themes::metrics::RADIUS_FULL)
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.background)
                    .hover(move |d| d.bg(hover))
                    .child(
                        div()
                            .id(("bookmark", ix))
                            .debug_selector(move || format!("bookmark-{ix}"))
                            .text_color(if chip.safe {
                                colors.text_accent
                            } else {
                                colors.text_muted
                            })
                            .when(chip.safe, |d| d.cursor_pointer())
                            .child(chip.title)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open(&open_id, cx);
                            })),
                    )
                    .child(
                        crate::controls::icon_button_sized(
                            ("bookmark-remove", ix),
                            crate::icon::IconName::X,
                            t!(cx, "remove-label").into_owned(),
                            crate::controls::IconButtonSize::Small,
                            colors,
                            {
                                let this = cx.entity();
                                move |_, cx| {
                                    this.update(cx, |this, cx| this.remove(remove_id.clone(), cx))
                                }
                            },
                        )
                        .debug_selector(move || format!("bookmark-remove-{ix}")),
                    ),
            );
        }
        if self.adding {
            row = row
                .child(
                    div()
                        .w(themes::metrics::LAYOUT_FIELD_SM)
                        .child(self.title.clone()),
                )
                .child(
                    div()
                        .w(themes::metrics::LAYOUT_FIELD_MD)
                        .child(self.link.clone()),
                )
                .when(self.invalid, |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(theme.status().error)
                            .child(t!(cx, "bookmarks-invalid").into_owned()),
                    )
                })
                .child(
                    div()
                        .id("bookmarks-save")
                        .debug_selector(|| "bookmarks-save".into())
                        .cursor_pointer()
                        .text_color(colors.text_accent)
                        .child(t!(cx, "bookmarks-save").into_owned())
                        .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                );
        } else {
            row = row.child(
                div()
                    .id("bookmarks-add")
                    .debug_selector(|| "bookmarks-add".into())
                    .cursor_pointer()
                    .text_color(colors.text_muted)
                    .child(t!(cx, "bookmarks-add").into_owned())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_add(cx))),
            );
        }
        row
    }
}
