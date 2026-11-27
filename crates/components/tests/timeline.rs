mod common;

use std::sync::Arc;
use std::time::Duration;

use components::{
    DeleteMessage, EditMessage, MarkUnread, Timeline, TimelineEvent, TimelineSource, author_name,
    format_time, group_flags, user_name,
};
use domain::test_support::{cid, member, msg, uid};
use domain::{
    Capabilities, ConversationId, DeliveryState, HistoryPage, Input, Member, Message, MessageId,
    MessageWindow, UserId, WorkspaceState, reduce,
};
use gpui::BorrowAppContext as _;
use gpui::{
    App, AppContext as _, Entity, Focusable as _, ScrollDelta, ScrollEvent, ScrollSource,
    TestAppContext, point, px,
};
use jiff::tz::TimeZone;
use settings::{Clock, WorkspaceKey};

fn m(t: &str, author: &str) -> Message {
    let mut x = msg("C1", t, "hi");
    x.author = Some(uid(author));
    x
}

#[test]
fn grouping_rules() {
    let msgs = vec![
        m("1000.000001", "u1"),
        m("1100.000001", "u1"), // same author, 100 s: grouped
        m("1500.000001", "u1"), // 400 s after previous: new group
        m("1520.000001", "u2"), // other author: new group
        m("1530.000001", "u1"), // other author again: new group
    ];
    assert_eq!(group_flags(&msgs), [false, true, false, false, false]);
    assert!(group_flags(&[]).is_empty());
}

/// A pending row sits below a confirmed one that the server stamped later:
/// it joins that group (same author) instead of showing a header that
/// vanishes once it is confirmed. A confirmed row never groups backwards.
#[test]
fn local_rows_join_the_group_above_on_a_negative_gap() {
    let mut pending = m("1000.000001", "u1");
    pending.state = DeliveryState::Pending;
    let mut failed = m("990.000001", "u1");
    failed.state = DeliveryState::Failed("no".into());
    let msgs = vec![m("1005.000001", "u1"), pending, failed];
    assert_eq!(group_flags(&msgs), [false, true, true]);
    let back = vec![m("1005.000001", "u1"), m("1000.000001", "u1")];
    assert_eq!(group_flags(&back), [false, false]);
    let mut other = m("1000.000001", "u2");
    other.state = DeliveryState::Pending;
    assert_eq!(
        group_flags(&[m("1005.000001", "u1"), other]),
        [false, false]
    );
}

#[test]
fn unresolved_ids_show_a_placeholder_never_the_raw_id() {
    let l = common::en();
    assert_eq!(author_name(None, &l).as_ref(), "Unknown");
    assert_eq!(user_name(None, &l), "Unknown");
    let known = member("U0ABC123", "Ana Reyes");
    assert_eq!(author_name(Some(&known), &l).as_ref(), "Ana Reyes");
    assert_eq!(user_name(Some(&known), &l), "Ana Reyes");
}

#[test]
fn handle_only_member_shows_the_handle() {
    let l = common::en();
    let mut h = member("U0ABC123", "");
    h.display_name = "".into();
    h.real_name = "".into();
    h.handle = "ana".into();
    assert_eq!(author_name(Some(&h), &l).as_ref(), "ana");
    assert_eq!(user_name(Some(&h), &l), "ana");
}

#[test]
fn clock_format_follows_settings() {
    let utc = TimeZone::UTC;
    // 1_700_000_000 = 2023-11-14 22:13:20 UTC
    let t = domain::test_support::ts("1700000000.000100");
    assert_eq!(format_time(&t, Clock::H24, &utc, &common::en()), "22:13");
    assert_eq!(
        format_time(&t, Clock::H12, &utc, &common::en()),
        "10:13\u{202f}PM"
    );
    let bad = domain::test_support::ts("nope");
    assert_eq!(format_time(&bad, Clock::H24, &utc, &common::en()), "");
}

struct Src {
    state: WorkspaceState,
    me: UserId,
    caps: Capabilities,
    loads: Vec<ConversationId>,
    edits: Vec<(MessageId, String)>,
    deletes: Vec<MessageId>,
    unreads: Vec<MessageId>,
    focus: Vec<(ConversationId, bool, bool)>,
    latest: Vec<ConversationId>,
    profiles: Vec<UserId>,
    retries: Vec<domain::ClientMsgId>,
    discards: Vec<domain::ClientMsgId>,
    failed: bool,
    history_retries: Vec<ConversationId>,
    expired: bool,
}

