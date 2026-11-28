mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::{
    ClosePanel, ComposerSource, OpenThread, ThreadPanel, ThreadPanelEvent, ThreadSource, Timeline,
    TimelineEvent, TimelineSource, ToggleBroadcast, relative_time,
};
use domain::test_support::{cid, member, mid, msg, uid};
use domain::{
    Capabilities, ConversationId, CustomEmoji, EmojiRef, HistoryPage, Input, Member, MessageId,
    MessageWindow, ThreadWindow, UserId, WorkspaceState, reduce,
};
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualContext as _, VisualTestContext};
use settings::{Clock, WorkspaceKey};

struct Src {
    state: WorkspaceState,
    caps: Capabilities,
    opened: Vec<MessageId>,
    closed: Vec<MessageId>,
    replies: Vec<(MessageId, String, bool)>,
    retries: Vec<domain::ClientMsgId>,
    discards: Vec<domain::ClientMsgId>,
    failed: bool,
    reply_retries: Vec<MessageId>,
    follow: Option<bool>,
    follows: Vec<(MessageId, bool)>,
}

impl TimelineSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn thread_open(&self, root: &MessageId) -> bool {
        self.state.threads.contains_key(root)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn self_user(&self) -> Option<&UserId> {
        None
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn load_older(&mut self, _: ConversationId, _: &mut App) {}
    fn edit(&mut self, _: MessageId, _: String, _: &mut App) {}
    fn delete(&mut self, _: MessageId, _: &mut App) {}
    fn react(&mut self, _: MessageId, _: EmojiRef, _: bool, _: &mut App) {}
    fn custom_emoji(&self) -> Vec<CustomEmoji> {
        Vec::new()
    }
    fn retry_send(&mut self, id: domain::ClientMsgId, _: &mut App) {
        self.retries.push(id);
    }
    fn discard_send(&mut self, id: domain::ClientMsgId, _: &mut App) {
        self.discards.push(id);
    }
}

impl ComposerSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn self_user(&self) -> Option<&UserId> {
        None
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
}

impl ThreadSource for Src {
    fn thread(&self, root: &MessageId) -> Option<&ThreadWindow> {
        self.state.threads.get(root)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn open_thread(&mut self, root: MessageId, _: &mut App) {
        self.opened.push(root);
    }
    fn close_thread(&mut self, root: MessageId, _: &mut App) {
        self.closed.push(root);
    }
    fn replies_failed(&self, _: &MessageId) -> bool {
        self.failed
    }
    fn retry_replies(&mut self, root: MessageId, _: &mut App) {
        self.reply_retries.push(root);
    }
    fn reply(&mut self, root: MessageId, text: String, broadcast: bool, _: &mut App) {
        self.replies.push((root, text, broadcast));
    }
    fn thread_follow(&self, _: &MessageId) -> Option<bool> {
        self.follow
    }
    fn set_thread_followed(&mut self, root: MessageId, follow: bool, _: &mut App) {
        self.follows.push((root, follow));
        self.follow = Some(follow);
    }
    fn react(&mut self, _: MessageId, _: EmojiRef, _: bool, _: &mut App) {}
    fn custom_emoji_image(&self, shortcode: &str) -> Option<domain::ImageRef> {
        (shortcode == "party")
            .then(|| domain::ImageRef::Web("https://example.test/party.png".into()))
    }
}

fn root_id() -> MessageId {
    mid("C1", "1000.000001")
}

fn new_src(caps: Capabilities) -> Src {
    new_src_with(caps, false)
}

fn new_src_with(caps: Capabilities, failed_reply: bool) -> Src {
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    state.members.insert(uid("u2"), member("u2", "Bob"));
    state.members.insert(uid("u3"), member("u3", "Cy"));
    let mut root = msg("C1", "1000.000001", "the root");
    root.reply_count = 2;
    root.reply_users = Arc::from(vec![uid("u2"), uid("u3")]);
    root.latest_reply = Some(domain::test_support::ts("1200.000001"));
    reduce(
        &mut state,
        Input::HistoryLoaded {
            conversation: cid("C1"),
            page: HistoryPage {
                messages: vec![root.clone()],
                has_more: false,
            },
        },
    );
    reduce(&mut state, Input::OpenThread(root_id()));
    let mut r1 = msg("C1", "1100.000001", "first reply");
    r1.thread_root = Some(root.id.ts.clone());
    let mut r2 = msg("C1", "1200.000001", "second reply");
    r2.thread_root = Some(root.id.ts.clone());
    r2.author = Some(uid("u3"));
    if failed_reply {
        r2.state = domain::DeliveryState::Failed("network".into());
        r2.client_msg_id = Some(domain::test_support::cmid("r2"));
    }
    reduce(
        &mut state,
        Input::RepliesLoaded {
            root: root_id(),
            page: HistoryPage {
                messages: vec![root, r1, r2],
                has_more: false,
            },
        },
    );
    Src {
        state,
        caps,
        opened: vec![],
        closed: vec![],
        replies: vec![],
        retries: vec![],
        discards: vec![],
        failed: false,
        reply_retries: vec![],
        follow: None,
        follows: vec![],
    }
}

fn threads_caps() -> Capabilities {
    Capabilities {
        threads: true,
        ..Default::default()
    }
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
}

fn timeline(
    cx: &mut TestAppContext,
    caps: Capabilities,
) -> (Entity<Timeline<Src>>, &mut VisualTestContext) {
    init(cx);
    let src = cx.new(|_| new_src(caps));
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    (view, vcx)
}

fn panel(
    cx: &mut TestAppContext,
) -> (
    Entity<Src>,
    Entity<ThreadPanel<Src>>,
    &mut VisualTestContext,
) {
    init(cx);
    let src = cx.new(|_| new_src(threads_caps()));
    let s2 = src.clone();
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| ThreadPanel::new(s2, root_id(), Clock::H24, cx));
    vcx.run_until_parked();
    (src, view, vcx)
}

#[gpui::test]
fn summary_names_unresolved_repliers_with_a_placeholder(cx: &mut TestAppContext) {
    init(cx);
    let src = cx.new(|_| {
        let mut s = new_src(threads_caps());
        s.state.members.remove(&uid("u3"));
        s
    });
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    let s = view
        .read_with(vcx, |v, cx| v.thread_summary(&root_id(), cx))
        .expect("summary");
    let names: Vec<_> = s.authors.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Bob", "Unknown"]);
}

#[gpui::test]
fn closed_thread_summary_shows_the_last_reply_not_the_open_state(cx: &mut TestAppContext) {
    init(cx);
    let mut s = new_src(threads_caps());
    s.state.threads.clear();
    let src = cx.new(|_| s);
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("thread-summary-open").is_none(),
        "a closed thread keeps the last-reply line"
    );
    assert!(vcx.debug_bounds("thread-summary-avatars").is_some());
    let summary = view
        .read_with(vcx, |v, cx| v.thread_summary(&root_id(), cx))
        .expect("summary");
    assert_eq!(summary.label, "2 replies");
}

