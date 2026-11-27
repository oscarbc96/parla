mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use components::{
    LinkAction, Node, RenderCache, RichColors, RichTextCx, Role, Timeline, TimelineSource,
    build_nodes, open_link, render_rich_text,
};
use domain::test_support::{cid, member, mid, msg, uid};
use domain::{
    Block, Capabilities, ContentParser, Control, ControlKind, Conversation, ConversationId,
    CustomEmoji, EmojiRef, HistoryPage, Inline, Input, Member, Message, MessageId, MessageWindow,
    PlainParser, RichText, Style, UserId, WorkspaceState, reduce,
};
use gpui::{App, AppContext as _, Entity, IntoElement as _, TestAppContext};
use jiff::tz::TimeZone;
use settings::{Clock, WorkspaceKey};

struct Counting {
    calls: Arc<AtomicUsize>,
}

impl ContentParser for Counting {
    fn parse(&self, m: &Message) -> RichText {
        self.calls.fetch_add(1, Ordering::SeqCst);
        RichText::plain(&m.text)
    }
}

struct Src {
    state: WorkspaceState,
    calls: Arc<AtomicUsize>,
    custom: Vec<CustomEmoji>,
    /// Full copies of the emoji table handed out by `custom_emoji()`.
    copies: Arc<AtomicUsize>,
    parser: Option<Arc<dyn ContentParser>>,
}

impl TimelineSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }
    fn capabilities(&self) -> &Capabilities {
        static CAPS: std::sync::OnceLock<Capabilities> = std::sync::OnceLock::new();
        CAPS.get_or_init(Capabilities::default)
    }
    fn load_older(&mut self, _: ConversationId, _: &mut App) {}
    fn edit(&mut self, _: MessageId, _: String, _: &mut App) {}
    fn delete(&mut self, _: MessageId, _: &mut App) {}
    fn react(&mut self, _: MessageId, _: EmojiRef, _: bool, _: &mut App) {}
    fn custom_emoji(&self) -> Vec<CustomEmoji> {
        self.copies.fetch_add(1, Ordering::SeqCst);
        self.custom.clone()
    }
    fn custom_emoji_image(&self, shortcode: &str) -> Option<domain::ImageRef> {
        self.custom
            .iter()
            .find(|c| &*c.shortcode == shortcode)
            .map(|c| c.image.clone())
    }
    fn content(&self) -> Arc<dyn ContentParser> {
        if let Some(p) = &self.parser {
            return p.clone();
        }
        Arc::new(Counting {
            calls: self.calls.clone(),
        })
    }
    fn conversation(&self, id: &ConversationId) -> Option<&Conversation> {
        self.state.conversations.get(id)
    }
    fn provider_name(&self) -> &str {
        "Fake"
    }
    fn web_url(&self, id: &MessageId) -> Option<String> {
        Some(format!("https://fake.example.test/{}", id.ts.0))
    }
    fn open_conversation(&mut self, _: ConversationId, _: &mut App) {}
    fn user_group(&self, id: &str) -> Option<&domain::UserGroup> {
        self.state.user_groups.get(id)
    }
}

fn src() -> Src {
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    state.self_user = Some(uid("u1"));
    state.members.insert(uid("u1"), member("u1", "Alice"));
    state.members.insert(uid("u2"), member("u2", "Bob"));
    Src {
        state,
        calls: Arc::new(AtomicUsize::new(0)),
        custom: Vec::new(),
        copies: Arc::new(AtomicUsize::new(0)),
        parser: None,
    }
}

fn rcx(s: &Src) -> RichTextCx<'_> {
    let mut cx = RichTextCx::new(
        s,
        RichColors::default(),
        Arc::new(common::en()),
        jiff::tz::TimeZone::UTC,
    );
    cx.tz = TimeZone::UTC;
    cx.clock = Clock::H24;
    cx.message = Some(mid("C1", "1000.000001"));
    cx
}

