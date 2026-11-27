//! Rich text rendering: [`build_nodes`] resolves a [`RichText`] against the
//! session (member names, dates, provider links) into a plain, testable
//! [`Node`] tree; [`render_rich_text`] turns that tree into GPUI elements.
//! Code is monospace without highlighting; links go through [`open_link`].

use std::borrow::Cow;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use domain::{
    Block, Broadcast, ContextItem, ConversationId, ImageRef, Inline, ListStyle, MessageId,
    RichText, Style, UserId,
};
use gpui::{
    AnyElement, App, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveText, IntoElement,
    SharedString, StrikethroughStyle, StyledText, UnderlineStyle, Window, div, prelude::*, px,
};
use i18n::Localizer;
use jiff::{Timestamp, tz::TimeZone};
use settings::Clock;
use themes::Theme;
use themes::{color, parla_tokens};

use crate::call_card::{CallAction, CallNode, call_node, render_call};
use crate::links::{LinkAction, open_link};
use crate::timeline::TimelineSource;

/// Colours the renderer needs, resolved once per frame from the theme.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RichColors {
    pub text: Hsla,
    pub muted: Hsla,
    pub link: Hsla,
    pub code_bg: Hsla,
    pub border: Hsla,
    /// Background of mentions of the current user (`message.mention.background`).
    pub mention_bg: Hsla,
    /// Raised ground of cards (`surface_background`).
    pub raised: Hsla,
    /// Live and join marks (`status.success`).
    pub success: Hsla,
    /// Readable text on a `success` fill (`text_on_accent`).
    pub on_success: Hsla,
    /// Quiet text such as an ended call's status (`text_subtle`).
    pub subtle: Hsla,
}

impl RichColors {
    pub fn from_theme(theme: &Theme) -> Self {
        let c = theme.colors();
        Self {
            text: c.text,
            muted: c.text_muted,
            link: c.text_accent,
            code_bg: parla_tokens(theme).bg_sunken,
            border: c.border,
            mention_bg: parla_tokens(theme).message_mention_background,
            raised: c.surface_background,
            success: theme.status().success,
            // Borrows `text_on_accent`: it is white on the accent fill, and both built-in themes keep the success green dark enough for it to read.
            on_success: c.text_on_accent,
            subtle: parla_tokens(theme).text_subtle,
        }
    }

    /// This set composited at `strength` over `ground`, opaque: how main's
    /// group opacity drew a dimmed row. A pending row's inherited text colour
    /// does not reach the colours the rich text sets itself (links, mentions,
    /// code chips, card text), so they fade here in one paint each.
    pub fn faded(self, strength: f32, ground: Hsla) -> Self {
        let fade = |colour: Hsla| color::faded(colour, ground, strength);
        Self {
            text: fade(self.text),
            muted: fade(self.muted),
            link: fade(self.link),
            code_bg: fade(self.code_bg),
            border: fade(self.border),
            mention_bg: fade(self.mention_bg),
            raised: fade(self.raised),
            success: fade(self.success),
            on_success: fade(self.on_success),
            subtle: fade(self.subtle),
        }
    }
}

pub type OpenConversation = Rc<dyn Fn(ConversationId, &mut Window, &mut App)>;
/// Called with the user of a clicked `@mention`.
pub type OpenUser = Rc<dyn Fn(UserId, &mut Window, &mut App)>;
pub type OpenUrl = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// Opens a vetted external URL through the host's handler, else the app.
fn open_external(url: String, handler: &Option<OpenUrl>, window: &mut Window, app: &mut App) {
    match handler {
        Some(f) => f(url, window, app),
        None => app.open_url(&url),
    }
}

/// [`open_link`]s `url` and, when it may leave the app, [`open_external`]s it.
fn open_link_external(url: &str, handler: &Option<OpenUrl>, window: &mut Window, app: &mut App) {
    if let LinkAction::OpenExternal(u) = open_link(url) {
        open_external(u, handler, window, app);
    }
}

pub type ImageHook<'a> = &'a dyn Fn(&ImageRef, &str) -> AnyElement;

/// Code font family until the code font setting sets another ([`RichTextCx::code_font`]).
pub const DEFAULT_CODE_FONT: &str = "JetBrains Mono";

