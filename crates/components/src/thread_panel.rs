//! Right-hand panel showing one thread: the root, its replies and a reply composer.

use domain::{Capabilities, EmojiRef, Member, MessageId, ThreadWindow, UserId};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ListAlignment, ListState, Render, Subscription, Window, actions, div, list, prelude::*, px,
};
use i18n::Translate as _;
use jiff::tz::TimeZone;
use settings::Clock;
use themes::{ActiveTheme, parla_tokens};

use crate::composer::{Composer, ComposerEvent, ComposerSource};
use crate::message_row::{MessageRow, author_name, continues, format_time};
use crate::reactions::{Pill, pills_row};
use crate::render_cache::RenderCache;
use crate::rich_text::{RichColors, RichTextCx, render_rich_text};
use crate::timeline::TimelineSource;

actions!(workspace, [ClosePanel]);

/// Key context of the panel (the workspace-level panel context).
pub const KEY_CONTEXT: &str = "Workspace";

/// What the thread panel needs from the session, so `components` does not depend on `parla`.
pub trait ThreadSource {
    fn thread(&self, root: &MessageId) -> Option<&ThreadWindow>;
    fn member(&self, u: &UserId) -> Option<&Member>;
    fn capabilities(&self) -> &Capabilities;
    fn open_thread(&mut self, root: MessageId, cx: &mut App);
    fn close_thread(&mut self, root: MessageId, cx: &mut App);
    /// The replies of the thread under `root` failed to load (Retry is offered).
    fn replies_failed(&self, _root: &MessageId) -> bool {
        false
    }
    /// Requests the replies of the thread under `root` again.
    fn retry_replies(&mut self, _root: MessageId, _cx: &mut App) {}
    /// Whether the user follows the thread under `root`; `None` when the
    /// provider cannot follow threads (no bell is shown).
    fn thread_follow(&self, _root: &MessageId) -> Option<bool> {
        None
    }
    /// Follows (`follow`) or unfollows the thread under `root`.
    fn set_thread_followed(&mut self, _root: MessageId, _follow: bool, _cx: &mut App) {}
    fn reply(&mut self, root: MessageId, text: String, broadcast: bool, cx: &mut App);
    fn react(&mut self, id: MessageId, emoji: EmojiRef, add: bool, cx: &mut App);
    /// Image of the custom emoji `shortcode`, if the workspace has one. Called
    /// while rendering, so implementations must look it up, not copy the table.
    fn custom_emoji_image(&self, _shortcode: &str) -> Option<domain::ImageRef> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadPanelEvent {
    /// The panel was closed; the host removes it.
    Close(MessageId),
    /// Open this thread in its own window.
    PopOut(MessageId),
}

pub struct ThreadPanel<S: ThreadSource + ComposerSource + TimelineSource + 'static> {
    session: Entity<S>,
    root: MessageId,
    clock: Clock,
    tz: TimeZone,
    list: ListState,
    count: usize,
    /// See [`Self::signature`]; row heights are re-measured only when it changes.
    sig: u64,
    reveal: Option<MessageId>,
    last_revealed: Option<MessageId>,
    composer: Entity<Composer<S>>,
    focus: FocusHandle,
    /// The header bell's focus.
    follow_focus: FocusHandle,
    cache: RenderCache,
    /// Row density, refreshed from the settings global (never read per row).
    density: settings::Density,
    /// Code font and avatar switch, refreshed from the settings global.
    code_font: gpui::SharedString,
    show_avatars: bool,
    /// Draw the panel's own header (title, pop-out, close); a host with its own
    /// header turns it off.
    show_header: bool,
    _subs: Vec<Subscription>,
}

impl<S: ThreadSource + ComposerSource + TimelineSource + 'static> EventEmitter<ThreadPanelEvent>
    for ThreadPanel<S>
{
}