#[gpui::test]
fn bot_authors_carry_the_app_tag(cx: &mut TestAppContext) {
    init(cx);
    let mut s = new_src(threads_caps());
    s.state.members.get_mut(&uid("u2")).expect("author").is_bot = true;
    let src = cx.new(|_| s);
    let (_view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("message-app-tag").is_some());
}

#[gpui::test]
fn people_authors_carry_no_app_tag(cx: &mut TestAppContext) {
    let (_view, vcx) = timeline(cx, threads_caps());
    assert!(vcx.debug_bounds("message-app-tag").is_none());
}

#[gpui::test]
fn summary_row_shows_count_and_opens(cx: &mut TestAppContext) {
    let (view, vcx) = timeline(cx, threads_caps());
    let s = view
        .read_with(vcx, |v, cx| v.thread_summary(&root_id(), cx))
        .expect("summary");
    assert_eq!(s.label, "2 replies");
    let names: Vec<_> = s.authors.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Bob", "Cy"]);
    assert!(!s.latest.is_empty());
    // The thread is open: the summary pill shows its open state (Thread board).
    assert!(vcx.debug_bounds("thread-summary-open").is_some());
    assert!(vcx.debug_bounds("thread-summary-avatars").is_some());

    let events = Rc::new(RefCell::new(vec![]));
    let e2 = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &TimelineEvent, _| {
            e2.borrow_mut().push(ev.clone())
        })
        .detach()
    });
    view.update(vcx, |v, cx| v.open_thread(root_id(), cx));
    view.update(vcx, |v, cx| v.select(Some(root_id()), cx));
    vcx.focus(&view);
    vcx.dispatch_action(OpenThread);
    assert_eq!(
        *events.borrow(),
        vec![
            TimelineEvent::OpenThread(root_id()),
            TimelineEvent::OpenThread(root_id())
        ]
    );
    assert_eq!(relative_time(1000, 1030, &common::en()), "just now");
    assert_eq!(
        relative_time(1000, 1000 + 5 * 60, &common::en()),
        "5 minutes ago"
    );
    assert_eq!(
        relative_time(1000, 1000 + 3 * 3600, &common::en()),
        "3 hours ago"
    );
    assert_eq!(
        relative_time(1000, 1000 + 2 * 86400, &common::en()),
        "2 days ago"
    );
}