/// Hair spaces inside an inline code span. A text highlight paints only the
/// glyphs its range covers, so these give the chip small horizontal padding.
const CODE_PAD: &str = "\u{2009}";

pub struct RichTextCx<'a> {
    pub source: &'a dyn TimelineSource,
    /// The message being rendered, for provider links.
    pub message: Option<MessageId>,
    pub tz: TimeZone,
    pub clock: Clock,
    pub colors: RichColors,
    /// Active language, for dates and the few strings the renderer owns.
    pub localizer: Arc<Localizer>,
    pub code_font: SharedString,
    /// Distinguishes element ids of different messages (see [`element_key`]).
    pub id: usize,
    pub on_open_conversation: Option<OpenConversation>,
    pub on_open_user: Option<OpenUser>,
    pub on_open_url: Option<OpenUrl>,
    /// Renders images (media pipeline); alt text is shown without it.
    pub image: Option<ImageHook<'a>>,
    /// Renders a custom emoji image inline; the alt text is the shortcode.
    pub emoji: Option<ImageHook<'a>>,
    /// Called with the conversation when a live huddle card's Join is clicked.
    pub on_join_huddle: Option<OpenConversation>,
}

impl<'a> RichTextCx<'a> {
    pub fn new(
        source: &'a dyn TimelineSource,
        colors: RichColors,
        localizer: Arc<Localizer>,
        tz: TimeZone,
    ) -> Self {
        Self {
            source,
            message: None,
            tz,
            clock: Clock::H24,
            colors,
            localizer,
            code_font: DEFAULT_CODE_FONT.into(),
            id: 0,
            on_open_conversation: None,
            on_open_user: None,
            on_open_url: None,
            image: None,
            emoji: None,
            on_join_huddle: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    Code,
    Link(Arc<str>),
    Mention {
        me: bool,
        /// The mentioned person; `None` for groups and broadcasts.
        user: Option<UserId>,
    },
    Conversation(ConversationId),
    /// A custom emoji whose image is known; the text is `:shortcode:`.
    CustomEmoji(ImageRef),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub style: Style,
    pub role: Option<Role>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextNode {
    pub text: String,
    pub spans: Vec<Span>,
}

impl TextNode {
    fn push(&mut self, s: &str, style: Style, role: Option<Role>) {
        let start = self.text.len();
        self.text.push_str(s);
        if style != Style::default() || role.is_some() {
            self.spans.push(Span {
                range: start..self.text.len(),
                style,
                role,
            });
        }
    }

    fn bold(mut self) -> Self {
        let len = self.text.len();
        for s in &mut self.spans {
            s.style.bold = true;
        }
        let mut spans = Vec::new();
        let mut at = 0;
        for s in std::mem::take(&mut self.spans) {
            if at < s.range.start {
                spans.push(Span {
                    range: at..s.range.start,
                    style: Style {
                        bold: true,
                        ..Style::default()
                    },
                    role: None,
                });
            }
            at = s.range.end;
            spans.push(s);
        }
        if at < len {
            spans.push(Span {
                range: at..len,
                style: Style {
                    bold: true,
                    ..Style::default()
                },
                role: None,
            });
        }
        self.spans = spans;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Text(TextNode),
    Heading(TextNode),
    Code(String),
    Quote(Vec<Node>),
    List {
        indent: u8,
        items: Vec<(String, TextNode)>,
    },
    Divider,
    Image {
        source: ImageRef,
        alt: String,
    },
    /// Muted single line of context items.
    Context(Vec<Node>),
    Section {
        text: Vec<Node>,
        fields: Vec<TextNode>,
        accessory: Option<Box<Node>>,
    },
    /// Disabled controls plus a link to the same message in the provider.
    Actions {
        labels: Vec<String>,
        open_label: String,
        open_url: Option<String>,
    },
    Table {
        header: bool,
        rows: Vec<Vec<TextNode>>,
    },
    Card {
        color: Option<u32>,
        children: Vec<Node>,
    },
    /// A huddle or third-party call card.
    Call(CallNode),
    Fallback(String),
}

pub(crate) fn mention_name(cx: &RichTextCx, id: &domain::UserId, label: Option<&str>) -> String {
    cx.source
        .member(id)
        .and_then(crate::sidebar::best_name)
        .map(|n| n.to_string())
        .or_else(|| label.filter(|l| !l.is_empty()).map(str::to_string))
        .unwrap_or_else(|| t!(cx.localizer, "author-unknown").into_owned())
}

fn format_date(unix: i64, fallback: &str, cx: &RichTextCx) -> String {
    let Ok(t) = Timestamp::from_second(unix) else {
        return fallback.to_string();
    };
    cx.localizer
        .date_time(t, &cx.tz, crate::message_row::hour_cycle(cx.clock))
}

fn text_node(items: &[Inline], cx: &RichTextCx) -> TextNode {
    let mut out = TextNode::default();
    let plain = Style::default();
    let custom_images = cx.emoji.is_some();
    for i in items {
        match i {
            Inline::Text { text, style } => out.push(text, *style, None),
            Inline::Code(t) => {
                out.push(&format!("{CODE_PAD}{t}{CODE_PAD}"), plain, Some(Role::Code))
            }
            Inline::Link { url, label, style } => out.push(
                label.as_deref().unwrap_or(url),
                *style,
                Some(Role::Link(url.clone())),
            ),
            Inline::User { id, label } => {
                let me = cx.source.self_user() == Some(id);
                let name = mention_name(cx, id, label.as_deref());
                out.push(
                    &format!("@{name}"),
                    plain,
                    Some(Role::Mention {
                        me,
                        user: Some(id.clone()),
                    }),
                );
            }
            Inline::Conversation { id, label } => {
                let name = cx
                    .source
                    .conversation(id)
                    .map(|c| c.name.to_string())
                    .filter(|n| !n.is_empty())
                    .or_else(|| label.as_deref().map(str::to_string))
                    .unwrap_or_else(|| id.0.to_string());
                out.push(
                    &format!("#{name}"),
                    plain,
                    Some(Role::Conversation(id.clone())),
                );
            }
            Inline::Group { id, label } => {
                // The workspace's current handle wins over the label the message carried.
                let group = cx.source.user_group(id);
                let l = group
                    .map(|g| &*g.handle)
                    .filter(|h| !h.is_empty())
                    .or(label.as_deref())
                    .unwrap_or(id);
                let s = if l.starts_with('@') {
                    l.to_string()
                } else {
                    format!("@{l}")
                };
                out.push(
                    &s,
                    plain,
                    Some(Role::Mention {
                        me: group.is_some_and(|g| g.is_member),
                        user: None,
                    }),
                );
            }
            Inline::Broadcast(b) => out.push(
                match b {
                    Broadcast::Here => "@here",
                    Broadcast::Channel => "@channel",
                    Broadcast::Everyone => "@everyone",
                },
                plain,
                Some(Role::Mention {
                    me: false,
                    user: None,
                }),
            ),
            Inline::Emoji(e) => match &e.unicode {
                Some(u) => out.push(u, plain, None),
                None => {
                    let image = custom_images
                        .then(|| cx.source.custom_emoji_image(&e.shortcode))
                        .flatten()
                        .map(Role::CustomEmoji);
                    out.push(&format!(":{}:", e.shortcode), plain, image)
                }
            },
            Inline::Date { unix, fallback } => {
                out.push(&format_date(*unix, fallback, cx), plain, None)
            }
            Inline::LineBreak => out.push("\n", plain, None),
        }
    }
    out
}

fn card_node(c: &domain::Card, cx: &RichTextCx) -> Node {
    let mut children = Vec::new();
    if let Some(t) = &c.thumb {
        children.push(image_node(t, ""));
    }
    if let Some(a) = &c.author {
        children.push(Node::Text(text_node(a, cx)));
    }
    if let Some(t) = &c.title {
        children.push(Node::Text(text_node(std::slice::from_ref(t), cx).bold()));
    }
    children.extend(c.body.iter().map(|b| block_node(b, cx)));
    // Fields form one tight block, a "key: value" line each.
    let mut fields = TextNode::default();
    for (n, (k, v)) in c.fields.iter().enumerate() {
        if n > 0 {
            fields.push("\n", Style::default(), None);
        }
        fields.push(
            &format!("{k}: "),
            Style {
                bold: true,
                ..Style::default()
            },
            None,
        );
        let v = text_node(v, cx);
        let off = fields.text.len();
        fields.text.push_str(&v.text);
        fields.spans.extend(v.spans.into_iter().map(|mut s| {
            s.range = s.range.start + off..s.range.end + off;
            s
        }));
    }
    if !c.fields.is_empty() {
        children.push(Node::Text(fields));
    }
    if let Some(i) = &c.image {
        children.push(image_node(i, ""));
    }
    if let Some(f) = &c.footer {
        children.push(Node::Context(vec![Node::Text(text_node(f, cx))]));
    }
    Node::Card {
        color: c.color,
        children,
    }
}

fn image_node(source: &ImageRef, alt: &str) -> Node {
    Node::Image {
        source: source.clone(),
        alt: alt.to_string(),
    }
}

fn block_node(b: &Block, cx: &RichTextCx) -> Node {
    match b {
        Block::Paragraph(i) => Node::Text(text_node(i, cx)),
        Block::Heading(i) => Node::Heading(text_node(i, cx)),
        Block::Quote(bs) => Node::Quote(bs.iter().map(|b| block_node(b, cx)).collect()),
        Block::Code { text, .. } => Node::Code(text.to_string()),
        Block::List {
            style,
            indent,
            items,
        } => Node::List {
            indent: *indent,
            items: items
                .iter()
                .enumerate()
                .map(|(i, it)| {
                    let marker = match style {
                        ListStyle::Bullet => "\u{2022}".to_string(),
                        ListStyle::Ordered { start } => format!("{}.", *start as usize + i),
                    };
                    (marker, text_node(it, cx))
                })
                .collect(),
        },
        Block::Divider => Node::Divider,
        Block::Image { source, alt, .. } => image_node(source, alt),
        Block::Context(items) => Node::Context(
            items
                .iter()
                .map(|i| match i {
                    ContextItem::Text(t) => Node::Text(text_node(t, cx)),
                    ContextItem::Image { source, alt } => image_node(source, alt),
                })
                .collect(),
        ),
        Block::Section {
            text,
            fields,
            accessory,
        } => Node::Section {
            text: text.iter().map(|b| block_node(b, cx)).collect(),
            fields: fields.iter().map(|f| text_node(f, cx)).collect(),
            accessory: accessory.as_ref().map(|a| Box::new(block_node(a, cx))),
        },
        Block::Actions(controls) => Node::Actions {
            labels: controls.iter().map(|c| c.label.to_string()).collect(),
            open_label: t!(
                cx.localizer,
                "rich-open-in",
                provider = cx.source.provider_name()
            )
            .into_owned(),
            open_url: cx.message.as_ref().and_then(|id| cx.source.web_url(id)),
        },
        Block::Table { header, rows } => Node::Table {
            header: *header,
            rows: rows
                .iter()
                .map(|r| r.iter().map(|c| text_node(c, cx)).collect())
                .collect(),
        },
        Block::Card(c) => card_node(c, cx),
        Block::Call(c) => Node::Call(call_node(c, cx)),
        Block::Unsupported { fallback } => Node::Fallback(fallback.to_string()),
    }
}

/// Resolves `rt` against the session into a [`Node`] tree.
pub fn build_nodes(rt: &RichText, cx: &RichTextCx) -> Vec<Node> {
    rt.blocks.iter().map(|b| block_node(b, cx)).collect()
}

/// Builds the element tree of `rt`.
pub fn render_rich_text(rt: &RichText, cx: &RichTextCx) -> AnyElement {
    let nodes = build_nodes(rt, cx);
    let ids = Cell::new(0usize);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w_0()
        .children(nodes.iter().map(|n| render_node(n, cx, &ids)))
        .into_any_element()
}

#[derive(Clone)]
enum Target {
    Url(Arc<str>),
    Conversation(ConversationId),
    User(UserId),
}

/// A key for element ids that follows the message, not its row: ids derived
/// from a row index shift when older messages are prepended, and GPUI would
/// attach hover and click state to the wrong row.
pub(crate) fn element_key(id: &domain::MessageId) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish() as usize
}

fn next_id(cx: &RichTextCx, ids: &Cell<usize>) -> (&'static str, usize) {
    let n = ids.get();
    ids.set(n + 1);
    ("rich-text", cx.id.wrapping_mul(4096).wrapping_add(n))
}

fn highlight(span: &Span, cx: &RichTextCx) -> HighlightStyle {
    let mut h = HighlightStyle::default();
    if span.style.bold {
        h.font_weight = Some(FontWeight::BOLD);
    }
    if span.style.italic {
        h.font_style = Some(FontStyle::Italic);
    }
    if span.style.strike {
        h.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.0),
            color: None,
        });
    }
    match &span.role {
        Some(Role::Code) => h.background_color = Some(cx.colors.code_bg),
        Some(Role::Link(_)) => {
            h.color = Some(cx.colors.link);
            h.underline = Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(cx.colors.link),
                wavy: false,
            });
        }
        Some(Role::Mention { me, .. }) => {
            h.color = Some(cx.colors.link);
            h.font_weight = Some(FontWeight::MEDIUM);
            if *me {
                h.background_color = Some(cx.colors.mention_bg);
            }
        }
        Some(Role::Conversation(_)) => h.color = Some(cx.colors.link),
        Some(Role::CustomEmoji(_)) => {}
        None => {}
    }
    h
}