impl<S: ThreadSource + ComposerSource + TimelineSource + 'static> ThreadPanel<S> {
    /// Opens `root` on the session and builds the panel.
    pub fn new(session: Entity<S>, root: MessageId, clock: Clock, cx: &mut Context<Self>) -> Self {
        let r = root.clone();
        session.update(cx, |s, cx| s.open_thread(r, cx));
        let composer = {
            let (s, c, r) = (session.clone(), root.conversation.clone(), root.clone());
            cx.new(|cx| {
                let mut comp = Composer::new(s, c, cx);
                comp.set_reply_root(Some(r), cx);
                comp
            })
        };
        let mut subs = Vec::new();
        subs.push(cx.observe(&session, |this, _, cx| {
            this.sync(cx);
            cx.notify();
        }));
        subs.push(cx.observe_global::<i18n::gpui::I18n>(|this, cx| {
            this.list.remeasure();
            cx.notify();
        }));
        subs.push(cx.observe_global::<settings::SettingsStore>(|this, cx| {
            let density = crate::density::current(cx);
            let mut changed = density != this.density;
            this.density = density;
            if let Some(d) = crate::live_settings::display(cx) {
                changed |= d.clock != this.clock
                    || d.code_font != this.code_font
                    || d.show_avatars != this.show_avatars;
                this.clock = d.clock;
                this.code_font = d.code_font;
                this.show_avatars = d.show_avatars;
            }
            if changed {
                this.list.remeasure();
                cx.notify();
            }
        }));
        subs.push(cx.subscribe(&composer, |this, _, ev: &ComposerEvent, cx| {
            if let ComposerEvent::SubmitReply {
                root,
                body,
                broadcast,
            } = ev.clone()
            {
                this.session.update(cx, |s, cx| {
                    let text = ComposerSource::encode(s, &body);
                    s.reply(root, text, broadcast, cx)
                });
            }
        }));
        let mut this = Self {
            session,
            root,
            clock,
            tz: crate::time_zone(cx),
            // design: allow list overdraw estimate in pixels, a rendering heuristic and not chrome
            list: ListState::new(0, ListAlignment::Top, px(200.0)),
            count: 0,
            sig: 0,
            reveal: None,
            last_revealed: None,
            composer,
            focus: cx.focus_handle(),
            follow_focus: cx.focus_handle().tab_stop(true),
            cache: RenderCache::new(),
            density: crate::density::current(cx),
            code_font: crate::live_settings::code_font(cx),
            show_avatars: crate::live_settings::display(cx).is_none_or(|d| d.show_avatars),
            show_header: true,
            _subs: subs,
        };
        this.sync(cx);
        this
    }

    /// Shows or hides the panel's own header.
    pub fn set_show_header(&mut self, show: bool) {
        self.show_header = show;
    }

    /// The clock, code font and avatar switch the rows render with (test hooks).
    pub fn clock(&self) -> Clock {
        self.clock
    }
    pub fn code_font(&self) -> &gpui::SharedString {
        &self.code_font
    }
    pub fn shows_avatars(&self) -> bool {
        self.show_avatars
    }

    /// The row density the list is laid out for (a test hook).
    pub fn density(&self) -> settings::Density {
        self.density
    }

    pub fn root(&self) -> &MessageId {
        &self.root
    }

    /// Brings `id` into view in the thread, when it is present.
    pub fn reveal(&mut self, id: MessageId, cx: &mut Context<Self>) {
        self.reveal = Some(id);
        self.apply_reveal(cx);
    }

    /// The last message successfully scrolled into view by [`Self::reveal`].
    pub fn last_revealed(&self) -> Option<&MessageId> {
        self.last_revealed.as_ref()
    }

    pub fn composer(&self) -> &Entity<Composer<S>> {
        &self.composer
    }

    /// Message texts shown, root first.
    pub fn texts(&self, cx: &App) -> Vec<String> {
        self.session
            .read(cx)
            .thread(&self.root)
            .map(|t| {
                t.root
                    .iter()
                    .chain(t.replies.messages())
                    .map(|m| m.text.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Hash of what a row's height depends on: ids, edit stamps, text length,
    /// delivery state and reactions.
    fn signature(&self, cx: &App) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Some(t) = self.session.read(cx).thread(&self.root) {
            for m in t.root.iter().chain(t.replies.messages()) {
                m.id.ts.0.hash(&mut h);
                m.edited.as_ref().map(|e| &e.0).hash(&mut h);
                m.text.len().hash(&mut h);
                std::mem::discriminant(&m.state).hash(&mut h);
                m.author.as_ref().map(|a| &a.0).hash(&mut h);
                for r in m.reactions.iter() {
                    (&r.emoji.shortcode, r.count, r.me).hash(&mut h);
                }
            }
        }
        h.finish()
    }

    fn sync(&mut self, cx: &App) {
        let n = self.session.read(cx).thread(&self.root).map_or(0, |t| {
            usize::from(t.root.is_some()) + t.replies.messages().len()
        });
        let sig = self.signature(cx);
        if n != self.count {
            self.count = n;
            self.sig = sig;
            self.list.reset(n);
            self.apply_reveal(cx);
        } else if sig != self.sig {
            self.sig = sig;
            self.list.remeasure();
            self.apply_reveal(cx);
        } else {
            self.apply_reveal(cx);
        }
    }

    fn apply_reveal(&mut self, cx: &App) {
        let Some(id) = self.reveal.clone() else {
            return;
        };
        let Some(t) = self.session.read(cx).thread(&self.root) else {
            return;
        };
        let offset = usize::from(t.root.is_some());
        let ix = if t.root.as_ref().is_some_and(|m| m.id == id) {
            Some(0)
        } else {
            t.replies
                .messages()
                .iter()
                .position(|m| m.id == id)
                .map(|i| i + offset)
        };
        if let Some(ix) = ix {
            self.reveal = None;
            self.last_revealed = Some(id);
            self.list.scroll_to_reveal_item(ix);
        }
    }

    /// Whether the user follows this thread; `None` when it cannot be followed.
    pub fn followed(&self, cx: &App) -> Option<bool> {
        self.session.read(cx).thread_follow(&self.root)
    }

    /// Follows the thread when it is not followed, else unfollows it.
    pub fn toggle_follow(&mut self, cx: &mut Context<Self>) {
        let Some(now) = self.followed(cx) else {
            return;
        };
        let root = self.root.clone();
        self.session
            .update(cx, |s, cx| s.set_thread_followed(root, !now, cx));
        cx.notify();
    }

    /// Moves the keyboard focus to the header bell.
    pub fn focus_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.follow_focus, cx);
    }

    fn close(&mut self, _: &ClosePanel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ThreadPanelEvent::Close(self.root.clone()));
    }

    fn pop_out(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ThreadPanelEvent::PopOut(self.root.clone()));
    }

    fn toggle_reaction(&mut self, id: MessageId, emoji: EmojiRef, cx: &mut Context<Self>) {
        let me = self
            .session
            .read(cx)
            .thread(&self.root)
            .into_iter()
            .flat_map(|t| t.root.iter().chain(t.replies.messages()))
            .find(|m| m.id == id)
            .is_some_and(|m| {
                m.reactions
                    .iter()
                    .any(|r| r.emoji.shortcode == emoji.shortcode && r.me)
            });
        self.session
            .update(cx, |s, cx| ThreadSource::react(s, id, emoji, !me, cx));
    }

    /// Pills of message `id` in the thread, in provider order.
    pub fn reaction_pills(&self, id: &MessageId, cx: &App) -> Vec<Pill> {
        let s = self.session.read(cx);
        let l = cx.localizer();
        s.thread(&self.root)
            .into_iter()
            .flat_map(|t| t.root.iter().chain(t.replies.messages()))
            .find(|m| &m.id == id)
            .map(|m| {
                crate::reactions::pills_of(
                    m,
                    |u| ThreadSource::member(s, u),
                    |c| ThreadSource::custom_emoji_image(s, c),
                    l,
                )
            })
            .unwrap_or_default()
    }

    fn render_item(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let s = self.session.read(cx);
        let Some(t) = s.thread(&self.root) else {
            return div().into_any_element();
        };
        // Row 0 is the root when there is one, then the replies.
        let offset = usize::from(t.root.is_some());
        let replies = t.replies.messages();
        let at = |i: usize| match i.checked_sub(offset) {
            Some(r) => replies.get(r),
            None => t.root.as_ref(),
        };
        let Some(m) = at(ix) else {
            return div().into_any_element();
        };
        let grouped = ix > 1 && at(ix - 1).is_some_and(|p| continues(p, m));
        let l = cx.localizer();
        let author = author_name(
            m.author.as_ref().and_then(|u| ThreadSource::member(s, u)),
            l,
        );
        let pills = crate::reactions::pills_of(
            m,
            |u| ThreadSource::member(s, u),
            |c| ThreadSource::custom_emoji_image(s, c),
            l,
        );
        let id = m.id.clone();
        let parser = s.content();
        let rich = self.cache.get_or_parse(m, parser.as_ref());
        // A pending reply dims through the inherited text colour, which does
        // not reach the colours the rich text sets itself; fade them over the
        // panel's editor ground (main's group opacity, one paint each).
        let queued = TimelineSource::offline(s);
        let colors = crate::message_row::rich_colors(
            RichColors::from_theme(&theme),
            &m.state,
            queued,
            theme.colors().editor_background,
        );
        let mut rcx = RichTextCx::new(s, colors, i18n::gpui::localizer(cx), self.tz.clone());
        rcx.message = Some(m.id.clone());
        rcx.clock = self.clock;
        rcx.code_font = self.code_font.clone();
        rcx.id = crate::rich_text::element_key(&m.id);
        let row = MessageRow {
            rich: Some(render_rich_text(&rich, &rcx)),
            author,
            status: None,
            author_link: None,
            time: format_time(&m.id.ts, self.clock, &self.tz, l).into(),
            text: gpui::SharedString::default(),
            edited: m.edited.is_some(),
            state: m.state.clone(),
            queued,
            failed_actions: {
                let (retry, discard) = (self.session.clone(), self.session.clone());
                crate::message_row::failed_actions_for(
                    m,
                    move |id, cx| retry.update(cx, |s, cx| TimelineSource::retry_send(s, id, cx)),
                    move |id, cx| {
                        discard.update(cx, |s, cx| TimelineSource::discard_send(s, id, cx))
                    },
                )
            },
            grouped,
            selected: false,
            density: self.density,
            show_avatar: self.show_avatars,
            avatar: m
                .author
                .as_ref()
                .and_then(|u| ThreadSource::member(s, u))
                .and_then(|mem| {
                    mem.avatar_for(themes::metrics::LAYOUT_MSG_AVATAR.as_f32())
                        .cloned()
                }),
        }
        .render(&theme, l);
        let mut el = div().w_full().child(row);
        if !pills.is_empty() {
            el = el.child(pills_row(ix, &id, pills, cx, Self::toggle_reaction, None));
        }
        el.into_any_element()
    }
}