impl Src {
    fn load(&mut self, ms: Vec<Message>, more: bool) {
        reduce(
            &mut self.state,
            Input::HistoryLoaded {
                conversation: cid("C1"),
                page: HistoryPage {
                    messages: ms,
                    has_more: more,
                },
            },
        );
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
    fn load_older(&mut self, c: ConversationId, _cx: &mut App) {
        self.loads.push(c);
    }
    fn history_failed(&self, _: &ConversationId) -> bool {
        self.failed
    }
    fn session_expired(&self) -> bool {
        self.expired
    }
    fn workspace_state(&self) -> Option<&WorkspaceState> {
        Some(&self.state)
    }
    fn retry_history(&mut self, c: ConversationId, _: &mut App) {
        self.history_retries.push(c);
    }
    fn conversation(&self, id: &ConversationId) -> Option<&domain::Conversation> {
        self.state.conversations.get(id)
    }
    fn jump(&self) -> Option<&MessageId> {
        self.state.jump.as_ref()
    }
    fn jump_to_latest(&mut self, c: ConversationId, _cx: &mut App) {
        self.latest.push(c);
    }
    fn edit(&mut self, id: MessageId, text: String, _cx: &mut App) {
        self.edits.push((id, text));
    }
    fn delete(&mut self, id: MessageId, _cx: &mut App) {
        self.deletes.push(id);
    }
    fn react(&mut self, _: MessageId, _: domain::EmojiRef, _: bool, _: &mut App) {}
    fn mark_unread(&mut self, id: MessageId, _cx: &mut App) {
        self.unreads.push(id);
    }
    fn focus(&mut self, c: ConversationId, at_bottom: bool, window_active: bool, _cx: &mut App) {
        self.focus.push((c, at_bottom, window_active));
    }
    fn custom_emoji(&self) -> Vec<domain::CustomEmoji> {
        Vec::new()
    }
    fn open_profile(&mut self, user: UserId, _cx: &mut App) {
        self.profiles.push(user);
    }
    fn retry_send(&mut self, id: domain::ClientMsgId, _cx: &mut App) {
        self.retries.push(id);
    }
    fn discard_send(&mut self, id: domain::ClientMsgId, _cx: &mut App) {
        self.discards.push(id);
    }
}

fn n_msgs(n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let mut x = m(&format!("{}.000001", 1_000_000 + i * 600), "u2");
            x.text = Arc::from(format!("message {i}"));
            x
        })
        .collect()
}

fn setup(
    cx: &mut TestAppContext,
    ms: Vec<Message>,
    more: bool,
    caps: Capabilities,
) -> (
    Entity<Src>,
    Entity<Timeline<Src>>,
    &mut gpui::VisualTestContext,
) {
    setup_in(cx, ms, more, caps, None)
}

/// [`setup`] with `C1` being `conversation`.
fn setup_in(
    cx: &mut TestAppContext,
    ms: Vec<Message>,
    more: bool,
    caps: Capabilities,
    conversation: Option<domain::Conversation>,
) -> (
    Entity<Src>,
    Entity<Timeline<Src>>,
    &mut gpui::VisualTestContext,
) {
    setup_with(cx, caps, conversation, move |s| s.load(ms, more))
}

/// [`setup_in`] with `prep` instead of a history page.
fn setup_with(
    cx: &mut TestAppContext,
    caps: Capabilities,
    conversation: Option<domain::Conversation>,
    prep: impl FnOnce(&mut Src),
) -> (
    Entity<Src>,
    Entity<Timeline<Src>>,
    &mut gpui::VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    let u = uid("u2");
    state.members.insert(u, member("u2", "Bob"));
    let mut s = Src {
        state,
        me: uid("u1"),
        caps,
        loads: vec![],
        edits: vec![],
        deletes: vec![],
        unreads: vec![],
        focus: vec![],
        latest: vec![],
        profiles: vec![],
        retries: vec![],
        discards: vec![],
        failed: false,
        history_retries: vec![],
        expired: false,
    };
    if let Some(c) = conversation {
        s.state.conversations.insert(c.id.clone(), c);
    }
    prep(&mut s);
    let src = cx.new(|_| s);
    let src2 = src.clone();
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src2, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    (src, view, vcx)
}

fn all_caps() -> Capabilities {
    Capabilities {
        edit: true,
        delete: true,
        ..Default::default()
    }
}

/// Scrolls the content down by `dy` (towards older messages) with a wheel event.
fn wheel(vcx: &mut gpui::VisualTestContext, dy: f32) {
    vcx.simulate_event(ScrollEvent {
        position: point(px(50.0), px(50.0)),
        delta: ScrollDelta::Pixels(point(px(0.0), px(dy))),
        source: ScrollSource::Pointer,
        ..Default::default()
    });
}

fn scroll_to_top(vcx: &mut gpui::VisualTestContext) {
    for _ in 0..40 {
        wheel(vcx, 1_000.0);
    }
}

#[gpui::test]
fn only_visible_rows_render(cx: &mut TestAppContext) {
    let (_s, view, vcx) = setup(cx, n_msgs(200), false, all_caps());
    let n = view.read_with(vcx, |v, _| v.rendered_rows());
    assert!(n > 0 && n < 60, "rendered {n}");
}