/// Splits `t` around custom emoji spans: text runs and emoji images in a wrapping row.
fn with_emoji(t: &TextNode, hook: ImageHook, cx: &RichTextCx, ids: &Cell<usize>) -> AnyElement {
    let mut row = div().flex().flex_wrap().items_center();
    let mut at = 0;
    let run = |from: usize, to: usize, cx: &RichTextCx, ids: &Cell<usize>| {
        let mut part = TextNode {
            text: t.text[from..to].to_string(),
            spans: Vec::new(),
        };
        for s in &t.spans {
            if s.range.start >= from && s.range.end <= to {
                part.spans.push(Span {
                    range: s.range.start - from..s.range.end - from,
                    ..s.clone()
                });
            }
        }
        text_element(&part, cx, ids)
    };
    for s in &t.spans {
        let Some(Role::CustomEmoji(image)) = &s.role else {
            continue;
        };
        if at < s.range.start {
            row = row.child(run(at, s.range.start, cx, ids));
        }
        let shortcode = t.text[s.range.clone()].trim_matches(':');
        row = row.child(hook(image, shortcode));
        at = s.range.end;
    }
    if at < t.text.len() {
        row = row.child(run(at, t.text.len(), cx, ids));
    }
    row.into_any_element()
}

/// A run of non-space characters longer than this gets break opportunities.
const LONG_TOKEN: usize = 24;
/// Characters between inserted break opportunities inside a long token.
const BREAK_EVERY: usize = 8;
const ZWSP: char = '\u{200b}';