#[gpui::test]
fn panel_lists_root_and_replies(cx: &mut TestAppContext) {
    let (src, view, vcx) = panel(cx);
    let rows = view.read_with(vcx, |p, cx| p.texts(cx));
    assert_eq!(rows, ["the root", "first reply", "second reply"]);
    assert_eq!(src.read_with(vcx, |s, _| s.opened.clone()), vec![root_id()]);
}

#[gpui::test]
fn reply_submits_with_broadcast_flag(cx: &mut TestAppContext) {
    let (src, view, vcx) = panel(cx);
    let composer = view.read_with(vcx, |p, _| p.composer().clone());
    composer.update(vcx, |c, cx| c.set_text("hello".into(), cx));
    vcx.focus(&composer);
    vcx.dispatch_action(ToggleBroadcast);
    assert!(composer.read_with(vcx, |c, _| c.broadcast()));
    vcx.dispatch_action(components::Send);
    vcx.run_until_parked();
    assert_eq!(
        src.read_with(vcx, |s, _| s.replies.clone()),
        vec![(root_id(), "hello".to_string(), true)]
    );
}

#[gpui::test]
fn close_panel_action(cx: &mut TestAppContext) {
    let (src, view, vcx) = panel(cx);
    let events = Rc::new(RefCell::new(vec![]));
    let e2 = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &ThreadPanelEvent, _| {
            e2.borrow_mut().push(ev.clone())
        })
        .detach()
    });
    let composer = view.read_with(vcx, |p, _| p.composer().clone());
    vcx.focus(&composer);
    vcx.dispatch_action(ClosePanel);
    assert_eq!(*events.borrow(), vec![ThreadPanelEvent::Close(root_id())]);
    assert!(
        src.read_with(vcx, |s, _| s.closed.is_empty()),
        "panel emits close; the owner releases the thread"
    );
}

#[gpui::test]
fn header_can_be_hidden_for_a_host_with_its_own(cx: &mut TestAppContext) {
    let (_src, view, vcx) = panel(cx);
    assert!(vcx.debug_bounds("thread-pop-out").is_some());
    assert!(vcx.debug_bounds("thread-close").is_some());
    view.update(vcx, |p, cx| {
        p.set_show_header(false);
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("thread-pop-out").is_none());
    assert!(vcx.debug_bounds("thread-close").is_none());
}

#[gpui::test]
fn threads_hidden_without_capability(cx: &mut TestAppContext) {
    let (view, vcx) = timeline(cx, Capabilities::default());
    assert!(
        view.read_with(vcx, |v, cx| v.thread_summary(&root_id(), cx))
            .is_none()
    );
    let events = Rc::new(RefCell::new(vec![]));
    let e2 = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &TimelineEvent, _| {
            e2.borrow_mut().push(ev.clone())
        })
        .detach()
    });
    view.update(vcx, |v, cx| v.select(Some(root_id()), cx));
    vcx.focus(&view);
    vcx.dispatch_action(OpenThread);
    view.update(vcx, |v, cx| v.open_thread(root_id(), cx));
    assert!(events.borrow().is_empty());
}