#[gpui::test]
fn new_message_sticks_to_bottom_only_at_bottom(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, n_msgs(100), false, all_caps());
    assert!(view.read_with(vcx, |v, _| v.is_at_bottom()));
    let mut more = n_msgs(101);
    more.push(m("9000000.000001", "u2"));
    src.update(vcx, |s, cx| {
        s.load(more.clone(), false);
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.is_at_bottom()), "stuck");

    wheel(vcx, 300.0);
    assert!(!view.read_with(vcx, |v, _| v.is_at_bottom()));
    more.push(m("9000100.000001", "u2"));
    src.update(vcx, |s, cx| {
        s.load(more.clone(), false);
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(
        !view.read_with(vcx, |v, _| v.is_at_bottom()),
        "must not jump"
    );
}

#[gpui::test]
fn scroll_to_top_loads_older_once(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, n_msgs(100), true, all_caps());
    scroll_to_top(vcx);
    assert_eq!(src.read_with(vcx, |s, _| s.loads.clone()), vec![cid("C1")]);

    // A new page arrives; reaching the top again requests the next page.
    src.update(vcx, |s, cx| {
        let mut older: Vec<Message> = (0..50)
            .map(|i| m(&format!("{}.000001", 500_000 + i * 600), "u2"))
            .collect();
        older.extend(n_msgs(100));
        s.load(older, true);
        cx.notify();
    });
    vcx.run_until_parked();
    scroll_to_top(vcx);
    assert_eq!(src.read_with(vcx, |s, _| s.loads.len()), 2);
}

#[gpui::test]
fn no_load_when_no_more_history(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, n_msgs(100), false, all_caps());
    scroll_to_top(vcx);
    assert!(src.read_with(vcx, |s, _| s.loads.is_empty()));
}

#[gpui::test]
fn edit_delete_only_on_own_messages(cx: &mut TestAppContext) {
    let mut mine = m("2000000.000001", "u1");
    mine.text = Arc::from("mine");
    let theirs = m("2000100.000001", "u2");
    let mut pending = m("2000200.000001", "u1");
    pending.state = DeliveryState::Pending;
    let (src, view, vcx) = setup(
        cx,
        vec![mine.clone(), theirs.clone(), pending.clone()],
        false,
        all_caps(),
    );
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &TimelineEvent, _| {
            ev.lock().unwrap().push(e.clone())
        })
        .detach()
    });

    // Not own: nothing happens.
    view.update(vcx, |v, cx| v.select(Some(theirs.id.clone()), cx));
    vcx.dispatch_action(EditMessage);
    vcx.dispatch_action(DeleteMessage);
    assert!(events.lock().unwrap().is_empty());
    assert!(view.read_with(vcx, |v, _| v.pending_delete().is_none()));

    // Own: edit emits a request, delete asks for confirmation first.
    view.update(vcx, |v, cx| v.select(Some(mine.id.clone()), cx));
    vcx.dispatch_action(EditMessage);
    assert_eq!(
        events.lock().unwrap().clone(),
        vec![TimelineEvent::EditRequested {
            id: mine.id.clone(),
            text: "mine".into()
        }]
    );
    vcx.dispatch_action(DeleteMessage);
    assert!(src.read_with(vcx, |s, _| s.deletes.is_empty()));
    assert_eq!(
        view.read_with(vcx, |v, _| v.pending_delete().cloned()),
        Some(mine.id.clone())
    );
    view.update(vcx, |v, cx| v.confirm_delete(cx));
    assert_eq!(
        src.read_with(vcx, |s, _| s.deletes.clone()),
        vec![mine.id.clone()]
    );

    // Pending (unsent) own messages are not editable.
    view.update(vcx, |v, cx| v.select(Some(pending.id.clone()), cx));
    let before = events.lock().unwrap().len();
    vcx.dispatch_action(EditMessage);
    assert_eq!(events.lock().unwrap().len(), before);
}

#[gpui::test]
fn delete_asks_for_a_confirmation_dialog_with_the_message_text(cx: &mut TestAppContext) {
    let mut mine = m("2000000.000001", "u1");
    mine.text = Arc::from("delete me");
    let (src, view, vcx) = setup(cx, vec![mine.clone()], false, all_caps());
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &TimelineEvent, _| {
            ev.lock().unwrap().push(e.clone())
        })
        .detach()
    });

    view.update(vcx, |v, cx| v.select(Some(mine.id.clone()), cx));
    vcx.dispatch_action(DeleteMessage);
    assert!(src.read_with(vcx, |s, _| s.deletes.is_empty()));
    // The host needs the message text to show the dialog's preview.
    assert_eq!(
        events.lock().unwrap().clone(),
        vec![TimelineEvent::DeleteRequested {
            id: mine.id.clone(),
            text: "delete me".into(),
        }]
    );
    assert_eq!(
        view.read_with(vcx, |v, _| v.pending_delete().cloned()),
        Some(mine.id.clone())
    );

    // Cancelling the dialog leaves the message; confirming runs the delete.
    view.update(vcx, |v, cx| v.cancel_delete(cx));
    assert_eq!(
        src.read_with(vcx, |s, _| s.deletes.clone()),
        Vec::<MessageId>::new()
    );
    vcx.dispatch_action(DeleteMessage);
    view.update(vcx, |v, cx| v.confirm_delete(cx));
    assert_eq!(
        src.read_with(vcx, |s, _| s.deletes.clone()),
        vec![mine.id.clone()]
    );
}

