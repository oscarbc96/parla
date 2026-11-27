//! Search panel: a keyboard-first query field over the workspace's local
//! message index (so it works offline), with debounced queries and hits that
//! show the conversation, author, time and a highlighted snippet.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use domain::{ConversationId, Message, MessageId, UserId};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, HighlightStyle, IntoElement,
    KeyDownEvent, Render, ScrollHandle, SharedString, StyledText, Subscription, Task, Window,
    actions, div, prelude::*,
};
use i18n::{Localizer, Translate as _};
use themes::metrics::{
    FONT_SIZE_LG, FONT_SIZE_MD, FONT_SIZE_SM, FONT_SIZE_XS, FONT_WEIGHT_BOLD, FONT_WEIGHT_MEDIUM,
    FONT_WEIGHT_SEMIBOLD, RADIUS_MD, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_7,
    SPACE_10,
};
use themes::{ActiveTheme, parla_tokens};

use crate::controls::field_frame;
use crate::message_row::PreviewLine;
use crate::text_input::{TextInput, TextInputEvent};

actions!(workspace, [Search]);

/// How long typing must pause before the index is queried.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// Key context of the panel.
pub const KEY_CONTEXT: &str = "SearchPanel";

/// What a search looks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchScope {
    #[default]
    Messages,
    Files,
    People,
    Conversations,
}

impl SearchScope {
    pub const ALL: [SearchScope; 4] = [
        SearchScope::Messages,
        SearchScope::Files,
        SearchScope::People,
        SearchScope::Conversations,
    ];

    fn label(self, l: &Localizer) -> String {
        match self {
            SearchScope::Messages => t!(l, "search-scope-messages").into_owned(),
            SearchScope::Files => t!(l, "search-scope-files").into_owned(),
            SearchScope::People => t!(l, "search-scope-people").into_owned(),
            SearchScope::Conversations => t!(l, "search-scope-conversations").into_owned(),
        }
    }
}

/// What choosing a row opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchTarget {
    /// A message (also a file hit: the message that carries the file).
    Message(MessageId),
    Conversation(ConversationId),
    Person(UserId),
}

/// One hit, already resolved to display text by the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRow {
    pub target: SearchTarget,
    pub conversation: SharedString,
    pub author: SharedString,
    pub time: SharedString,
    pub snippet: SharedString,
}

impl SearchRow {
    /// A message hit.
    pub fn message(
        id: MessageId,
        conversation: impl Into<SharedString>,
        author: impl Into<SharedString>,
        time: impl Into<SharedString>,
        snippet: impl Into<SharedString>,
    ) -> Self {
        Self {
            target: SearchTarget::Message(id),
            conversation: conversation.into(),
            author: author.into(),
            time: time.into(),
            snippet: snippet.into(),
        }
    }
}

/// The latest hits and the query they answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub query: Arc<str>,
    pub scope: SearchScope,
    /// Answers from the device (shown at once).
    pub rows: Vec<SearchRow>,
    /// Answers from the provider's server, shown below `rows`.
    pub remote: Vec<SearchRow>,
    /// Filter names that matched nothing (`in:#x`), dropped from the query.
    pub warnings: Vec<String>,
    /// The server has more to give ([`SearchSource::more`]).
    pub more: bool,
    /// The server could not answer; `rows` are still valid.
    pub remote_error: bool,
    /// The connection is down, so only the device was searched.
    pub offline: bool,
    /// The server has not answered yet.
    pub pending: bool,
}