#[gpui::test]
fn custom_emoji_reactions_carry_their_image(cx: &mut TestAppContext) {
    init(cx);
    let mut s = new_src(threads_caps());
    let party = EmojiRef {
        shortcode: "party".into(),
        unicode: None,
        skin_tone: None,
    };
    let root = s
        .state
        .threads
        .get_mut(&root_id())
        .unwrap()
        .root
        .as_mut()
        .unwrap();
    root.reactions = Arc::from(vec![domain::Reaction {
        emoji: party,
        count: 1,
        users: Arc::from(vec![uid("u2")]),
        me: false,
    }]);
    let src = cx.new(|_| s);
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| ThreadPanel::new(src, root_id(), Clock::H24, cx));
    vcx.run_until_parked();
    let pills = view.read_with(vcx, |p, cx| p.reaction_pills(&root_id(), cx));
    assert_eq!(pills.len(), 1);
    assert_eq!(
        pills[0].image,
        Some(domain::ImageRef::Web(
            "https://example.test/party.png".into()
        ))
    );
}

#[gpui::test]
fn reveal_scrolls_to_a_reply(cx: &mut TestAppContext) {
    let (_src, view, vcx) = panel(cx);
    let reply = mid("C1", "1200.000001");
    view.update(vcx, |p, cx| p.reveal(reply.clone(), cx));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |p, _| p.last_revealed().cloned()),
        Some(reply)
    );
}

#[gpui::test]
fn failed_reply_offers_actions_and_retry_reaches_it(cx: &mut TestAppContext) {
    init(cx);
    let src = cx.new(|_| new_src_with(threads_caps(), true));
    let s2 = src.clone();
    let (_view, vcx) =
        cx.add_window_view(move |_w, cx| ThreadPanel::new(s2, root_id(), Clock::H24, cx));
    vcx.run_until_parked();
    // Only the failed reply (ts 1200.000001) shows the actions.
    assert!(
        vcx.debug_bounds("message-failed-retry-1100.000001")
            .is_none()
    );
    for sel in [
        "message-failed-delete-1200.000001",
        "message-failed-copy-1200.000001",
    ] {
        assert!(vcx.debug_bounds(sel).is_some(), "{sel} is shown");
    }
    let b = vcx
        .debug_bounds("message-failed-retry-1200.000001")
        .expect("retry is shown");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    let b = vcx
        .debug_bounds("message-failed-delete-1200.000001")
        .expect("delete is shown");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    src.read_with(vcx, |s, _| {
        assert_eq!(s.retries, [domain::test_support::cmid("r2")]);
        assert_eq!(s.discards, [domain::test_support::cmid("r2")]);
    });
}

/// A panel whose replies have not arrived (only the root is held).
fn unloaded_panel(
    cx: &mut TestAppContext,
    failed: bool,
) -> (
    Entity<Src>,
    Entity<ThreadPanel<Src>>,
    &mut VisualTestContext,
) {
    init(cx);
    let src = cx.new(|_| {
        let mut s = new_src(threads_caps());
        let t = s.state.threads.get_mut(&root_id()).unwrap();
        t.replies = MessageWindow::new(1000);
        s.failed = failed;
        s
    });
    let s2 = src.clone();
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| ThreadPanel::new(s2, root_id(), Clock::H24, cx));
    vcx.run_until_parked();
    (src, view, vcx)
}

#[gpui::test]
fn replies_in_flight_show_a_skeleton(cx: &mut TestAppContext) {
    let (_s, _view, vcx) = unloaded_panel(cx, false);
    assert!(vcx.debug_bounds("thread-loading").is_some());
    assert!(vcx.debug_bounds("thread-load-error").is_none());
}

#[gpui::test]
fn loaded_replies_show_neither_skeleton_nor_error(cx: &mut TestAppContext) {
    let (_s, _view, vcx) = panel(cx);
    assert!(vcx.debug_bounds("thread-loading").is_none());
    assert!(vcx.debug_bounds("thread-load-error").is_none());
}