#[gpui::test]
fn a_conversation_change_cancels_the_pending_delete(cx: &mut TestAppContext) {
    let mine = m("2000000.000001", "u1");
    let (_src, view, vcx) = setup(cx, vec![mine.clone()], false, all_caps());
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ev = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &TimelineEvent, _| {
            ev.lock().unwrap().push(e.clone())
        })
        .detach()
    });
    view.update(vcx, |v, cx| v.select(Some(mine.id.clone()), cx));
    vcx.dispatch_action(DeleteMessage);
    view.update(vcx, |v, cx| v.set_conversation(cid("C2"), cx));
    assert_eq!(
        events.lock().unwrap().last().cloned(),
        Some(TimelineEvent::DeleteCancelled)
    );
    assert_eq!(
        view.read_with(vcx, |v, _| v.pending_delete().cloned()),
        None
    );
}

#[gpui::test]
fn capabilities_gate_edit_delete(cx: &mut TestAppContext) {
    let mine = m("2000000.000001", "u1");
    let (src, view, vcx) = setup(cx, vec![mine.clone()], false, Capabilities::default());
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    view.update(vcx, |v, cx| v.select(Some(mine.id.clone()), cx));
    vcx.dispatch_action(EditMessage);
    vcx.dispatch_action(DeleteMessage);
    assert!(view.read_with(vcx, |v, _| v.pending_delete().is_none()));
    assert!(src.read_with(vcx, |s, _| s.edits.is_empty() && s.deletes.is_empty()));
}

#[gpui::test]
fn mark_unread_action_dispatches(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, n_msgs(5), false, all_caps());
    let id = MessageId {
        conversation: cid("C1"),
        ts: domain::test_support::ts(&format!("{}.000001", 1_000_000 + 2 * 600)),
    };
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    view.update(vcx, |t, cx| t.select(Some(id.clone()), cx));
    vcx.dispatch_action(MarkUnread);
    assert_eq!(src.read_with(vcx, |s, _| s.unreads.clone()), vec![id]);
}

#[gpui::test]
fn focus_reported_on_scroll_to_bottom(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, n_msgs(100), false, all_caps());
    vcx.run_until_parked();
    let last =
        |vcx: &mut gpui::VisualTestContext| src.read_with(vcx, |s, _| s.focus.last().cloned());
    let first = last(vcx).expect("reported on open");
    assert_eq!((first.0, first.1), (cid("C1"), true));
    wheel(vcx, 300.0);
    vcx.run_until_parked();
    assert_eq!(last(vcx).map(|f| f.1), Some(false));
    wheel(vcx, -100_000.0);
    vcx.run_until_parked();
    assert_eq!(last(vcx).map(|f| f.1), Some(true));
}

fn id_at(i: usize) -> MessageId {
    MessageId {
        conversation: cid("C1"),
        ts: domain::test_support::ts(&format!("{}.000001", 1_000_000 + i * 600)),
    }
}

#[gpui::test]
fn reveal_selects_a_loaded_message(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, n_msgs(100), false, all_caps());
    view.update(vcx, |t, cx| t.reveal(id_at(3), cx));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |t, _| t.selected().cloned()),
        Some(id_at(3))
    );
    assert!(!view.read_with(vcx, |t, _| t.is_at_bottom()));
}

#[gpui::test]
fn reveal_loads_older_pages_until_the_message_arrives(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, n_msgs(10)[5..].to_vec(), true, all_caps());
    view.update(vcx, |t, cx| t.reveal(id_at(2), cx));
    vcx.run_until_parked();
    assert!(src.read_with(vcx, |s, _| !s.loads.is_empty()));
    assert_eq!(view.read_with(vcx, |t, _| t.selected().cloned()), None);
    src.update(vcx, |s, cx| {
        s.load(n_msgs(10), false);
        cx.notify();
    });
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |t, _| t.selected().cloned()),
        Some(id_at(2))
    );
}

#[gpui::test]
fn reveal_gives_up_when_history_is_exhausted(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, n_msgs(5), false, all_caps());
    view.update(vcx, |t, cx| t.reveal(id_at(50), cx));
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |t, _| t.selected().cloned()), None);
}

#[gpui::test]
fn an_empty_loaded_conversation_says_so(cx: &mut TestAppContext) {
    let (_s, _view, vcx) = setup(cx, vec![], false, all_caps());
    let b = vcx.debug_bounds("timeline-empty").unwrap();
    let list = vcx.debug_bounds("timeline-list").unwrap();
    assert!(
        b.size.height > gpui::px(50.0) && b.origin.y < list.bottom(),
        "{b:?}"
    );
}