/// What the panel needs from the session, so `components` does not depend on `parla`.
pub trait SearchSource {
    /// Starts a search; the answer shows up in [`Self::search_results`] and notifies.
    /// A blank query clears the results and never reaches the server.
    fn search(&mut self, scope: SearchScope, input: String, cx: &mut App);
    /// Asks the server for the next page of the current search.
    fn more(&mut self, _cx: &mut App) {}
    fn search_results(&self) -> &SearchResults;
    /// Whether the provider can search on its server (shows the scope tabs).
    fn remote_search(&self) -> bool {
        false
    }
    fn provider_name(&self) -> &str {
        "provider"
    }
    /// The messages around hit `id`, the hit included, oldest first (at most
    /// [`CONTEXT_AROUND`] on each side); empty while they are not known.
    fn context(&self, _id: &MessageId, _cx: &App) -> Vec<PreviewLine> {
        Vec::new()
    }
    /// The selected hit changed to `id` (`None`: not a message): reads its
    /// context unless known, dropping any read for the hit before. The
    /// answer notifies.
    fn load_context(&mut self, _id: Option<MessageId>, _cx: &mut App) {}
}

/// Width of the "In context" pane when `avail` is free right of the sidebar
/// (the timeline, the hits and the pane share it), or `None` to hide it: the
/// pane takes only what the timeline can spare above
/// `LAYOUT_TIMELINE_MIN`, so it yields before the timeline does.
pub fn search_context_width(avail: gpui::Pixels) -> Option<gpui::Pixels> {
    use themes::metrics::{LAYOUT_CONTEXT_PANE_MIN, LAYOUT_DETAIL_PANEL, LAYOUT_TIMELINE_MIN};
    let spare = avail - LAYOUT_DETAIL_PANEL - LAYOUT_TIMELINE_MIN;
    (spare >= LAYOUT_CONTEXT_PANE_MIN).then(|| spare.min(LAYOUT_DETAIL_PANEL))
}

/// Messages shown on each side of a hit in the context pane.
pub const CONTEXT_AROUND: usize = 2;

/// The slice of `msgs` (oldest first) around `id`: at most
/// [`CONTEXT_AROUND`] on each side, or `None` when `id` is not among them.
pub fn context_around<'a>(msgs: &'a [Message], id: &MessageId) -> Option<&'a [Message]> {
    let i = msgs.iter().position(|m| m.id == *id)?;
    Some(&msgs[i.saturating_sub(CONTEXT_AROUND)..(i + CONTEXT_AROUND + 1).min(msgs.len())])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchPanelEvent {
    /// The user chose a hit: show its conversation scrolled to the message.
    Open(MessageId),
    /// The user chose a conversation.
    OpenConversation(ConversationId),
    /// The user chose a person.
    OpenPerson(UserId),
    Close,
}

fn norm(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Replaces `@<id>` tokens of a snippet with `@<name>` when `name` knows the id.
pub fn resolve_user_ids(snippet: &str, name: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(snippet.len());
    let mut prev: Option<char> = None;
    let mut rest = snippet;
    while let Some(at) = rest.find('@') {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        if let Some(c) = head.chars().next_back() {
            prev = Some(c);
        }
        let token_len = tail[1..]
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(tail.len() - 1);
        let token = &tail[1..=token_len];
        let standalone = prev.is_none_or(|c| !c.is_alphanumeric());
        match name(token).filter(|_| standalone && token_len > 0) {
            Some(n) => {
                out.push('@');
                out.push_str(&n);
            }
            None => {
                out.push('@');
                out.push_str(token);
            }
        }
        prev = token.chars().next_back().or(Some('@'));
        rest = &tail[1 + token_len..];
    }
    out.push_str(rest);
    out
}

/// Byte ranges of the words of `snippet` that match a term of `query`
/// (case-insensitive; a trailing `*` on a term also matches longer words).
pub fn highlight_spans(snippet: &str, query: &str) -> Vec<Range<usize>> {
    let terms: Vec<(String, bool)> = query
        .split_whitespace()
        .filter_map(|tok| {
            let prefix = tok.ends_with('*');
            let n = norm(tok.trim_end_matches('*'));
            (!n.is_empty()).then_some((n, prefix))
        })
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let flush = |from: usize, to: usize, out: &mut Vec<Range<usize>>| {
        let word = &snippet[from..to];
        let lead = word
            .char_indices()
            .find(|(_, c)| c.is_alphanumeric())
            .map(|(i, _)| i);
        let trail = word
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_alphanumeric())
            .map(|(i, c)| i + c.len_utf8());
        let (Some(l), Some(r)) = (lead, trail) else {
            return;
        };
        let n = norm(word);
        if terms
            .iter()
            .any(|(t, prefix)| n == *t || (*prefix && n.starts_with(t.as_str())))
        {
            out.push(from + l..from + r);
        }
    };
    for (i, c) in snippet.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                flush(s, i, &mut out);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        flush(s, snippet.len(), &mut out);
    }
    out
}