/// Calls `f` with each byte offset of `text` before which a break goes, in
/// order, and returns how many there were. Allocates nothing.
fn break_points(text: &str, mut f: impl FnMut(usize)) -> usize {
    let mut count = 0;
    let mut flush = |start: usize, len: usize, end: usize| {
        if len > LONG_TOKEN {
            for (i, _) in text[start..end]
                .char_indices()
                .skip(BREAK_EVERY)
                .step_by(BREAK_EVERY)
            {
                f(start + i);
                count += 1;
            }
        }
    };
    // Start byte and length in characters of the current non-space run.
    let mut start = 0;
    let mut len = 0;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() || c == ZWSP {
            flush(start, len, i);
            len = 0;
        } else {
            if len == 0 {
                start = i;
            }
            len += 1;
        }
    }
    flush(start, len, text.len());
    count
}

/// Layout only wraps at word boundaries, so an unbroken token (a hash, a long
/// URL) wider than the column would run off its edge. Inserts a zero-width
/// space every few characters of any token longer than [`LONG_TOKEN`]; spans
/// keep covering the same characters. Text without such a token is returned
/// borrowed, without a copy.
pub fn break_long_tokens(t: &TextNode) -> Cow<'_, TextNode> {
    // Counts first, so text without a long token allocates nothing and the
    // offsets fit one exact allocation (this runs on every render).
    let count = break_points(&t.text, |_| {});
    if count == 0 {
        return Cow::Borrowed(t);
    }
    // Byte offsets (in the old text) before which a break is inserted.
    let mut at: Vec<usize> = Vec::with_capacity(count);
    break_points(&t.text, |i| at.push(i));
    let mut text = String::with_capacity(t.text.len() + at.len() * ZWSP.len_utf8());
    let mut prev = 0;
    for &i in &at {
        text.push_str(&t.text[prev..i]);
        text.push(ZWSP);
        prev = i;
    }
    text.push_str(&t.text[prev..]);
    let z = ZWSP.len_utf8();
    // A start lands after a break inserted at it, an end before one.
    let start = |o: usize| o + z * at.partition_point(|&i| i <= o);
    let end = |o: usize| o + z * at.partition_point(|&i| i < o);
    let spans = t
        .spans
        .iter()
        .map(|s| Span {
            range: start(s.range.start)..end(s.range.end),
            ..s.clone()
        })
        .collect();
    Cow::Owned(TextNode { text, spans })
}