#[gpui::test]
fn failed_replies_show_an_error_and_retry_asks_again(cx: &mut TestAppContext) {
    let (src, _view, vcx) = unloaded_panel(cx, true);
    assert!(vcx.debug_bounds("thread-load-error").is_some());
    assert!(vcx.debug_bounds("thread-loading").is_none());
    let b = vcx.debug_bounds("thread-retry").expect("retry button");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(
        src.read_with(vcx, |s, _| s.reply_retries.clone()),
        vec![root_id()]
    );
}

#[gpui::test]
fn the_bell_shows_only_when_the_thread_can_be_followed(cx: &mut TestAppContext) {
    let (src, _view, vcx) = panel(cx);
    assert!(vcx.debug_bounds("thread-follow").is_none());
    src.update(vcx, |s, cx| {
        s.follow = Some(false);
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("thread-follow").is_some());
}

#[gpui::test]
fn the_bell_follows_and_unfollows(cx: &mut TestAppContext) {
    let (src, view, vcx) = panel(cx);
    src.update(vcx, |s, cx| {
        s.follow = Some(false);
        cx.notify();
    });
    vcx.run_until_parked();
    let at = vcx.debug_bounds("thread-follow").expect("bell").center();
    vcx.simulate_click(at, gpui::Modifiers::none());
    assert_eq!(
        src.read_with(vcx, |s, _| s.follows.clone()),
        [(root_id(), true)]
    );
    assert_eq!(view.read_with(vcx, |p, cx| p.followed(cx)), Some(true));
    // Keyboard: the bell is a tab stop that Enter activates.
    view.update_in(vcx, |p, w, cx| p.focus_follow(w, cx));
    vcx.simulate_keystrokes("enter");
    assert_eq!(
        src.read_with(vcx, |s, _| s.follows.clone()),
        [(root_id(), true), (root_id(), false)]
    );
}

#[gpui::test]
fn the_header_title_bell_and_pop_out_line_up(cx: &mut TestAppContext) {
    let (src, _view, vcx) = panel(cx);
    src.update(vcx, |s, cx| {
        s.follow = Some(true);
        cx.notify();
    });
    vcx.run_until_parked();
    let bell = vcx.debug_bounds("thread-follow").expect("bell");
    let pop = vcx.debug_bounds("thread-pop-out").expect("pop-out");
    let close = vcx.debug_bounds("thread-close").expect("close");
    let title = vcx.debug_bounds("thread-title").expect("title");
    // The pop-out is an icon button the size of the bell and the close.
    assert_eq!(pop.size, bell.size);
    assert_eq!(pop.size, close.size);
    for b in [pop, close, title] {
        let off = (b.center().y - bell.center().y).abs();
        assert!(off < gpui::px(0.5), "{b:?} off the bell's line");
    }
}

#[gpui::test]
fn thread_rows_follow_a_settings_reload(cx: &mut TestAppContext) {
    use gpui::BorrowAppContext as _;
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.settings, "{}").unwrap();
    let file = paths.settings.clone();
    cx.update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    let (_src, view, vcx) = panel(cx);
    assert!(view.read_with(vcx, |p, _| p.shows_avatars()));
    assert!(vcx.debug_bounds("message-avatar").is_some());
    std::fs::write(
        &file,
        r#"{"buffer_font_family": "Menlo", "messages": {"show_avatars": false, "clock": "12h"}}"#,
    )
    .unwrap();
    vcx.update(|_, cx| {
        cx.update_global::<settings::SettingsStore, _>(|s, _| {
            s.reload();
        })
    });
    vcx.run_until_parked();
    assert!(!view.read_with(vcx, |p, _| p.shows_avatars()));
    assert!(vcx.debug_bounds("message-avatar").is_none());
    assert_eq!(
        view.read_with(vcx, |p, _| p.code_font().to_string()),
        "Menlo"
    );
    assert_eq!(view.read_with(vcx, |p, _| p.clock()), Clock::H12);
}