pub struct SearchPanel<S: SearchSource + 'static> {
    /// Keeps the selected hit in view when Up and Down move it.
    scroll: ScrollHandle,
    session: Entity<S>,
    query: String,
    scope: SearchScope,
    selected: usize,
    /// Query whose results `selected` was last reset for.
    seen: Arc<str>,
    pending: Option<Task<()>>,
    /// The query field: caret, selection, IME and clipboard come from it.
    input: Entity<TextInput>,
    _input_sub: Subscription,
    /// The selected message hit and the lines around it, read on a data or
    /// selection change, never per frame.
    preview_for: Option<MessageId>,
    preview: Vec<PreviewLine>,
    preview_where: SharedString,
    /// The hit last reported to [`SearchSource::load_context`].
    asked: Option<Option<MessageId>>,
    /// Width of the context pane, `None` when the window has no room for it.
    context_width: Option<gpui::Pixels>,
}

impl<S: SearchSource + 'static> EventEmitter<SearchPanelEvent> for SearchPanel<S> {}

impl<S: SearchSource + 'static> SearchPanel<S> {
    pub fn new(session: Entity<S>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |this, _, cx| {
            this.sync_selection(cx);
            this.refresh_preview(cx);
            cx.notify();
        })
        .detach();
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx);
            i.set_single_line(true);
            // Down belongs to the hit list.
            i.set_intercept_down(true);
            i
        });
        let _input_sub = cx.subscribe(&input, |this, input, _: &TextInputEvent, cx| {
            let text = input.read(cx).text().to_string();
            if text != this.query {
                this.query = text;
                this.schedule(cx);
                cx.notify();
            }
        });
        Self {
            session,
            query: String::new(),
            scope: SearchScope::Messages,
            selected: 0,
            seen: Arc::from(""),
            pending: None,
            input,
            _input_sub,
            scroll: ScrollHandle::new(),
            preview_for: None,
            preview: Vec::new(),
            preview_where: SharedString::default(),
            asked: None,
            context_width: Some(themes::metrics::LAYOUT_DETAIL_PANEL),
        }
    }

    /// A panel whose field starts with `in:<target> ` (`#general`, `@ana`), so
    /// a search opened from a timeline stays in that conversation.
    pub fn in_conversation(session: Entity<S>, target: &str, cx: &mut Context<Self>) -> Self {
        let mut this = Self::new(session, cx);
        this.query = format!("in:{target} ");
        let q = this.query.clone();
        this.input.update(cx, |i, cx| i.set_text(&q, cx));
        this
    }

    /// Scopes offered: all four when the provider searches on its server,
    /// else messages on the device only.
    pub fn scopes(&self, cx: &App) -> Vec<SearchScope> {
        if self.session.read(cx).remote_search() {
            SearchScope::ALL.to_vec()
        } else {
            vec![SearchScope::Messages]
        }
    }

    pub fn scope(&self) -> SearchScope {
        self.scope
    }

    /// Switches the tab and searches again.
    pub fn set_scope(&mut self, scope: SearchScope, cx: &mut Context<Self>) {
        if self.scope == scope || !self.scopes(cx).contains(&scope) {
            return;
        }
        self.scope = scope;
        self.selected = 0;
        self.refresh_preview(cx);
        self.schedule(cx);
        cx.notify();
    }

    /// Hits in selection order: the device's first, then the server's.
    pub fn hits(&self, cx: &App) -> Vec<SearchRow> {
        let r = self.session.read(cx).search_results();
        r.rows.iter().chain(&r.remote).cloned().collect()
    }

    fn hit_count(&self, cx: &App) -> usize {
        let r = self.session.read(cx).search_results();
        r.rows.len() + r.remote.len()
    }

    /// The hit at `ix` in selection order, without copying the others.
    fn hit_at(&self, ix: usize, cx: &App) -> Option<SearchRow> {
        let r = self.session.read(cx).search_results();
        r.rows.iter().chain(&r.remote).nth(ix).cloned()
    }

    /// Notes shown under the field: offline, server failure, ignored filters.
    pub fn notes(&self, cx: &App) -> Vec<String> {
        let r = self.session.read(cx).search_results();
        let l = cx.localizer();
        let mut out = Vec::new();
        if r.offline {
            out.push(t!(l, "search-offline-note").into_owned());
        } else if r.remote_error {
            out.push(t!(l, "search-remote-error").into_owned());
        }
        if !r.warnings.is_empty() {
            let names = r.warnings.join(", ");
            out.push(t!(l, "search-warning-unknown", names = names.as_str()).into_owned());
        }
        out
    }

    /// Text in the field.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Index of the highlighted hit.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Replaces the field text and schedules the search.
    pub fn set_query(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.query = text.into();
        let q = self.query.clone();
        self.input.update(cx, |i, cx| i.set_text(&q, cx));
        self.schedule(cx);
        // Results already held (a reopened panel) show in context at once.
        self.sync_selection(cx);
        self.refresh_preview(cx);
        cx.notify();
    }

    fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.query.trim().is_empty() {
            // Nothing to wait for: clear at once.
            self.pending = None;
            self.fire(cx);
            return;
        }
        self.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, |this, cx| {
                this.pending = None;
                this.fire(cx);
            })
            .ok();
        }));
    }

    fn fire(&mut self, cx: &mut Context<Self>) {
        let q = self.query.trim().to_string();
        let scope = self.scope;
        self.session.update(cx, |s, cx| s.search(scope, q, cx));
    }

    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let r = self.session.read(cx).search_results();
        if r.query != self.seen {
            self.seen = r.query.clone();
            self.selected = 0;
        }
        let n = r.rows.len() + r.remote.len();
        self.selected = self.selected.min(n.saturating_sub(1));
    }

    /// Scrolls the list to the selected hit. Children of the list are the
    /// notes, the count line, then the hits (with a header before the server's).
    fn reveal_selected(&self, cx: &App) {
        let r = self.session.read(cx).search_results();
        let header = usize::from(self.selected >= r.rows.len() && !r.remote.is_empty());
        let lead = self.notes(cx).len() + 1;
        self.scroll.scroll_to_item(lead + self.selected + header);
    }

    /// The lines of the context pane: the selected message hit with the
    /// messages around it, or the hit alone while those are not known.
    pub fn preview(&self) -> &[PreviewLine] {
        &self.preview
    }

    /// Reads the context of the selected hit again; asks the source once
    /// per hit when it has none.
    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let row = self.hit_at(self.selected, cx);
        let message = row.as_ref().and_then(|r| match &r.target {
            SearchTarget::Message(id) => Some(id.clone()),
            _ => None,
        });
        // Only a change of the selected hit is reported, never a redraw.
        if self.asked.as_ref() != Some(&message) && (message.is_some() || self.asked.is_some()) {
            self.asked = Some(message.clone());
            let ask = message.clone();
            self.session.update(cx, |s, cx| s.load_context(ask, cx));
        }
        let (Some(row), Some(id)) = (row, message) else {
            self.preview_for = None;
            self.preview.clear();
            return;
        };
        let mut lines = self.session.read(cx).context(&id, cx);
        if lines.is_empty() {
            lines = vec![PreviewLine {
                id: id.clone(),
                author: row.author.clone(),
                avatar: None,
                time: row.time.clone(),
                text: row.snippet.clone(),
            }];
        }
        self.preview_where = format!("{} \u{b7} {}", row.conversation, row.time).into();
        self.preview_for = Some(id);
        self.preview = lines;
    }

    /// Sets the context pane's width (the host knows the window's room);
    /// `None` hides it.
    pub fn set_context_width(&mut self, width: Option<gpui::Pixels>) {
        self.context_width = width;
    }

    /// "Jump to message": opens the selected hit's conversation at it.
    pub fn jump(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.preview_for.clone() {
            cx.emit(SearchPanelEvent::Open(id));
        }
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.hit_at(self.selected, cx) {
            cx.emit(open_event(&row.target));
        }
    }

    /// Keys the field leaves to the panel: choosing and moving through the hits, closing.
    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if m.platform || m.control || m.alt || m.shift || self.input.read(cx).is_composing() {
            return;
        }
        let rows = self.hit_count(cx);
        match k.key.as_str() {
            "escape" => cx.emit(SearchPanelEvent::Close),
            "enter" => self.open_selected(cx),
            "down" => {
                self.selected = (self.selected + 1).min(rows.saturating_sub(1));
                self.reveal_selected(cx);
                self.refresh_preview(cx);
            }
            "up" => {
                self.selected = self.selected.saturating_sub(1);
                self.reveal_selected(cx);
                self.refresh_preview(cx);
            }
            _ => return,
        }
        cx.notify();
    }
}