fn text_element(t: &TextNode, cx: &RichTextCx, ids: &Cell<usize>) -> AnyElement {
    if t.text.is_empty() {
        return div().into_any_element();
    }
    if let Some(hook) = cx.emoji
        && t.spans
            .iter()
            .any(|s| matches!(s.role, Some(Role::CustomEmoji(_))))
    {
        return with_emoji(t, hook, cx, ids);
    }
    let broken = break_long_tokens(t);
    let t = &*broken;
    let highlights: Vec<_> = t
        .spans
        .iter()
        .map(|s| (s.range.clone(), highlight(s, cx)))
        .collect();
    let code: Vec<_> = t
        .spans
        .iter()
        .filter(|s| s.role == Some(Role::Code))
        .map(|s| (s.range.clone(), cx.code_font.clone()))
        .collect();
    let mut ranges = Vec::new();
    let mut targets = Vec::new();
    for s in &t.spans {
        match &s.role {
            Some(Role::Link(u)) => {
                ranges.push(s.range.clone());
                targets.push(Target::Url(u.clone()));
            }
            Some(Role::Conversation(c)) => {
                ranges.push(s.range.clone());
                targets.push(Target::Conversation(c.clone()));
            }
            Some(Role::Mention { user: Some(u), .. }) => {
                ranges.push(s.range.clone());
                targets.push(Target::User(u.clone()));
            }
            _ => {}
        }
    }
    // Moves the text when tokens were broken; copies it only otherwise.
    let text = match broken {
        Cow::Owned(n) => n.text,
        Cow::Borrowed(b) => b.text.clone(),
    };
    let styled = StyledText::new(text)
        .with_highlights(highlights)
        .with_font_family_overrides(code);
    if ranges.is_empty() {
        return styled.into_any_element();
    }
    let open = cx.on_open_conversation.clone();
    let open_user = cx.on_open_user.clone();
    let open_url = cx.on_open_url.clone();
    let text =
        InteractiveText::new(next_id(cx, ids), styled).on_click(ranges, move |ix, window, app| {
            match &targets[ix] {
                Target::Url(u) => open_link_external(u, &open_url, window, app),
                Target::Conversation(c) => {
                    if let Some(f) = &open {
                        f(c.clone(), window, app);
                    }
                }
                Target::User(u) => {
                    if let Some(f) = &open_user {
                        f(u.clone(), window, app);
                    }
                }
            }
        });
    // `InteractiveText` sets the pointing-hand cursor over clickable ranges.
    div().relative().child(text).into_any_element()
}