#[gpui::test]
fn a_conversation_still_loading_shows_no_empty_message(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, n_msgs(1), false, all_caps());
    // A fresh conversation: opened, no page yet.
    src.update(vcx, |s, cx| {
        s.state.windows.clear();
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("timeline-empty").is_none());
    // The pane shows a skeleton meanwhile, not a blank.
    assert!(vcx.debug_bounds("timeline-loading").is_some());
    assert!(vcx.debug_bounds("timeline-load-error").is_none());
}

fn unload(src: &Entity<Src>, vcx: &mut gpui::VisualTestContext, failed: bool) {
    src.update(vcx, |s, cx| {
        s.state.windows.clear();
        s.failed = failed;
        cx.notify();
    });
    vcx.run_until_parked();
}

#[gpui::test]
fn a_loaded_conversation_shows_neither_skeleton_nor_error(cx: &mut TestAppContext) {
    let (_s, _view, vcx) = setup(cx, n_msgs(3), false, all_caps());
    assert!(vcx.debug_bounds("timeline-loading").is_none());
    assert!(vcx.debug_bounds("timeline-load-error").is_none());
}

#[gpui::test]
fn a_failed_first_page_shows_an_error_with_retry(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, n_msgs(1), false, all_caps());
    unload(&src, vcx, true);
    assert!(vcx.debug_bounds("timeline-load-error").is_some());
    assert!(vcx.debug_bounds("timeline-loading").is_none());
    let b = vcx.debug_bounds("timeline-retry").expect("retry button");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(
        src.read_with(vcx, |s, _| s.history_retries.clone()),
        vec![cid("C1")]
    );
}

#[gpui::test]
fn retry_is_reachable_and_activated_from_the_keyboard(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, n_msgs(1), false, all_caps());
    unload(&src, vcx, true);
    vcx.update(|w, cx| w.focus_next(cx));
    vcx.simulate_keystrokes("enter");
    assert_eq!(src.read_with(vcx, |s, _| s.history_retries.len()), 1);
}

#[gpui::test]
fn a_conversation_with_messages_shows_no_empty_message(cx: &mut TestAppContext) {
    let (_s, _view, vcx) = setup(cx, n_msgs(3), false, all_caps());
    assert!(vcx.debug_bounds("timeline-empty").is_none());
}

/// Detaches the window from the live end around `target`, as a jump does.
fn detach(src: &mut Src, around: Vec<Message>, target: &Message) {
    reduce(
        &mut src.state,
        Input::AroundLoaded {
            target: target.id.clone(),
            page: domain::AroundPage {
                messages: around,
                has_more_before: true,
                has_more_after: true,
            },
        },
    );
}

#[gpui::test]
fn detached_timeline_shows_jump_to_latest(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, n_msgs(100), true, all_caps());
    assert!(!view.read_with(vcx, |v, cx| v.is_detached(cx)));
    assert!(vcx.debug_bounds("timeline-jump-to-latest").is_none());
    let all = n_msgs(100);
    let target = all[40].clone();
    src.update(vcx, |s, cx| {
        detach(s, all[30..60].to_vec(), &target);
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, cx| v.is_detached(cx)));
    assert!(vcx.debug_bounds("timeline-jump-to-latest").is_some());
    view.update(vcx, |v, cx| v.jump_to_latest(cx));
    assert_eq!(src.read_with(vcx, |s, _| s.latest.clone()), [cid("C1")]);
}

#[gpui::test]
fn jump_target_is_selected_and_highlighted_once(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, n_msgs(100), true, all_caps());
    let all = n_msgs(100);
    let target = all[40].clone();
    src.update(vcx, |s, cx| {
        detach(s, all[30..60].to_vec(), &target);
        cx.notify();
    });
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.highlighted().cloned()),
        Some(target.id.clone())
    );
    assert_eq!(
        view.read_with(vcx, |v, _| v.selected().cloned()),
        Some(target.id.clone())
    );
    // The highlight fades by itself.
    vcx.executor()
        .advance_clock(components::timeline::HIGHLIGHT + std::time::Duration::from_millis(50));
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.highlighted().is_none()));
    // The same jump is not shown again.
    src.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.highlighted().is_none()));
}

#[gpui::test]
fn author_click_opens_profile(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, n_msgs(3), false, all_caps());
    let b = vcx.debug_bounds("message-author-1000000.000001");
    assert!(b.is_some(), "the author name of the first row is a target");
    vcx.simulate_click(b.unwrap().center(), gpui::Modifiers::none());
    assert_eq!(src.read_with(vcx, |s, _| s.profiles.clone()), [uid("u2")]);
}

#[gpui::test]
fn a_density_change_relayouts_the_list(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.settings, r#"{"density": "compact"}"#).unwrap();
    let settings_file = paths.settings.clone();
    cx.update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    let (_src, view, vcx) = setup(cx, n_msgs(5), false, all_caps());
    assert_eq!(
        view.read_with(vcx, |t, _| t.density()),
        settings::Density::Compact
    );
    std::fs::write(&settings_file, r#"{"density": "cozy"}"#).unwrap();
    vcx.update(|_, cx| {
        cx.update_global::<settings::SettingsStore, _>(|s, _| {
            s.reload();
        })
    });
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |t, _| t.density()),
        settings::Density::Cozy
    );
}