impl<S: SearchSource + 'static> Focusable for SearchPanel<S> {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl<S: SearchSource + 'static> Render for SearchPanel<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let placeholder = t!(cx, "search-placeholder").into_owned();
        self.input.update(cx, |i, _| i.set_placeholder(placeholder));
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let results = self.session.read(cx).search_results();
        let blank = self.query.trim().is_empty();
        let provider = self.session.read(cx).provider_name().to_string();
        let remote_search = self.session.read(cx).remote_search();
        let notes = self.notes(cx);
        let highlight = HighlightStyle {
            font_weight: Some(FONT_WEIGHT_BOLD),
            background_color: Some(parla_tokens(&theme).message_mention_background),
            underline: Some(gpui::UnderlineStyle {
                thickness: SPACE_1,
                color: Some(parla_tokens(&theme).logo.accent_1),
                wavy: false,
            }),
            ..Default::default()
        };
        let mut body = div()
            .id("search-results")
            .gap(SPACE_4)
            .pb(SPACE_4)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .track_scroll(&self.scroll)
            .overflow_y_scroll();
        for n in notes {
            body = body.child(note(&n, colors.text_muted));
        }
        let total = results.rows.len() + results.remote.len();
        if blank {
            body = body.child(note(&t!(cx, "search-hint"), colors.text_muted));
        } else if total == 0 {
            body = body.child(note(&t!(cx, "search-no-results"), colors.text_muted));
        } else {
            body = body.child(note(
                &results_label(total, cx.localizer()),
                colors.text_muted,
            ));
        }
        if !blank {
            let local_len = results.rows.len();
            for (ix, row) in results.rows.iter().chain(&results.remote).enumerate() {
                if ix == local_len && !results.remote.is_empty() {
                    body = body.child(note(
                        &t!(cx, "search-from-provider", provider = provider.as_str()),
                        colors.text_muted,
                    ));
                }
                let spans = highlight_spans(&row.snippet, &results.query);
                let target = row.target.clone();
                let selected = ix == self.selected;
                let el = div()
                    .id(("search-hit", ix))
                    .mx(SPACE_6)
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .flex()
                    .flex_col()
                    .gap(SPACE_2)
                    .rounded(RADIUS_MD)
                    .border_1()
                    .border_color(if selected {
                        colors.element_background
                    } else {
                        colors.border
                    })
                    .bg(colors.surface_background)
                    .when(selected, |d| {
                        // The selected hit's soft accent halo.
                        d.shadow(vec![gpui::BoxShadow {
                            color: colors.accent_soft,
                            offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
                            blur_radius: gpui::px(0.0),
                            spread_radius: SPACE_1,
                            inset: false,
                        }])
                    })
                    .text_size(FONT_SIZE_MD)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = ix;
                        cx.emit(open_event(&target));
                    }))
                    .child(
                        div()
                            .flex()
                            .gap(SPACE_2)
                            .text_size(FONT_SIZE_XS)
                            .text_color(colors.text_muted)
                            .child(row.conversation.clone())
                            .child("\u{b7}")
                            .child(row.time.clone()),
                    )
                    .child(
                        div()
                            .font_weight(FONT_WEIGHT_SEMIBOLD)
                            .child(row.author.clone()),
                    )
                    .child(
                        StyledText::new(row.snippet.clone())
                            .with_highlights(spans.into_iter().map(|r| (r, highlight))),
                    );
                body = body.child(el);
            }
            if results.more {
                body = body.child(
                    div()
                        .id("search-more")
                        .px(SPACE_6)
                        .py(SPACE_4)
                        .text_size(FONT_SIZE_XS)
                        .font_weight(FONT_WEIGHT_MEDIUM)
                        .cursor_pointer()
                        .text_color(colors.text_accent)
                        .child(t!(cx, "search-more").into_owned())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.more(cx));
                        })),
                );
            }
        }
        let mut tabs = div()
            .flex()
            .gap(SPACE_7)
            .px(SPACE_6)
            .border_b_1()
            .border_color(colors.border)
            .text_size(FONT_SIZE_SM);
        if remote_search {
            for sc in SearchScope::ALL {
                let active = sc == self.scope;
                tabs = tabs.child(
                    div()
                        .id(("search-scope", sc as usize))
                        .cursor_pointer()
                        .h(SPACE_10)
                        .flex()
                        .items_center()
                        .border_b_2()
                        .border_color(if active {
                            colors.text_accent
                        } else {
                            gpui::transparent_black()
                        })
                        .text_color(if active {
                            colors.text
                        } else {
                            colors.text_muted
                        })
                        .font_weight(if active {
                            FONT_WEIGHT_SEMIBOLD
                        } else {
                            FONT_WEIGHT_MEDIUM
                        })
                        .child(sc.label(cx.localizer()))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_scope(sc, cx))),
                );
            }
        }
        // Kept while a non-message hit is selected, so the width never jumps.
        let preview = self
            .context_width
            .filter(|_| !blank)
            .map(|w| self.render_preview(w, cx));
        let list = div()
            .flex()
            .flex_col()
            .flex_none()
            .flex_shrink_0()
            .w(themes::metrics::LAYOUT_DETAIL_PANEL)
            .h_full()
            .bg(colors.editor_background)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .px(SPACE_6)
                    .h(SPACE_10)
                    .border_b_1()
                    .border_color(colors.border)
                    .text_size(FONT_SIZE_LG)
                    .font_weight(FONT_WEIGHT_BOLD)
                    .child(t!(cx, "search-title").into_owned())
                    .child(crate::controls::icon_button(
                        "search-close",
                        crate::icon::IconName::X,
                        t!(cx, "close-label").into_owned(),
                        colors,
                        {
                            let this = cx.entity();
                            move |_, cx| this.update(cx, |_, cx| cx.emit(SearchPanelEvent::Close))
                        },
                    )),
            )
            .child(
                field_frame(focused, colors, parla_tokens(&theme).border_control)
                    .id("search-field")
                    .mx(SPACE_6)
                    .my(SPACE_4)
                    .child(self.input.clone()),
            )
            .when(remote_search, |d| d.child(tabs))
            .child(body)
            .when(!remote_search, |d| {
                d.child(note(&t!(cx, "search-local-note"), colors.text_muted))
            });
        div()
            .key_context(KEY_CONTEXT)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .h_full()
            .min_w_0()
            .text_color(colors.text)
            .child(list)
            .children(preview)
    }
}