impl<S: ThreadSource + ComposerSource + TimelineSource + 'static> Focusable for ThreadPanel<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: ThreadSource + ComposerSource + TimelineSource + 'static> Render for ThreadPanel<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        // Replies still in flight (a skeleton) or failed (Retry) show under
        // the root instead of nothing.
        let replies_pending = self
            .session
            .read(cx)
            .thread(&self.root)
            .is_some_and(|t| !t.replies.is_loaded() && t.replies.is_empty());
        let status = replies_pending.then(|| {
            let failed = self.session.read(cx).replies_failed(&self.root);
            let l = i18n::gpui::localizer(cx);
            if failed {
                let root = self.root.clone();
                crate::load_state::load_error(
                    "thread-load-error",
                    "thread-retry",
                    t!(l, "thread-load-error-title").into_owned(),
                    t!(l, "timeline-load-error-hint").into_owned(),
                    t!(l, "load-retry").into_owned(),
                    cx,
                    move |this: &mut Self, _, cx| {
                        let r = root.clone();
                        this.session.update(cx, |s, cx| s.retry_replies(r, cx));
                    },
                )
            } else {
                crate::load_state::skeleton(
                    "thread-loading",
                    1,
                    t!(l, "thread-loading").into_owned(),
                    cx,
                )
                .into_any_element()
            }
        });
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::close))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.editor_background)
            .text_color(colors.text)
            .when(self.show_header, |d| {
                d.child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .px(themes::metrics::SPACE_7)
                        .h(themes::metrics::LAYOUT_HEADER)
                        .bg(colors.surface_background)
                        .border_b_1()
                        .border_color(parla_tokens(&theme).border_subtle)
                        .text_size(themes::metrics::FONT_SIZE_LG)
                        .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                        .child(
                            div()
                                .debug_selector(|| "thread-title".into())
                                .child(t!(cx, "thread-title").into_owned()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when_some(self.followed(cx), |d, on| {
                                    let (icon, label) = if on {
                                        (
                                            crate::icon::IconName::Bell,
                                            t!(cx, "thread-unfollow").into_owned(),
                                        )
                                    } else {
                                        (
                                            crate::icon::IconName::BellOff,
                                            t!(cx, "thread-follow").into_owned(),
                                        )
                                    };
                                    let this = cx.entity();
                                    d.child(
                                        crate::controls::icon_button_styled(
                                            "thread-follow",
                                            icon,
                                            label,
                                            crate::controls::IconButtonSize::Large,
                                            on.then_some(colors.text_accent),
                                            on,
                                            colors,
                                            move |_, cx| {
                                                this.update(cx, |this, cx| this.toggle_follow(cx))
                                            },
                                        )
                                        .track_focus(&self.follow_focus)
                                        .debug_selector(|| "thread-follow".into()),
                                    )
                                })
                                .child(
                                    crate::controls::icon_button(
                                        "thread-pop-out",
                                        crate::icon::IconName::External,
                                        t!(cx, "thread-pop-out").into_owned(),
                                        colors,
                                        {
                                            let this = cx.entity();
                                            move |w, cx| {
                                                this.update(cx, |this, cx| this.pop_out(w, cx))
                                            }
                                        },
                                    )
                                    .debug_selector(|| "thread-pop-out".into()),
                                )
                                .child(
                                    crate::controls::icon_button(
                                        "thread-close",
                                        crate::icon::IconName::X,
                                        t!(cx, "close-label").into_owned(),
                                        colors,
                                        {
                                            let this = cx.entity();
                                            move |w, cx| {
                                                this.update(cx, |this, cx| {
                                                    this.close(&ClosePanel, w, cx)
                                                })
                                            }
                                        },
                                    )
                                    .debug_selector(|| "thread-close".into()),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(list(self.list.clone(), cx.processor(Self::render_item)).size_full()),
            )
            .when_some(status, |d, el| {
                d.child(
                    div()
                        .flex_none()
                        .h(themes::metrics::LAYOUT_LOAD_STATE_HEIGHT)
                        .child(el),
                )
            })
            .child(self.composer.clone())
    }
}