fn text(s: &str) -> Inline {
    Inline::Text {
        text: s.into(),
        style: Style::default(),
    }
}

fn para(items: Vec<Inline>) -> RichText {
    RichText {
        blocks: Arc::from(vec![Block::Paragraph(items)]),
    }
}

fn only_text(nodes: &[Node]) -> &components::TextNode {
    match &nodes[0] {
        Node::Text(t) => t,
        n => panic!("not text: {n:?}"),
    }
}

#[test]
fn links_only_safe_schemes() {
    for ok in ["https://a.test/x", "http://a.test", "mailto:a@b.test"] {
        assert_eq!(
            open_link(ok),
            LinkAction::OpenExternal(ok.to_string()),
            "{ok}"
        );
    }
    for bad in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "file:///etc/passwd",
        "parla://x",
        "data:text/html,hi",
        "//evil.test",
        "",
    ] {
        assert_eq!(open_link(bad), LinkAction::Refuse, "{bad}");
    }
}

#[test]
fn cache_reuses_and_invalidates_on_edit() {
    let calls = Arc::new(AtomicUsize::new(0));
    let p = Counting {
        calls: calls.clone(),
    };
    let mut cache = RenderCache::new();
    let mut m = msg("C1", "1000.000001", "hi");
    let a = cache.get_or_parse(&m, &p);
    let b = cache.get_or_parse(&m, &p);
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    m.edited = Some(domain::test_support::ts("1000.000900"));
    cache.get_or_parse(&m, &p);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn cache_is_bounded() {
    let mut cache = RenderCache::new();
    for i in 0..600 {
        cache.get_or_parse(&msg("C1", &format!("{i}.000001"), "x"), &PlainParser);
    }
    assert_eq!(cache.len(), 512);
}

#[test]
fn group_mentions_show_the_handle_and_highlight_my_groups() {
    let mut s = src();
    for (id, handle, mine) in [("S1", "eng", true), ("S2", "design", false)] {
        s.state.user_groups.insert(
            id.into(),
            domain::UserGroup {
                id: id.into(),
                handle: handle.into(),
                name: handle.into(),
                member_count: 3,
                is_member: mine,
            },
        );
    }
    let g = |id: &str, label: Option<&str>| Inline::Group {
        id: id.into(),
        label: label.map(Into::into),
    };
    let rt = para(vec![
        g("S1", None),
        text(" "),
        g("S2", Some("@old-name")),
        text(" "),
        g("S9", Some("ops")),
    ]);
    let nodes = build_nodes(&rt, &rcx(&s));
    let t = only_text(&nodes);
    assert_eq!(t.text, "@eng @design @ops");
    let roles: Vec<_> = t.spans.iter().filter_map(|s| s.role.clone()).collect();
    let mention = |me| Role::Mention { me, user: None };
    assert_eq!(roles, [mention(true), mention(false), mention(false)]);
}

#[test]
fn self_mention_highlighted() {
    let s = src();
    let rt = para(vec![
        text("hi "),
        Inline::User {
            id: uid("u1"),
            label: None,
        },
        text(" and "),
        Inline::User {
            id: uid("u2"),
            label: None,
        },
    ]);
    let nodes = build_nodes(&rt, &rcx(&s));
    let t = only_text(&nodes);
    assert_eq!(t.text, "hi @Alice and @Bob");
    let roles: Vec<_> = t.spans.iter().filter_map(|s| s.role.clone()).collect();
    assert_eq!(
        roles,
        [
            Role::Mention {
                me: true,
                user: Some(uid("u1"))
            },
            Role::Mention {
                me: false,
                user: Some(uid("u2"))
            }
        ]
    );
}

#[test]
fn unknown_member_falls_back_to_label() {
    let s = src();
    let rt = para(vec![
        Inline::User {
            id: uid("u9"),
            label: Some("carol".into()),
        },
        text(" "),
        Inline::User {
            id: uid("u8"),
            label: None,
        },
    ]);
    let nodes = build_nodes(&rt, &rcx(&s));
    assert_eq!(only_text(&nodes).text, "@carol @Unknown");
}

#[test]
fn handle_only_member_mention_shows_the_handle() {
    let mut s = src();
    let mut h = member("u7", "");
    h.display_name = "".into();
    h.real_name = "".into();
    h.handle = "dana".into();
    s.state.members.insert(uid("u7"), h);
    let rt = para(vec![Inline::User {
        id: uid("u7"),
        label: None,
    }]);
    let nodes = build_nodes(&rt, &rcx(&s));
    assert_eq!(only_text(&nodes).text, "@dana");
}

#[test]
fn actions_render_disabled_with_open_link() {
    let s = src();
    let rt = RichText {
        blocks: Arc::from(vec![Block::Actions(vec![Control {
            label: "Approve".into(),
            kind: ControlKind::Button,
        }])]),
    };
    let nodes = build_nodes(&rt, &rcx(&s));
    match &nodes[0] {
        Node::Actions {
            labels,
            open_label,
            open_url,
        } => {
            assert_eq!(labels, &["Approve".to_string()]);
            assert_eq!(open_label, "Open in Fake");
            assert_eq!(
                open_url.as_deref(),
                Some("https://fake.example.test/1000.000001")
            );
        }
        n => panic!("{n:?}"),
    }
}

#[test]
fn render_tree_snapshot() {
    let s = src();
    let rt = RichText {
        blocks: Arc::from(vec![
            Block::Paragraph(vec![
                Inline::Text {
                    text: "bold".into(),
                    style: Style {
                        bold: true,
                        ..Default::default()
                    },
                },
                text(" "),
                Inline::Code("x".into()),
                text(" "),
                Inline::Link {
                    url: "https://a.test".into(),
                    label: Some("site".into()),
                    style: Style::default(),
                },
                text(" "),
                Inline::Date {
                    unix: 1_700_000_000,
                    fallback: "fb".into(),
                },
            ]),
            Block::Quote(vec![Block::Paragraph(vec![text("q")])]),
            Block::Code {
                lang: None,
                text: "let a = 1;".into(),
            },
            Block::List {
                style: domain::ListStyle::Ordered { start: 3 },
                indent: 0,
                items: vec![vec![text("a")], vec![text("b")]],
            },
            Block::Divider,
            Block::Unsupported {
                fallback: "nope".into(),
            },
        ]),
    };
    insta::assert_debug_snapshot!(build_nodes(&rt, &rcx(&s)));
}

fn setup(
    cx: &mut TestAppContext,
    n: usize,
) -> (
    Entity<Src>,
    Entity<Timeline<Src>>,
    &mut gpui::VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut s = src();
    let ms: Vec<Message> = (0..n)
        .map(|i| {
            msg(
                "C1",
                &format!("{}.000001", 1_000_000 + i * 600),
                &format!("m{i}"),
            )
        })
        .collect();
    reduce(
        &mut s.state,
        Input::HistoryLoaded {
            conversation: cid("C1"),
            page: HistoryPage {
                messages: ms,
                has_more: false,
            },
        },
    );
    let e = cx.new(|_| s);
    let e2 = e.clone();
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(e2, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    (e, view, vcx)
}

#[gpui::test]
fn only_visible_rows_parse(cx: &mut TestAppContext) {
    let (e, _view, vcx) = setup(cx, 200);
    let n = e.read_with(vcx, |s, _| s.calls.load(Ordering::SeqCst));
    assert!(n > 0 && n < 60, "parsed {n}");
}

#[gpui::test]
fn render_rich_text_builds_an_element(cx: &mut TestAppContext) {
    cx.update(themes::init_default);
    let s = src();
    let rt = para(vec![
        text("a "),
        Inline::Link {
            url: "https://a.test".into(),
            label: None,
            style: Style::default(),
        },
    ]);
    let nodes = build_nodes(&rt, &rcx(&s));
    let t = only_text(&nodes);
    assert!(t.text.starts_with("a "), "{:?}", t.text);
    assert!(
        t.spans
            .iter()
            .any(|sp| matches!(&sp.role, Some(Role::Link(_)))),
        "the link reaches the rendered nodes"
    );
    cx.update(|_| {
        let _ = render_rich_text(&rt, &rcx(&s));
    });
}

fn image_block(url: &str) -> RichText {
    RichText {
        blocks: Arc::from(vec![Block::Image {
            source: domain::ImageRef::Web(url.into()),
            alt: "pic".into(),
            title: None,
        }]),
    }
}

#[gpui::test]
fn image_hook_renders_image_blocks(cx: &mut TestAppContext) {
    let s = src();
    let seen = std::cell::RefCell::new(Vec::new());
    let hook = |r: &domain::ImageRef, alt: &str| {
        seen.borrow_mut().push((r.clone(), alt.to_string()));
        gpui::div().into_any_element()
    };
    let mut rcx = RichTextCx::new(
        &s,
        RichColors::default(),
        Arc::new(common::en()),
        jiff::tz::TimeZone::UTC,
    );
    rcx.image = Some(&hook);
    cx.update(|_| {
        let _ = render_rich_text(&image_block("https://example.invalid/a.png"), &rcx);
    });
    assert_eq!(
        seen.borrow().as_slice(),
        &[(
            domain::ImageRef::Web("https://example.invalid/a.png".into()),
            "pic".to_string()
        )]
    );
}

#[gpui::test]
fn custom_emoji_render_as_images(cx: &mut TestAppContext) {
    let mut s = src();
    let url = domain::ImageRef::Web("https://example.invalid/party.png".into());
    s.custom.push(CustomEmoji {
        shortcode: "party".into(),
        image: url.clone(),
    });
    let rt = RichText {
        blocks: Arc::from(vec![Block::Paragraph(vec![
            Inline::Text {
                text: "hi ".into(),
                style: Style::default(),
            },
            Inline::Emoji(EmojiRef {
                shortcode: "party".into(),
                unicode: None,
                skin_tone: None,
            }),
            Inline::Emoji(EmojiRef {
                shortcode: "unknown".into(),
                unicode: None,
                skin_tone: None,
            }),
        ])]),
    };
    let seen = std::cell::RefCell::new(Vec::new());
    let hook = |r: &domain::ImageRef, alt: &str| {
        seen.borrow_mut().push((r.clone(), alt.to_string()));
        gpui::div().into_any_element()
    };
    let colors = RichColors::default();
    let mut rcx = RichTextCx::new(&s, colors, Arc::new(common::en()), jiff::tz::TimeZone::UTC);
    rcx.emoji = Some(&hook);
    let nodes = build_nodes(&rt, &rcx);
    let Node::Text(t) = &nodes[0] else { panic!() };
    assert_eq!(t.text, "hi :party::unknown:");
    assert!(
        t.spans
            .iter()
            .any(|sp| sp.role == Some(Role::CustomEmoji(url.clone())))
    );
    cx.update(|_| {
        let _ = render_rich_text(&rt, &rcx);
    });
    // Only the emoji with a known image becomes an image; the other stays text.
    assert_eq!(seen.borrow().as_slice(), &[(url, "party".to_string())]);
}

#[test]
fn long_tokens_get_break_opportunities_and_spans_follow() {
    use components::{Span, TextNode, break_long_tokens};
    let long = "a".repeat(50);
    let t = TextNode {
        text: format!("see {long} end"),
        spans: vec![Span {
            range: 4..54,
            style: Style::default(),
            role: Some(Role::Code),
        }],
    };
    let out = break_long_tokens(&t);
    assert!(out.text.contains('\u{200b}'), "break opportunities added");
    // Every unbroken run is short enough to fit a narrow column.
    for word in out
        .text
        .split(|c: char| c.is_whitespace() || c == '\u{200b}')
    {
        assert!(word.chars().count() <= 12, "run {word:?}");
    }
    // The span still covers exactly the long token (plus its inserted breaks).
    let s = &out.spans[0];
    let covered = &out.text[s.range.clone()];
    assert_eq!(covered.replace('\u{200b}', ""), long);
    // Short text is returned untouched.
    let short = TextNode {
        text: "short words only".into(),
        spans: vec![],
    };
    assert_eq!(&*break_long_tokens(&short), &short);
}

/// Parses topics as one linked word, to prove the header goes through the
/// provider parser.
struct TopicParser;

impl ContentParser for TopicParser {
    fn parse(&self, m: &Message) -> RichText {
        RichText::plain(&m.text)
    }
    fn parse_text(&self, text: &str) -> RichText {
        para(vec![Inline::Link {
            url: "https://a.test/x".into(),
            label: Some(text.into()),
            style: Style::default(),
        }])
    }
}

#[test]
fn header_topic_is_parsed_by_the_provider_parser() {
    let rt = TopicParser.parse_text("docs");
    let s = src();
    let nodes = build_nodes(&rt, &rcx(&s));
    let t = only_text(&nodes);
    assert_eq!(t.text, "docs");
    assert!(matches!(&t.spans[0].role, Some(Role::Link(u)) if u.as_ref() == "https://a.test/x"));
}

#[gpui::test]
fn conversation_header_builds_with_a_markup_topic(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut s = src();
    s.state
        .conversations
        .insert(cid("C1"), domain::test_support::conv("C1", "general"));
    let mut c = domain::test_support::conv("C1", "general");
    c.topic = Arc::from("see <https://a.test/x|docs>");
    let (_view, cx) = cx.add_window_view(|_, _| HeaderHost {
        src: s,
        conversation: c,
        actions: components::HeaderActions::default(),
        offset: 0.0,
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("conversation-header").is_some(),
        "the header is drawn"
    );
    assert!(
        cx.debug_bounds("conversation-topic").is_some(),
        "the markup topic is drawn"
    );
}

/// Topic that is a channel link, a space and a mention of Bob.
struct LinkTopic;

impl ContentParser for LinkTopic {
    fn parse(&self, m: &Message) -> RichText {
        RichText::plain(&m.text)
    }
    fn parse_text(&self, _: &str) -> RichText {
        para(vec![
            Inline::Conversation {
                id: cid("C2"),
                label: Some("random".into()),
            },
            text(" "),
            Inline::User {
                id: uid("u2"),
                label: None,
            },
        ])
    }
}

struct HeaderHost {
    src: Src,
    conversation: Conversation,
    actions: components::HeaderActions,
    /// Left offset of the header, like a sidebar or a rail beside it.
    offset: f32,
}

impl gpui::Render for HeaderHost {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use themes::ActiveTheme as _;
        let theme = cx.theme().clone();
        use gpui::{ParentElement as _, Styled as _};
        gpui::div()
            .pl(gpui::px(self.offset))
            .child(components::conversation_header(
                &self.src,
                &self.src.state,
                &self.conversation,
                &theme,
                &self.actions,
                cx,
            ))
    }
}

#[gpui::test]
fn header_topic_channel_and_mention_are_clickable(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let mut s = src();
    s.parser = Some(Arc::new(LinkTopic));
    s.state
        .conversations
        .insert(cid("C2"), domain::test_support::conv("C2", "random"));
    let mut conversation = domain::test_support::conv("C1", "general");
    conversation.topic = Arc::from("anything");
    let actions = components::HeaderActions {
        conversation: Some(Rc::new({
            let o = opened.clone();
            move |c, _, _| o.borrow_mut().push(format!("c:{}", c.0))
        })),
        user: Some(Rc::new({
            let o = opened.clone();
            move |u, _, _| o.borrow_mut().push(format!("u:{}", u.0))
        })),
        ..Default::default()
    };
    let (_v, cx) = cx.add_window_view(|_, _| HeaderHost {
        src: s,
        conversation,
        actions,
        offset: 0.0,
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let topic = cx.debug_bounds("conversation-topic").expect("topic");
    let y = topic.origin.y + topic.size.height / 2.0;
    // "#random" starts the topic; the mention ends it ("#random @Bob").
    cx.simulate_click(
        gpui::point(topic.origin.x + gpui::px(4.0), y),
        gpui::Modifiers::none(),
    );
    cx.simulate_click(
        gpui::point(topic.origin.x + gpui::px(70.0), y),
        gpui::Modifiers::none(),
    );
    assert_eq!(*opened.borrow(), ["c:C2", "u:u2"]);
}

/// The header's buttons call their actions: the member count and the details
/// button open the details panel, the star toggles, the huddle button joins.
#[gpui::test]
fn header_buttons_run_their_actions(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let s = src();
    let mut conversation = domain::test_support::conv("C1", "general");
    conversation.member_count = Some(3);
    let mk = |tag: &'static str| -> components::ToggleFn {
        let l = log.clone();
        Rc::new(move |c, _, _| l.borrow_mut().push(format!("{tag}:{}", c.0)))
    };
    let actions = components::HeaderActions {
        star: Some(mk("star")),
        members: Some(mk("members")),
        details: Some(mk("details")),
        huddle: Some(mk("huddle")),
        pins: Some(mk("pins")),
        ..Default::default()
    };
    let (_v, cx) = cx.add_window_view(|_, _| HeaderHost {
        src: s,
        conversation,
        actions,
        offset: 0.0,
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    for sel in [
        "header-star",
        "header-members",
        "header-details",
        "header-huddle",
        "header-pins",
    ] {
        let b = cx.debug_bounds(sel).unwrap_or_else(|| panic!("{sel}"));
        cx.simulate_click(b.center(), gpui::Modifiers::none());
    }
    assert_eq!(
        *log.borrow(),
        [
            "star:C1",
            "members:C1",
            "details:C1",
            "huddle:C1",
            "pins:C1"
        ]
    );
    // Without an action a button is not drawn.
    assert!(cx.debug_bounds("header-more").is_none());
}

/// The Messages and Files tabs switch the conversation's view: each click
/// asks for its tab, Enter on a focused tab does too, and without a switch
/// action only Messages is drawn.
#[gpui::test]
fn header_tabs_switch_between_messages_and_files(cx: &mut TestAppContext) {
    use components::header::ConversationTab;
    use std::cell::RefCell;
    use std::rc::Rc;
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let log: Rc<RefCell<Vec<ConversationTab>>> = Rc::default();
    let actions = components::HeaderActions {
        tab: ConversationTab::Files,
        switch_tab: Some(Rc::new({
            let l = log.clone();
            move |t, _, _| l.borrow_mut().push(t)
        })),
        ..Default::default()
    };
    let (view, cx) = cx.add_window_view(|_, _| HeaderHost {
        src: src(),
        conversation: domain::test_support::conv("C1", "general"),
        actions,
        offset: 0.0,
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let files = cx.debug_bounds("header-tab-files").expect("files tab");
    cx.simulate_click(files.center(), gpui::Modifiers::none());
    let messages = cx
        .debug_bounds("header-tab-messages")
        .expect("messages tab");
    cx.simulate_click(messages.center(), gpui::Modifiers::none());
    assert_eq!(
        *log.borrow(),
        [ConversationTab::Files, ConversationTab::Messages]
    );
    cx.simulate_keystrokes("tab enter");
    assert_eq!(log.borrow().len(), 3, "a focused tab opens on Enter");
    view.update(cx, |h, cx| {
        h.actions = components::HeaderActions::default();
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("header-tab-messages").is_some());
    assert!(cx.debug_bounds("header-tab-files").is_none());
}

/// The bell and More open their menu under the button that was clicked, wherever
/// the header sits (a sidebar, a rail or a right panel moves it).
#[gpui::test]
fn header_menus_anchor_to_their_own_button(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    for offset in [0.0, 137.0] {
        type Log = Rc<RefCell<Vec<(&'static str, gpui::Point<gpui::Pixels>)>>>;
        let log: Log = Rc::default();
        let mk = |tag: &'static str| -> components::NotifyMenuFn {
            let l = log.clone();
            Rc::new(move |_, at, _, _| l.borrow_mut().push((tag, at)))
        };
        let actions = components::HeaderActions {
            notify_menu: Some(mk("bell")),
            more_menu: Some(mk("more")),
            ..Default::default()
        };
        let (_v, cx) = cx.add_window_view(|_, _| HeaderHost {
            src: src(),
            conversation: domain::test_support::conv("C1", "general"),
            actions,
            offset,
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let mut want = Vec::new();
        for (tag, sel) in [("bell", "header-notify"), ("more", "header-more")] {
            let b = cx.debug_bounds(sel).unwrap_or_else(|| panic!("{sel}"));
            assert!(b.left() >= gpui::px(offset), "{sel} sits after the offset");
            want.push((tag, components::header::menu_anchor(b)));
            cx.simulate_click(b.center(), gpui::Modifiers::none());
        }
        assert_eq!(*log.borrow(), want);
        assert_ne!(want[0].1, want[1].1, "two buttons, two anchors");
    }
}

/// Rendering a row must not copy the workspace's whole emoji table: a
/// workspace has thousands of them and every visible row renders every frame.
#[gpui::test]
fn rendering_custom_emoji_never_copies_the_emoji_table(_cx: &mut TestAppContext) {
    let mut s = src();
    for i in 0..3000 {
        s.custom.push(CustomEmoji {
            shortcode: format!("e{i}").into(),
            image: domain::ImageRef::Web(format!("https://example.invalid/e{i}.png").into()),
        });
    }
    let mut items = Vec::new();
    for i in 0..40 {
        items.push(text("alert "));
        items.push(Inline::Emoji(EmojiRef {
            shortcode: format!("e{}", i * 70).into(),
            unicode: None,
            skin_tone: None,
        }));
    }
    let rt = para(items);
    let hook = |_: &domain::ImageRef, _: &str| gpui::div().into_any_element();
    let mut rcx = RichTextCx::new(
        &s,
        RichColors::default(),
        Arc::new(common::en()),
        jiff::tz::TimeZone::UTC,
    );
    rcx.emoji = Some(&hook);
    for _ in 0..100 {
        let nodes = build_nodes(&rt, &rcx);
        let Node::Text(t) = &nodes[0] else { panic!() };
        assert!(
            t.spans
                .iter()
                .filter(|sp| matches!(sp.role, Some(Role::CustomEmoji(_))))
                .count()
                == 40
        );
    }
    assert_eq!(
        s.copies.load(Ordering::SeqCst),
        0,
        "the emoji table was copied while rendering"
    );
}

/// A direct message's header: the peer's avatar with presence, a line with
/// presence, local time and status, and a profile button for the peer.
#[gpui::test]
fn direct_message_header_shows_the_peer(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let mut s = src();
    s.state.presence.insert(uid("u2"), false);
    let mut conversation = domain::test_support::conv("D1", "Bob");
    conversation.kind = domain::ConversationKind::Direct;
    conversation.peer = Some(uid("u2"));
    let actions = components::HeaderActions {
        user: Some(Rc::new({
            let o = opened.clone();
            move |u, _, _| o.borrow_mut().push(format!("u:{}", u.0))
        })),
        ..Default::default()
    };
    let (_v, cx) = cx.add_window_view(|_, _| HeaderHost {
        src: s,
        conversation,
        actions,
        offset: 0.0,
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("header-dm-avatar").is_some());
    assert!(cx.debug_bounds("header-presence-away").is_some());
    assert!(cx.debug_bounds("header-dm-subtitle").is_some());
    let b = cx.debug_bounds("header-profile").expect("profile button");
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(*opened.borrow(), ["u:u2"]);
}

#[test]
fn inline_code_has_a_padded_chip() {
    let rt = para(vec![
        text("before "),
        Inline::Code("x".into()),
        text(" after"),
    ]);
    let s = src();
    let nodes = build_nodes(&rt, &rcx(&s));
    let Node::Text(t) = &nodes[0] else {
        panic!("not text")
    };
    let span = t
        .spans
        .iter()
        .find(|s| s.role == Some(Role::Code))
        .expect("code span");
    let covered = &t.text[span.range.clone()];
    // A text highlight paints only the glyphs it covers, so the chip gets its
    // horizontal padding from hair spaces inside the same span.
    assert!(
        covered.starts_with('\u{2009}') && covered.ends_with('\u{2009}'),
        "inline code needs hair-space padding: {covered:?}"
    );
    assert_eq!(covered.replace('\u{2009}', ""), "x");
}

#[gpui::test]
fn inline_code_chip_is_visible_in_both_appearances(cx: &mut TestAppContext) {
    fn lin(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn lum(h: gpui::Hsla) -> f32 {
        let c = themes::color::hsla_to_rgba(h);
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }
    fn ratio(a: gpui::Hsla, b: gpui::Hsla) -> f32 {
        let (x, y) = (lum(a), lum(b));
        let (hi, lo) = if x >= y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }
    cx.update(|cx| {
        themes::init_default(cx);
        let reg = themes::ThemeRegistry::global(cx);
        for name in [themes::DEFAULT_LIGHT, themes::DEFAULT_DARK] {
            let theme = reg.get(name).unwrap();
            let c = theme.colors();
            let chip = RichColors::from_theme(&theme).code_bg;
            // The timeline paints messages on the editor ground.
            for ground in [c.editor_background, c.surface_background] {
                let r = ratio(chip, ground);
                assert!(
                    r >= 1.08,
                    "{name}: code chip invisible on message ground ({r:.2}:1)"
                );
            }
        }
    });
}

#[gpui::test]
fn a_faded_palette_dims_every_colour_the_rich_text_draws_with(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        let reg = themes::ThemeRegistry::global(cx);
        for name in [themes::DEFAULT_LIGHT, themes::DEFAULT_DARK] {
            let theme = reg.get(name).unwrap();
            let base = RichColors::from_theme(&theme);
            // The timeline paints messages on the editor ground.
            let ground = theme.colors().editor_background;
            let faded = base.faded(0.5, ground);
            for (label, before, after) in [
                ("text", base.text, faded.text),
                ("muted", base.muted, faded.muted),
                ("link", base.link, faded.link),
                ("code_bg", base.code_bg, faded.code_bg),
                ("border", base.border, faded.border),
                ("mention_bg", base.mention_bg, faded.mention_bg),
                ("raised", base.raised, faded.raised),
                ("success", base.success, faded.success),
                ("on_success", base.on_success, faded.on_success),
                ("subtle", base.subtle, faded.subtle),
            ] {
                if before != ground {
                    assert_ne!(before, after, "{name}: {label} kept its full strength");
                }
                assert_eq!(
                    after,
                    themes::color::faded(before, ground, 0.5),
                    "{name}: {label} is not faded over the row ground"
                );
                let rgba = themes::color::hsla_to_rgba(after);
                assert!(
                    (rgba.a - 1.0).abs() < 0.001,
                    "{name}: {label} is not opaque after fading: {rgba:?}"
                );
            }
        }
    });
}