impl<S: SearchSource + 'static> SearchPanel<S> {
    /// The "In context" pane beside the hits (board Search): the selected
    /// hit among its neighbours and "Jump to message".
    fn render_preview(
        &self,
        width: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        use themes::metrics as m;
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let tokens = parla_tokens(&theme);
        let hit = self.preview_for.clone();
        let mut lines = div()
            .id("search-preview-lines")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py(SPACE_4);
        for p in &self.preview {
            let is_hit = hit.as_ref() == Some(&p.id);
            lines = lines.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_1)
                    .px(SPACE_6)
                    .py(SPACE_3)
                    .when(is_hit, |d| {
                        d.bg(tokens.message_mention_background)
                            .border_l_2()
                            .border_color(tokens.signal)
                    })
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(SPACE_4)
                            .child(
                                div()
                                    .text_size(FONT_SIZE_SM)
                                    .font_weight(FONT_WEIGHT_SEMIBOLD)
                                    .text_color(if is_hit {
                                        colors.text
                                    } else {
                                        colors.text_muted
                                    })
                                    .child(p.author.clone()),
                            )
                            .child(
                                div()
                                    .text_size(FONT_SIZE_XS)
                                    .text_color(tokens.text_subtle)
                                    .child(p.time.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(FONT_SIZE_SM)
                            .text_color(if is_hit {
                                colors.text
                            } else {
                                colors.text_muted
                            })
                            .child(p.text.clone()),
                    ),
            );
        }
        let jump = crate::controls::no_focus_on_click(
            crate::controls::button(
                "search-jump",
                t!(cx, "search-jump").into_owned(),
                crate::controls::ButtonKind::Primary,
                true,
                colors,
            )
            .flex_1()
            // Icon first, as on the board: the label is added first.
            .flex_row_reverse()
            .gap(SPACE_3)
            .debug_selector(|| "search-jump".into()),
        )
        .child(
            crate::icon::Icon::new(crate::icon::IconName::ArrowRight)
                .size(m::ICON_SIZE_MD)
                .color(colors.text_on_accent),
        );
        let jump = crate::controls::on_activate(jump, cx, |this, _, cx| this.jump(cx));
        let held = hit.is_some();
        if !held {
            lines = lines.child(
                div()
                    .debug_selector(|| "search-preview-empty".into())
                    .px(SPACE_6)
                    .py(SPACE_4)
                    .text_size(FONT_SIZE_SM)
                    .text_color(colors.text_muted)
                    .child(t!(cx, "search-context-empty").into_owned()),
            );
        }
        div()
            .id("search-preview")
            .debug_selector(|| "search-preview".into())
            .flex()
            .flex_col()
            .flex_none()
            .w(width)
            .h_full()
            .bg(colors.surface_background)
            .border_l_1()
            .border_color(colors.border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .flex_none()
                    .min_h(SPACE_10)
                    .px(SPACE_6)
                    .py(SPACE_3)
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .text_size(FONT_SIZE_LG)
                            .font_weight(FONT_WEIGHT_BOLD)
                            .child(t!(cx, "search-in-context").into_owned()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(FONT_SIZE_XS)
                            .text_color(tokens.text_subtle)
                            .when(held, |d| d.child(self.preview_where.clone())),
                    ),
            )
            .child(lines)
            .when(held, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .px(SPACE_6)
                        .pt(SPACE_5)
                        .pb(SPACE_6)
                        .border_t_1()
                        .border_color(tokens.border_subtle)
                        .child(jump),
                )
            })
    }
}

/// "1 result" / "N results" in the active language.
pub fn results_label(n: usize, l: &Localizer) -> String {
    t!(l, "search-results", count = n).into_owned()
}

fn open_event(t: &SearchTarget) -> SearchPanelEvent {
    match t {
        SearchTarget::Message(id) => SearchPanelEvent::Open(id.clone()),
        SearchTarget::Conversation(c) => SearchPanelEvent::OpenConversation(c.clone()),
        SearchTarget::Person(u) => SearchPanelEvent::OpenPerson(u.clone()),
    }
}

fn note(text: &str, color: gpui::Hsla) -> gpui::Div {
    div()
        .px(SPACE_6)
        .py(SPACE_2)
        .text_size(FONT_SIZE_XS)
        .text_color(color)
        .child(text.to_string())
}
