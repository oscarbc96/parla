mod common;

use std::sync::Arc;

use components::{
    BookmarksBar, CollectionEvent, CollectionsSource, MessageAction, PinsPopover, SavedView,
    SendLaterConfig, ThreadsView, Timeline, TimelineSource, actions_for,
};
use domain::test_support::{cid, member, msg, uid};
use domain::{
    Bookmark, BookmarkId, Capabilities, ContentParser, ConversationId, DeliveryState, HistoryPage,
    InboxThread, Input, Member, Message, MessageId, MessageWindow, Page, Pin, SavedItem, UserId,
    WorkspaceState, reduce,
};
use gpui::{App, AppContext as _, Entity, EventEmitter, TestAppContext, VisualTestContext};
use jiff::tz::TimeZone;
use settings::{Clock, WorkspaceKey};

fn m(ts: &str, author: &str, text: &str) -> Message {
    let mut x = msg("C1", ts, text);
    x.author = Some(uid(author));
    x
}

fn mid(ts: &str) -> MessageId {
    m(ts, "u2", "").id
}

/// A session stub: lists as the test sets them, inputs recorded.
struct Src {
    pins: Option<Vec<Pin>>,
    bookmarks: Option<Vec<Bookmark>>,
    saved: Option<Page<SavedItem>>,
    inbox: Option<Page<InboxThread>>,
    held: Vec<Message>,
    state: WorkspaceState,
    me: UserId,
    caps: Capabilities,
    url: Option<String>,
    inputs: Vec<Input>,
    opened: Vec<String>,
    pinned: Vec<(MessageId, bool)>,
    saved_calls: Vec<(MessageId, bool)>,
    remind: bool,
    reminded: Vec<(MessageId, i64)>,
    remind_zone: Option<jiff::tz::TimeZone>,
    parses: Arc<std::sync::atomic::AtomicUsize>,
}

/// Counts how often a preview parses message text.
struct CountingParser(Arc<std::sync::atomic::AtomicUsize>);

impl ContentParser for CountingParser {
    fn parse(&self, m: &Message) -> domain::RichText {
        domain::RichText::plain(&m.text)
    }
    fn parse_text(&self, text: &str) -> domain::RichText {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        domain::RichText::plain(text)
    }
}

impl Src {
    fn new() -> Src {
        let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
        state.members.insert(uid("u2"), member("u2", "Bob"));
        Src {
            pins: None,
            bookmarks: None,
            saved: None,
            inbox: None,
            held: vec![],
            state,
            me: uid("u1"),
            caps: Capabilities::default(),
            url: None,
            inputs: vec![],
            opened: vec![],
            pinned: vec![],
            saved_calls: vec![],
            remind: false,
            reminded: vec![],
            remind_zone: None,
            parses: Arc::default(),
        }
    }
}

impl CollectionsSource for Src {
    fn pins(&self, _: &ConversationId) -> Option<&[Pin]> {
        self.pins.as_deref()
    }
    fn bookmarks(&self, _: &ConversationId) -> Option<&[Bookmark]> {
        self.bookmarks.as_deref()
    }
    fn saved(&self) -> Option<&Page<SavedItem>> {
        self.saved.as_ref()
    }
    fn thread_inbox(&self) -> Option<&Page<InboxThread>> {
        self.inbox.as_ref()
    }
    fn referenced(&self, id: &MessageId) -> Option<&Message> {
        self.held.iter().find(|m| &m.id == id)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn content(&self) -> Arc<dyn ContentParser> {
        Arc::new(CountingParser(self.parses.clone()))
    }
    fn time_config(&self, _: &App) -> SendLaterConfig {
        SendLaterConfig {
            presets: vec![],
            zone: TimeZone::UTC,
            clock: Clock::H24,
            server: false,
        }
    }
    fn dispatch(&mut self, input: Input, _: &mut App) {
        self.inputs.push(input);
    }
    fn open_external(&mut self, url: &str, _: &mut App) {
        self.opened.push(url.to_string());
    }
}

impl TimelineSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn self_user(&self) -> Option<&UserId> {
        Some(&self.me)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn load_older(&mut self, _: ConversationId, _: &mut App) {}
    fn edit(&mut self, _: MessageId, _: String, _: &mut App) {}
    fn delete(&mut self, _: MessageId, _: &mut App) {}
    fn react(&mut self, _: MessageId, _: domain::EmojiRef, _: bool, _: &mut App) {}
    fn custom_emoji(&self) -> Vec<domain::CustomEmoji> {
        Vec::new()
    }
    fn web_url(&self, _: &MessageId) -> Option<String> {
        self.url.clone()
    }
    fn set_pinned(&mut self, id: MessageId, pinned: bool, _: &mut App) {
        self.pinned.push((id, pinned));
    }
    fn set_saved(&mut self, id: MessageId, saved: bool, _: &mut App) {
        self.saved_calls.push((id, saved));
    }
    fn can_remind(&self) -> bool {
        self.remind
    }
    fn remind_about(&mut self, id: MessageId, at: i64, _: &mut App) {
        self.reminded.push((id, at));
    }
    fn reminder_zone(&self, _: &App) -> Option<jiff::tz::TimeZone> {
        self.remind_zone.clone()
    }
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
}

fn src(cx: &mut TestAppContext, f: impl FnOnce(&mut Src)) -> Entity<Src> {
    let mut s = Src::new();
    f(&mut s);
    cx.new(|_| s)
}

fn pin(ts: &str, at: i64) -> Pin {
    Pin {
        message: mid(ts),
        by: None,
        at,
    }
}

/// Collects the events a view emits.
fn events<V: EventEmitter<CollectionEvent> + 'static>(
    view: &Entity<V>,
    vcx: &mut VisualTestContext,
) -> Arc<std::sync::Mutex<Vec<CollectionEvent>>> {
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let l = log.clone();
    vcx.update(|_, cx| {
        cx.subscribe(view, move |_, ev: &CollectionEvent, _| {
            l.lock().unwrap().push(ev.clone())
        })
        .detach();
    });
    log
}