fn render_node(n: &Node, cx: &RichTextCx, ids: &Cell<usize>) -> AnyElement {
    let c = cx.colors;
    match n {
        Node::Text(t) => text_element(t, cx, ids),
        Node::Heading(t) => div()
            .font_weight(FontWeight::BOLD)
            .text_lg()
            .child(text_element(t, cx, ids))
            .into_any_element(),
        Node::Code(t) => div()
            .w_full()
            .p_2()
            .rounded_md()
            .bg(c.code_bg)
            .font_family(cx.code_font.clone())
            .text_sm()
            .child(StyledText::new(t.clone()))
            .into_any_element(),
        Node::Quote(children) => div()
            .flex()
            .flex_col()
            .gap_1()
            .pl_2()
            .border_l_2()
            .border_color(c.border)
            .text_color(c.muted)
            .children(children.iter().map(|n| render_node(n, cx, ids)))
            .into_any_element(),
        Node::List { indent, items } => div()
            .flex()
            .flex_col()
            .pl(px(16.0 * (*indent as f32 + 1.0)))
            .children(items.iter().map(|(marker, t)| {
                div()
                    .flex()
                    .gap_1()
                    .child(div().min_w(px(14.0)).child(marker.clone()))
                    .child(div().flex_1().min_w_0().child(text_element(t, cx, ids)))
            }))
            .into_any_element(),
        Node::Divider => div()
            .w_full()
            .h(px(1.0))
            .my_1()
            .bg(c.border)
            .into_any_element(),
        Node::Image { source, alt } => match cx.image {
            Some(hook) => hook(source, alt),
            None => div()
                .text_xs()
                .text_color(c.muted)
                .child(if alt.is_empty() {
                    t!(cx.localizer, "rich-image").into_owned()
                } else {
                    t!(cx.localizer, "rich-image-alt", alt = alt.as_ref() as &str).into_owned()
                })
                .into_any_element(),
        },
        Node::Context(items) => div()
            .flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .text_xs()
            .text_color(c.muted)
            .children(items.iter().map(|n| render_node(n, cx, ids)))
            .into_any_element(),
        Node::Section {
            text,
            fields,
            accessory,
        } => {
            let mut left = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .children(text.iter().map(|n| render_node(n, cx, ids)));
            if !fields.is_empty() {
                left = left.child(div().flex().flex_wrap().gap_x_4().gap_y_1().children(
                    fields.iter().map(|f| {
                        div()
                            .w(themes::metrics::LAYOUT_FIELD_MD)
                            .child(text_element(f, cx, ids))
                    }),
                ));
            }
            let mut row = div().flex().gap_2().child(left);
            if let Some(a) = accessory {
                row = row.child(div().flex_none().child(render_node(a, cx, ids)));
            }
            row.into_any_element()
        }
        Node::Actions {
            labels,
            open_label,
            open_url,
        } => {
            let mut row = div().flex().flex_wrap().gap_2().items_center();
            for l in labels {
                row = row.child(
                    div()
                        .px_2()
                        .rounded_md()
                        .border_1()
                        .border_color(c.border)
                        .text_xs()
                        .opacity(0.5)
                        .child(l.clone()),
                );
            }
            let link = div().text_xs().text_color(c.link).child(open_label.clone());
            row.child(match open_url {
                Some(u) => {
                    let u = u.clone();
                    let handler = cx.on_open_url.clone();
                    link.id(next_id(cx, ids))
                        .underline()
                        .on_click(move |_, window, app| {
                            open_link_external(&u, &handler, window, app);
                        })
                        .into_any_element()
                }
                None => link.opacity(0.5).into_any_element(),
            })
            .into_any_element()
        }
        Node::Table { header, rows } => div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(c.border)
            .children(rows.iter().enumerate().map(|(ri, r)| {
                div()
                    .flex()
                    .when(*header && ri == 0, |d| d.font_weight(FontWeight::BOLD))
                    .children(r.iter().map(|cell| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .py_0p5()
                            .border_1()
                            .border_color(c.border)
                            .child(text_element(cell, cx, ids))
                    }))
            }))
            .into_any_element(),
        Node::Card { color, children } => {
            let bar = color
                // design: allow card colour is supplied by the message, not the theme
                .map(|v| color::rgb_to_hsla(gpui::rgb(v)))
                .unwrap_or(c.border);
            div()
                .flex()
                .child(div().w(px(4.0)).flex_none().rounded_sm().bg(bar))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .pl_2()
                        .min_w_0()
                        .children(children.iter().map(|n| render_node(n, cx, ids))),
                )
                .into_any_element()
        }
        Node::Call(n) => {
            let join = cx.on_join_huddle.clone();
            let open_url = cx.on_open_url.clone();
            render_call(n, c, next_id(cx, ids), move |a, w, app| match a {
                CallAction::OpenUrl(u) => open_external(u, &open_url, w, app),
                CallAction::JoinHuddle(conv) => {
                    if let Some(j) = &join {
                        j(conv, w, app);
                    }
                }
            })
        }
        Node::Fallback(t) => div()
            .italic()
            .text_color(c.muted)
            .child(t.clone())
            .into_any_element(),
    }
}