#[gpui::test]
fn display_settings_apply_on_reload(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.settings, "{}").unwrap();
    let file = paths.settings.clone();
    cx.update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    let (_src, view, vcx) = setup(cx, n_msgs(3), false, all_caps());
    assert!(view.read_with(vcx, |t, _| t.shows_avatars()));
    assert!(vcx.debug_bounds("message-avatar").is_some());
    assert_eq!(
        view.read_with(vcx, |t, _| t.code_font().to_string()),
        "JetBrains Mono"
    );
    std::fs::write(
        &file,
        r#"{"buffer_font_family": "Menlo",
            "messages": {"show_avatars": false, "clock": "12h", "inline_images": "always"}}"#,
    )
    .unwrap();
    vcx.update(|_, cx| {
        cx.update_global::<settings::SettingsStore, _>(|s, _| {
            s.reload();
        })
    });
    vcx.run_until_parked();
    assert!(!view.read_with(vcx, |t, _| t.shows_avatars()));
    assert!(
        vcx.debug_bounds("message-avatar").is_none(),
        "hidden avatars are not drawn"
    );
    assert_eq!(
        view.read_with(vcx, |t, _| t.code_font().to_string()),
        "Menlo"
    );
    assert_eq!(view.read_with(vcx, |t, _| t.clock()), settings::Clock::H12);
    assert_eq!(
        view.read_with(vcx, |t, _| t.inline_images()),
        settings::InlineImages::Always
    );
}

#[gpui::test]
fn failed_message_offers_retry_delete_and_copy(cx: &mut TestAppContext) {
    let mut ok = m("1000.000001", "u2");
    ok.text = Arc::from("delivered");
    let mut failed = m("2000.000001", "u1");
    failed.text = Arc::from("lost words");
    failed.state = DeliveryState::Failed("network".into());
    failed.client_msg_id = Some(domain::test_support::cmid("f1"));
    let (src, _view, vcx) = setup(cx, vec![ok, failed], false, all_caps());
    // Delivered rows show no actions.
    assert!(
        vcx.debug_bounds("message-failed-retry-1000.000001")
            .is_none()
    );
    for sel in [
        "message-failed-retry-2000.000001",
        "message-failed-delete-2000.000001",
        "message-failed-copy-2000.000001",
    ] {
        let b = vcx
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} is shown"));
        vcx.simulate_click(b.center(), gpui::Modifiers::none());
    }
    src.read_with(vcx, |s, _| {
        assert_eq!(s.retries, [domain::test_support::cmid("f1")]);
        assert_eq!(s.discards, [domain::test_support::cmid("f1")]);
    });
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("lost words")
    );
}

fn open_with_marker(
    cx: &mut TestAppContext,
    unread: bool,
    read_ix: usize,
) -> (
    Entity<Src>,
    Entity<Timeline<Src>>,
    &mut gpui::VisualTestContext,
) {
    open_with_marker_at(cx, unread, Some(read_ix), false)
}

/// `read_ix: None` puts the marker before every loaded message; `more` says
/// older history is still unloaded.
fn open_with_marker_at(
    cx: &mut TestAppContext,
    unread: bool,
    read_ix: Option<usize>,
    more: bool,
) -> (
    Entity<Src>,
    Entity<Timeline<Src>>,
    &mut gpui::VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let ms = n_msgs(4);
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    state.members.insert(uid("u2"), member("u2", "Bob"));
    let mut c = domain::test_support::conv("C1", "general");
    c.unread = unread;
    c.last_read = Some(match read_ix {
        Some(i) => ms[i].id.ts.clone(),
        None => domain::test_support::ts("500.000001"),
    });
    state.conversations.insert(cid("C1"), c);
    let mut s = Src {
        state,
        me: uid("u1"),
        caps: all_caps(),
        loads: vec![],
        edits: vec![],
        deletes: vec![],
        unreads: vec![],
        focus: vec![],
        latest: vec![],
        profiles: vec![],
        retries: vec![],
        discards: vec![],
        failed: false,
        history_retries: vec![],
        expired: false,
    };
    s.load(ms, more);
    let src = cx.new(|_| s);
    let src2 = src.clone();
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src2, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    (src, view, vcx)
}

#[gpui::test]
fn day_divider_opens_the_window(cx: &mut TestAppContext) {
    let (_s, _v, vcx) = setup(cx, n_msgs(3), false, all_caps());
    assert!(vcx.debug_bounds("timeline-day-divider").is_some());
    assert!(vcx.debug_bounds("timeline-new-divider").is_none());
}