#[gpui::test]
fn previews_parse_each_message_once(cx: &mut TestAppContext) {
    init(cx);
    let s = src(cx, |s| {
        s.pins = Some(vec![pin("1000.000001", 10), pin("2000.000001", 20)]);
        s.held = vec![
            m("1000.000001", "u2", "old pin"),
            m("2000.000001", "u2", "new pin"),
        ];
    });
    let parses = s.read_with(cx, |s, _| s.parses.clone());
    let s2 = s.clone();
    let (v, vcx) = cx.add_window_view(move |_w, cx| PinsPopover::new(s2, cid("C1"), cx));
    for _ in 0..3 {
        vcx.update(|_, cx| v.read(cx).rows(cx)).expect("loaded");
    }
    assert_eq!(parses.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[gpui::test]
fn pins_popover_lists_and_jumps(cx: &mut TestAppContext) {
    init(cx);
    let s = src(cx, |s| {
        s.pins = Some(vec![pin("1000.000001", 10), pin("2000.000001", 20)]);
        s.held = vec![
            m("1000.000001", "u2", "old pin"),
            m("2000.000001", "u2", "new pin"),
        ];
    });
    let s2 = s.clone();
    let (v, vcx) = cx.add_window_view(move |_w, cx| PinsPopover::new(s2, cid("C1"), cx));
    // Opening asks the session for the list.
    s.read_with(vcx, |s, _| {
        assert!(matches!(&s.inputs[..], [Input::OpenPins(c)] if *c == cid("C1")))
    });
    let rows = vcx.update(|_, cx| v.read(cx).rows(cx)).expect("loaded");
    // Newest pin first, with the author and text.
    let texts: Vec<_> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, ["new pin", "old pin"]);
    assert_eq!(rows[0].author.as_ref(), "Bob");

    let log = events(&v, vcx);
    v.update(vcx, |v, cx| v.unpin(mid("2000.000001"), cx));
    s.read_with(vcx, |s, _| {
        assert!(matches!(
            s.inputs.last(),
            Some(Input::SetPinned { id, pinned: false }) if *id == mid("2000.000001")
        ))
    });
    v.update(vcx, |v, cx| v.jump(mid("1000.000001"), cx));
    assert_eq!(
        *log.lock().unwrap(),
        [
            CollectionEvent::OpenMessage(mid("1000.000001")),
            CollectionEvent::Close
        ]
    );

    // A pin whose message is not held yet reads as loading, not blank.
    s.update(vcx, |s, cx| {
        s.held.clear();
        cx.notify();
    });
    let rows = vcx.update(|_, cx| v.read(cx).rows(cx)).unwrap();
    assert!(rows.iter().all(|r| r.loading));
}

fn bookmark(id: &str, title: &str, link: &str) -> Bookmark {
    Bookmark {
        id: BookmarkId(Arc::from(id)),
        conversation: cid("C1"),
        title: Arc::from(title),
        link: Arc::from(link),
        emoji: None,
    }
}

#[gpui::test]
fn bookmarks_bar_opens_safe_links_only(cx: &mut TestAppContext) {
    init(cx);
    let s = src(cx, |s| {
        s.bookmarks = Some(vec![
            bookmark("B1", "Docs", "https://example.com/docs"),
            bookmark("B2", "Mail", "mailto:team@example.com"),
            bookmark("B3", "Evil", "javascript:alert(1)"),
            bookmark("B4", "File", "file:///etc/passwd"),
        ]);
    });
    let s2 = s.clone();
    let (bar, vcx) = cx.add_window_view(move |_w, cx| BookmarksBar::new(s2, cid("C1"), cx));
    s.read_with(vcx, |s, _| {
        assert!(matches!(&s.inputs[..], [Input::OpenBookmarks(c)] if *c == cid("C1")))
    });
    let chips = vcx.update(|_, cx| bar.read(cx).chips(cx)).expect("loaded");
    assert_eq!(
        chips.iter().map(|c| c.safe).collect::<Vec<_>>(),
        [true, true, false, false]
    );
    for (id, expect) in [
        ("B1", true),
        ("B2", true),
        ("B3", false),
        ("B4", false),
        ("nope", false),
    ] {
        let got = bar.update(vcx, |b, cx| b.open(&BookmarkId(Arc::from(id)), cx));
        assert_eq!(got, expect, "{id}");
    }
    s.read_with(vcx, |s, _| {
        assert_eq!(
            s.opened,
            ["https://example.com/docs", "mailto:team@example.com"]
        )
    });

    // Adding needs a title and a safe link; removing dispatches.
    assert!(!bar.update(vcx, |b, cx| b.add("", "https://example.com", cx)));
    assert!(!bar.update(vcx, |b, cx| b.add("x", "javascript:1", cx)));
    assert!(bar.read_with(vcx, |b, _| b.add_invalid()));
    assert!(bar.update(vcx, |b, cx| b.add(
        " Roadmap ",
        " https://example.com/r ",
        cx
    )));
    bar.update(vcx, |b, cx| b.remove(BookmarkId(Arc::from("B1")), cx));
    s.read_with(vcx, |s, _| {
        let adds: Vec<_> = s
            .inputs
            .iter()
            .filter_map(|i| match i {
                Input::AddBookmark { title, link, .. } => {
                    Some((title.to_string(), link.to_string()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            adds,
            [("Roadmap".to_string(), "https://example.com/r".to_string())]
        );
        assert!(matches!(
            s.inputs.last(),
            Some(Input::RemoveBookmark { id, .. }) if id.0.as_ref() == "B1"
        ));
    });
}

fn saved(ts: &str, at: i64, done: bool) -> SavedItem {
    SavedItem {
        message: mid(ts),
        saved_at: at,
        due: None,
        done,
    }
}

#[gpui::test]
fn later_view_pages_and_removes(cx: &mut TestAppContext) {
    init(cx);
    let s = src(cx, |s| {
        s.saved = Some(Page {
            items: vec![
                saved("1000.000001", 10, true),
                saved("2000.000001", 30, false),
                saved("3000.000001", 20, false),
            ],
            next: Some(Arc::from("c2")),
        });
        s.held = vec![m("2000.000001", "u2", "newer save")];
    });
    let s2 = s.clone();
    let (v, vcx) = cx.add_window_view(move |_w, cx| SavedView::new(s2, cx));
    s.read_with(vcx, |s, _| {
        assert!(matches!(&s.inputs[..], [Input::OpenSaved]))
    });
    let rows = vcx.update(|_, cx| v.read(cx).rows(cx)).expect("loaded");
    // Open items by newest save first, done ones last.
    assert_eq!(
        rows.iter()
            .map(|r| r.preview.message.ts.0.as_ref())
            .collect::<Vec<_>>(),
        ["2000.000001", "3000.000001", "1000.000001"]
    );
    assert_eq!(rows[0].preview.text, "newer save");
    assert!(rows[1].preview.loading && rows[2].done);

    // A next page is asked for once, and only while there is one.
    assert!(vcx.update(|_, cx| v.read(cx).has_more(cx)));
    v.update(vcx, |v, cx| v.more(cx));
    s.update(vcx, |s, cx| {
        s.saved.as_mut().unwrap().next = None;
        cx.notify();
    });
    v.update(vcx, |v, cx| v.more(cx));
    s.read_with(vcx, |s, _| {
        assert_eq!(
            s.inputs
                .iter()
                .filter(|i| matches!(i, Input::MoreSaved))
                .count(),
            1
        )
    });

    v.update(vcx, |v, cx| v.remove(mid("2000.000001"), cx));
    s.read_with(vcx, |s, _| {
        assert!(matches!(
            s.inputs.last(),
            Some(Input::SetSaved { id, saved: false }) if *id == mid("2000.000001")
        ))
    });
    let log = events(&v, vcx);
    v.update(vcx, |v, cx| v.open(mid("3000.000001"), cx));
    assert_eq!(
        *log.lock().unwrap(),
        [CollectionEvent::OpenMessage(mid("3000.000001"))]
    );
}

#[gpui::test]
fn threads_view_opens_thread_panel(cx: &mut TestAppContext) {
    init(cx);
    let s = src(cx, |s| {
        s.inbox = Some(Page {
            items: vec![
                InboxThread {
                    root: mid("1000.000001"),
                    unread_replies: 0,
                    mentions: 0,
                    latest: domain::test_support::ts("5000.000001"),
                },
                InboxThread {
                    root: mid("2000.000001"),
                    unread_replies: 3,
                    mentions: 1,
                    latest: domain::test_support::ts("4000.000001"),
                },
            ],
            next: None,
        });
        s.held = vec![m("2000.000001", "u2", "root with replies")];
    });
    let s2 = s.clone();
    let (v, vcx) = cx.add_window_view(move |_w, cx| ThreadsView::new(s2, cx));
    s.read_with(vcx, |s, _| {
        assert!(matches!(&s.inputs[..], [Input::OpenThreadInbox]))
    });
    let rows = vcx.update(|_, cx| v.read(cx).rows(cx)).expect("loaded");
    // Unread threads come first even when another has newer activity.
    assert_eq!(rows[0].root.message, mid("2000.000001"));
    assert_eq!(rows[0].unread_replies, 3);
    let log = events(&v, vcx);
    v.update(vcx, |v, cx| v.open(rows[0].root.message.clone(), cx));
    assert_eq!(
        *log.lock().unwrap(),
        [CollectionEvent::OpenThread(mid("2000.000001"))]
    );
}

#[gpui::test]
fn threads_view_pages_on_more(cx: &mut TestAppContext) {
    init(cx);
    let s = src(cx, |s| {
        s.inbox = Some(Page {
            items: vec![InboxThread {
                root: mid("1000.000001"),
                unread_replies: 0,
                mentions: 0,
                latest: domain::test_support::ts("5000.000001"),
            }],
            next: Some("c1".into()),
        });
        s.held = vec![m("1000.000001", "u2", "root")];
    });
    let s2 = s.clone();
    let (v, vcx) = cx.add_window_view(move |_w, cx| ThreadsView::new(s2, cx));
    assert!(vcx.update(|_, cx| v.read(cx).has_more(cx)));
    let b = vcx.debug_bounds("threads-more").expect("more button");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    s.read_with(vcx, |s, _| {
        assert!(matches!(
            &s.inputs[..],
            [Input::OpenThreadInbox, Input::MoreThreadInbox]
        ))
    });
    // On the last page there is nothing more to ask for.
    s.update(vcx, |s, _| s.inbox.as_mut().unwrap().next = None);
    v.update(vcx, |v, cx| v.more(cx));
    s.read_with(vcx, |s, _| assert_eq!(s.inputs.len(), 2));
}

fn timeline<'a>(
    cx: &'a mut TestAppContext,
    caps: Capabilities,
    url: Option<&str>,
    messages: Vec<Message>,
) -> (
    Entity<Src>,
    Entity<Timeline<Src>>,
    &'a mut VisualTestContext,
) {
    init(cx);
    let mut s = Src::new();
    s.caps = caps;
    s.url = url.map(str::to_string);
    reduce(
        &mut s.state,
        Input::HistoryLoaded {
            conversation: cid("C1"),
            page: HistoryPage {
                messages,
                has_more: false,
            },
        },
    );
    let src = cx.new(|_| s);
    let s2 = src.clone();
    let (t, vcx) = cx.add_window_view(move |_w, cx| Timeline::new(s2, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    (src, t, vcx)
}

#[gpui::test]
fn copy_link_writes_web_url(cx: &mut TestAppContext) {
    let (_s, t, vcx) = timeline(
        cx,
        Capabilities::default(),
        Some("https://example.com/archives/C1/p1000000001"),
        vec![m("1000.000001", "u2", "hello")],
    );
    assert!(t.update(vcx, |t, cx| t.copy_link(&mid("1000.000001"), cx)));
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("https://example.com/archives/C1/p1000000001")
    );
    // The copy says so in a toast (one, however often it is copied).
    let shown = |vcx: &mut gpui::VisualTestContext| {
        vcx.update(|_, cx| {
            components::Toasts::target(cx)
                .unwrap()
                .read(cx)
                .visible()
                .iter()
                .map(|s| s.title.to_string())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(shown(vcx), ["Link copied"]);
    // The menu action does the same, once the message offers it.
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("stale".into()));
    vcx.update(|w, cx| {
        t.update(cx, |t, cx| {
            t.run_action(MessageAction::CopyLink, mid("1000.000001"), w, cx)
        })
    });
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("https://example.com/archives/C1/p1000000001")
    );
    assert_eq!(shown(vcx), ["Link copied"]);
}

#[gpui::test]
fn no_provider_link_copies_nothing(cx: &mut TestAppContext) {
    let (_s, t, vcx) = timeline(
        cx,
        Capabilities::default(),
        None,
        vec![m("1000.000001", "u2", "hello")],
    );
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("keep".into()));
    assert!(!t.update(vcx, |t, cx| t.copy_link(&mid("1000.000001"), cx)));
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("keep")
    );
    vcx.update(|_, cx| {
        assert!(
            components::Toasts::target(cx).is_none_or(|t| t.read(cx).visible().is_empty()),
            "nothing was copied, nothing is said"
        );
    });
    let acts = t.read_with(vcx, |t, cx| t.actions(&mid("1000.000001"), cx));
    assert!(!acts.contains(&MessageAction::CopyLink));
}

#[test]
fn actions_respect_capabilities() {
    let theirs = m("1000.000001", "u2", "x");
    let mine = m("2000.000001", "u1", "y");
    let none = Capabilities::default();
    let all = Capabilities {
        threads: true,
        edit: true,
        delete: true,
        pins: true,
        saved: true,
        ..Default::default()
    };
    use MessageAction::*;
    // Without capabilities only the universal actions remain.
    assert_eq!(
        actions_for(&theirs, &none, false, false),
        [AddReaction, MarkUnread]
    );
    assert_eq!(
        actions_for(&mine, &none, true, true),
        [AddReaction, CopyLink, MarkUnread]
    );
    // With them, others' messages never offer edit or delete.
    assert_eq!(
        actions_for(&theirs, &all, false, true),
        [ReplyInThread, AddReaction, Save, Pin, CopyLink, MarkUnread]
    );
    assert_eq!(
        actions_for(&mine, &all, true, true),
        [
            ReplyInThread,
            AddReaction,
            Save,
            Pin,
            CopyLink,
            MarkUnread,
            Edit,
            Delete
        ]
    );
    // The flags flip the labels.
    let mut flagged = theirs;
    flagged.pinned = true;
    flagged.saved = true;
    let acts = actions_for(&flagged, &all, false, false);
    assert!(acts.contains(&Unpin) && acts.contains(&RemoveSaved));
    assert!(!acts.contains(&Pin) && !acts.contains(&Save));
    // A message that is still sending or failed offers nothing.
    let mut pending = mine;
    pending.state = DeliveryState::Pending;
    assert!(actions_for(&pending, &all, true, true).is_empty());
}

#[gpui::test]
fn menu_runs_pin_and_save_only_with_capability(cx: &mut TestAppContext) {
    let caps = Capabilities {
        pins: true,
        saved: true,
        ..Default::default()
    };
    let (s, t, vcx) = timeline(cx, caps, None, vec![m("1000.000001", "u2", "hello")]);
    let id = mid("1000.000001");
    for a in [MessageAction::Pin, MessageAction::Save] {
        vcx.update(|w, cx| t.update(cx, |t, cx| t.run_action(a, id.clone(), w, cx)));
    }
    // Edit is not offered to a message of someone else: nothing happens.
    vcx.update(|w, cx| {
        t.update(cx, |t, cx| {
            t.run_action(MessageAction::Delete, id.clone(), w, cx)
        })
    });
    s.read_with(vcx, |s, _| {
        assert_eq!(s.pinned, [(id.clone(), true)]);
        assert_eq!(s.saved_calls, [(id.clone(), true)]);
    });
    assert!(t.read_with(vcx, |t, _| t.pending_delete().is_none()));

    // Without the capabilities the same calls do nothing.
    let (s, t, vcx) = timeline(
        cx,
        Capabilities::default(),
        None,
        vec![m("1000.000001", "u2", "hello")],
    );
    for a in [MessageAction::Pin, MessageAction::Save] {
        vcx.update(|w, cx| t.update(cx, |t, cx| t.run_action(a, id.clone(), w, cx)));
    }
    s.read_with(vcx, |s, _| {
        assert!(s.pinned.is_empty() && s.saved_calls.is_empty())
    });
}

#[gpui::test]
fn remind_me_offers_presets_and_adds_a_reminder(cx: &mut TestAppContext) {
    const NOW: i64 = 1_794_736_800;
    let (s, t, vcx) = timeline(
        cx,
        Capabilities::default(),
        None,
        vec![m("1000.000001", "u2", "hello")],
    );
    vcx.update(|_, cx| {
        components::set_now(
            cx,
            Arc::new(domain::FixedNow(jiff::Timestamp::from_second(NOW).unwrap())),
        )
    });
    let id = mid("1000.000001");
    let acts = t.read_with(vcx, |t, cx| t.actions(&id, cx));
    assert!(!acts.contains(&MessageAction::RemindMe));
    s.update(vcx, |s, _| s.remind = true);
    let acts = t.read_with(vcx, |t, cx| t.actions(&id, cx));
    // After Save for later's place (here: after Add reaction).
    assert_eq!(acts[1], MessageAction::RemindMe);
    vcx.update(|w, cx| {
        t.update(cx, |t, cx| {
            t.run_action(MessageAction::RemindMe, id.clone(), w, cx)
        })
    });
    assert_eq!(
        t.read_with(vcx, |t, _| t.remind_menu().cloned()),
        Some(id.clone())
    );
    t.update(vcx, |t, cx| {
        t.remind(id.clone(), domain::RemindPreset::In1Hour, cx)
    });
    assert_eq!(
        s.read_with(vcx, |s, _| s.reminded.clone()),
        [(id, NOW + 3_600)]
    );
    assert!(t.read_with(vcx, |t, _| t.remind_menu().is_none()));
}

#[gpui::test]
fn remind_me_reads_presets_in_the_reminders_zone(cx: &mut TestAppContext) {
    const NOW: i64 = 1_794_736_800;
    let (s, t, vcx) = timeline(
        cx,
        Capabilities::default(),
        None,
        vec![m("1000.000001", "u2", "hello")],
    );
    let tokyo = jiff::tz::TimeZone::get("Asia/Tokyo").unwrap();
    vcx.update(|_, cx| {
        components::set_now(
            cx,
            Arc::new(domain::FixedNow(jiff::Timestamp::from_second(NOW).unwrap())),
        )
    });
    s.update(vcx, |s, _| {
        s.remind = true;
        s.remind_zone = Some(tokyo.clone());
    });
    let id = mid("1000.000001");
    t.update(vcx, |t, cx| {
        t.remind(id.clone(), domain::RemindPreset::Tomorrow, cx)
    });
    let want = domain::RemindPreset::Tomorrow
        .at(&jiff::Timestamp::from_second(NOW)
            .unwrap()
            .to_zoned_datetime(tokyo))
        .unwrap();
    assert_eq!(s.read_with(vcx, |s, _| s.reminded.clone()), [(id, want)]);
}

#[gpui::test]
fn the_remind_strip_takes_the_focus_toggles_and_closes_on_escape(cx: &mut TestAppContext) {
    let (s, t, vcx) = timeline(
        cx,
        Capabilities::default(),
        None,
        vec![m("1000.000001", "u2", "hello")],
    );
    s.update(vcx, |s, _| s.remind = true);
    let id = mid("1000.000001");
    let open = |vcx: &mut gpui::VisualTestContext| {
        vcx.update(|w, cx| {
            t.update(cx, |t, cx| {
                t.run_action(MessageAction::RemindMe, id.clone(), w, cx)
            })
        });
        vcx.run_until_parked();
    };
    open(vcx);
    assert!(t.read_with(vcx, |t, _| t.remind_menu().is_some()));
    assert!(
        vcx.update(|w, cx| t.read(cx).remind_focus().is_focused(w)),
        "the first preset has the focus"
    );
    // The action again closes it.
    open(vcx);
    assert!(t.read_with(vcx, |t, _| t.remind_menu().is_none()));
    open(vcx);
    vcx.simulate_keystrokes("escape");
    assert!(t.read_with(vcx, |t, _| t.remind_menu().is_none()));
    assert!(s.read_with(vcx, |s, _| s.reminded.is_empty()));
}