#[gpui::test]
fn new_divider_shows_above_first_unread_and_stays_while_reading(cx: &mut TestAppContext) {
    let (src, _v, vcx) = open_with_marker(cx, true, 1);
    let new = vcx.debug_bounds("timeline-new-divider").expect("divider");
    let third = vcx.debug_bounds("timeline-row-1000000.000001").is_some();
    assert!(third);
    let below = vcx.debug_bounds("timeline-row-1001200.000001").unwrap();
    assert!(new.bottom() <= below.top());
    // Reading everything (the marker advances) does not move the divider.
    src.update(vcx, |s, cx| {
        let c = s.state.conversations.get_mut(&cid("C1")).unwrap();
        c.unread = false;
        c.last_read = Some(domain::test_support::ts("1001800.000001"));
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("timeline-new-divider").is_some());
}

#[gpui::test]
fn no_new_divider_when_everything_is_read(cx: &mut TestAppContext) {
    let (_s, _v, vcx) = open_with_marker(cx, false, 1);
    assert!(vcx.debug_bounds("timeline-new-divider").is_none());
}

#[gpui::test]
fn new_divider_waits_for_the_window_to_reach_the_marker(cx: &mut TestAppContext) {
    // Marker older than the loaded page and more history to load: unknown yet.
    let (src, _v, vcx) = open_with_marker_at(cx, true, None, true);
    assert!(vcx.debug_bounds("timeline-new-divider").is_none());
    // History is exhausted: the first loaded message is the first unread one.
    let ms = n_msgs(4);
    src.update(vcx, |s, cx| {
        s.load(ms, false);
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("timeline-new-divider").is_some());
}

#[gpui::test]
fn day_labels_follow_the_local_date_without_a_timer(cx: &mut TestAppContext) {
    use components::timeline_dividers::DayLabel;
    let (src, view, vcx) = setup(cx, n_msgs(3), false, all_caps());
    let at = |secs: i64| {
        std::sync::Arc::new(domain::FixedNow(
            jiff::Timestamp::from_second(secs).unwrap(),
        )) as std::sync::Arc<dyn domain::Now>
    };
    vcx.update(|_, cx| {
        components::set_time_zone(cx, TimeZone::UTC);
        cx.set_global(components::NowGlobal(at(1_000_000 + 3_600)));
    });
    src.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.leads()[0].day),
        Some(DayLabel::Today)
    );
    // The next local day, nothing else changed: only a notify is needed.
    vcx.update(|_, cx| cx.set_global(components::NowGlobal(at(1_000_000 + 86_400))));
    src.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.leads()[0].day),
        Some(DayLabel::Yesterday)
    );
}

fn direct_c1() -> domain::Conversation {
    let mut c = domain::test_support::conv("C1", "Bob");
    c.kind = domain::ConversationKind::Direct;
    c.peer = Some(uid("u2"));
    c
}

/// At the start of a direct message's history the intro sits above the first
/// message, with View profile and Share a file.
#[gpui::test]
fn direct_message_intro_at_history_start(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup_in(cx, n_msgs(3), false, all_caps(), Some(direct_c1()));
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    let intro = vcx.debug_bounds("timeline-intro").expect("intro drawn");
    let first = vcx
        .debug_bounds("timeline-row-1000000.000001")
        .expect("first row drawn");
    assert!(intro.bottom() <= first.top());
    assert!(vcx.debug_bounds("timeline-intro-share").is_some());
    // No provider action adds people to a direct message.
    assert!(vcx.debug_bounds("timeline-intro-add").is_none());
    let b = vcx.debug_bounds("timeline-intro-profile").expect("profile");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(src.read_with(vcx, |s, _| s.profiles.clone()), [uid("u2")]);
}

#[gpui::test]
fn no_intro_while_older_history_remains(cx: &mut TestAppContext) {
    let (_src, _view, vcx) = setup_in(cx, n_msgs(3), true, all_caps(), Some(direct_c1()));
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("timeline-intro").is_none());
}

#[gpui::test]
fn no_intro_in_a_channel(cx: &mut TestAppContext) {
    let (_src, _view, vcx) = setup(cx, n_msgs(3), false, all_caps());
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("timeline-intro").is_none());
}

/// An empty direct message shows the intro instead of the empty line.
#[gpui::test]
fn empty_direct_message_shows_the_intro(cx: &mut TestAppContext) {
    let (_src, _view, vcx) = setup_in(cx, Vec::new(), false, all_caps(), Some(direct_c1()));
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("timeline-intro").is_some());
    assert!(vcx.debug_bounds("timeline-empty").is_none());
}

/// A live message that arrives before the first history page is not the
/// start of history: no intro until the page says there is nothing older.
#[gpui::test]
fn no_intro_before_history_loads(cx: &mut TestAppContext) {
    let (_src, _view, vcx) = setup_with(cx, all_caps(), Some(direct_c1()), |s| {
        reduce(
            &mut s.state,
            Input::Provider(domain::ProviderEvent::MessagePosted(m(
                "1000000.000001",
                "u2",
            ))),
        );
    });
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("timeline-intro").is_none());
}

/// With the session expired nothing can be uploaded: no Share a file.
#[gpui::test]
fn intro_has_no_share_while_the_session_is_expired(cx: &mut TestAppContext) {
    let (_src, _view, vcx) = setup_with(cx, all_caps(), Some(direct_c1()), |s| {
        s.expired = true;
        s.load(n_msgs(2), false);
    });
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("timeline-intro").is_some());
    assert!(vcx.debug_bounds("timeline-intro-share").is_none());
}

/// The intro names the conversation as the header does, and a self DM gets
/// its own line.
#[test]
fn intro_copy_follows_the_header() {
    let l = common::en();
    let mut st = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 10);
    st.self_user = Some(uid("u1"));
    st.members.insert(uid("u1"), member("u1", "Me"));
    st.members.insert(uid("u2"), member("u2", "Bob"));
    let mut g = domain::test_support::conv("G1", "mpdm-bob--carla-1");
    g.kind = domain::ConversationKind::Group;
    g.participants = Arc::from([Arc::<str>::from("bob"), Arc::from("carla")]);
    let (name, line) = components::timeline::intro_copy(&st, &g, &l);
    assert_eq!(name, components::header_info(&st, &g, &l).title);
    assert!(line.contains(&name), "{line}");
    let mut own = domain::test_support::conv("D9", "me");
    own.kind = domain::ConversationKind::Direct;
    own.peer = Some(uid("u1"));
    let (_, line) = components::timeline::intro_copy(&st, &own, &l);
    assert!(line.starts_with("This is your space"), "{line}");
    let (name, line) = components::timeline::intro_copy(&st, &direct_c1(), &l);
    assert_eq!(name, "Bob");
    assert_eq!(
        line,
        "This is the start of your conversation with Bob. Messages here are only between the two of you."
    );
}

/// The server confirming a send gives it a new id (its ts) but not a new row:
/// the selection follows it, the list is not reset (a lone row used to be
/// replaced wholesale) and the scroll position stays.
#[gpui::test]
fn a_confirmed_send_keeps_its_row_selection_and_scroll(cx: &mut TestAppContext) {
    let sent = |ts: &str, state: DeliveryState| {
        let mut x = m(ts, "u1");
        x.text = Arc::from("mine");
        x.client_msg_id = Some(domain::test_support::cmid("k1"));
        x.state = state;
        x
    };
    let mut ms = n_msgs(100);
    ms.push(sent("9000000.000001", DeliveryState::Pending));
    let (src, view, vcx) = setup(cx, ms.clone(), false, all_caps());
    let pending_id = ms[100].id.clone();
    view.update(vcx, |v, cx| v.select(Some(pending_id), cx));
    wheel(vcx, 300.0);
    assert!(!view.read_with(vcx, |v, _| v.is_at_bottom()));

    let acked = sent("9000003.000001", DeliveryState::Delivered);
    ms[100] = acked.clone();
    src.update(vcx, |s, cx| {
        s.load(ms.clone(), false);
        cx.notify();
    });
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.selected().cloned()),
        Some(acked.id.clone()),
        "the selection follows the confirmed message"
    );
    assert!(
        !view.read_with(vcx, |v, _| v.is_at_bottom()),
        "scroll unchanged"
    );
}

/// A clock the test moves.
struct Tick(std::sync::atomic::AtomicI64);

impl domain::Now for Tick {
    fn now(&self) -> jiff::Timestamp {
        jiff::Timestamp::from_second(self.0.load(std::sync::atomic::Ordering::SeqCst)).unwrap()
    }
}

/// A status whose time ran out leaves the message rows by itself: one timer
/// at the expiry repaints, with no session notification and no polling.
#[gpui::test]
fn an_expired_status_leaves_the_rows_at_its_expiry(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering;
    let (src, _view, vcx) = setup(cx, n_msgs(3), false, all_caps());
    let clock = Arc::new(Tick(std::sync::atomic::AtomicI64::new(1_000_010)));
    let now: Arc<dyn domain::Now> = clock.clone();
    vcx.update(|_, cx| {
        components::set_time_zone(cx, TimeZone::UTC);
        cx.set_global(components::NowGlobal(now));
    });
    src.update(vcx, |s, cx| {
        let mut bob = member("u2", "Bob");
        bob.status = Some(domain::Status {
            emoji: Some(domain::EmojiRef {
                shortcode: Arc::from("palm_tree"),
                unicode: Some(Arc::from("\u{1F334}")),
                skin_tone: None,
            }),
            text: Arc::from("Out"),
            expires: Some(1_000_060),
        });
        s.state.members.insert(uid("u2"), bob);
        cx.notify();
    });
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("message-row-status").is_some(), "shown");

    // The time passes; nothing notifies the session.
    clock.0.store(1_000_061, Ordering::SeqCst);
    vcx.executor().advance_clock(Duration::from_secs(51));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("message-row-status").is_none(), "gone");
}
