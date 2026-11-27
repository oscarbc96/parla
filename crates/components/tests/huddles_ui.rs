//! Huddle awareness: sidebar marks, the bar above the timeline and call cards.
mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use components::{
    CallAction, HuddleBar, HuddleSource, Node, RichColors, RichTextCx, SidebarRow, TimelineSource,
    build_nodes, build_rows,
};
use domain::test_support::{cid, conv, member, mid, uid};
use domain::{
    Block, CallCard, CallKind, Capabilities, ConversationId, Huddle, HuddleId, Member,
    MessageWindow, RichText, UserId, WorkspaceState,
};
use gpui::{App, AppContext as _, Entity, TestAppContext};
use jiff::Timestamp;
use settings::{SidebarSettings, WorkspaceKey};

const NOW: i64 = 1_700_000_000;

fn huddle(c: &str, who: &[&str], started: i64) -> Huddle {
    Huddle {
        id: HuddleId(Arc::from(format!("H-{c}"))),
        conversation: cid(c),
        participants: who.iter().map(|u| uid(u)).collect::<Vec<_>>().into(),
        started,
        created_by: who.first().map(|u| uid(u)),
        is_dm: false,
    }
}

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    s.self_user = Some(uid("u1"));
    for (id, name) in [
        ("u1", "Alice"),
        ("u2", "Bob"),
        ("u3", "Carol"),
        ("u4", "Dan"),
        ("u5", "Eve"),
        ("u6", "Frank"),
        ("u7", "Grace"),
    ] {
        s.members.insert(uid(id), member(id, name));
    }
    s
}

// sidebar

#[test]
fn sidebar_shows_huddle_count() {
    let mut s = state();
    let (a, b) = (conv("C1", "general"), conv("C2", "random"));
    s.conversations.insert(a.id.clone(), a);
    s.conversations.insert(b.id.clone(), b);
    s.huddles
        .insert(cid("C1"), huddle("C1", &["u2", "u3", "u4"], NOW));
    let rows = build_rows(
        &s,
        None,
        &SidebarSettings::default(),
        &HashSet::new(),
        &common::en(),
    );
    let of = |id: &str| {
        rows.iter().find_map(|r| match r {
            SidebarRow::Conversation { id: i, huddle, .. } if &*i.0 == id => Some(*huddle),
            _ => None,
        })
    };
    assert_eq!(of("C1"), Some(Some(3)));
    assert_eq!(of("C2"), Some(None));
}

// bar

struct Src {
    state: WorkspaceState,
    joined: Vec<ConversationId>,
}

impl HuddleSource for Src {
    fn huddle(&self, c: &ConversationId) -> Option<&Huddle> {
        self.state.huddles.get(c)
    }
    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn join_huddle(&mut self, c: ConversationId, _: &mut App) {
        self.joined.push(c);
    }
}

fn setup(cx: &mut TestAppContext, h: Option<Huddle>) -> Entity<Src> {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        cx.set_global(components::NowGlobal(Arc::new(domain::FixedNow(
            Timestamp::from_second(NOW).unwrap(),
        ))));
    });
    let mut state = state();
    if let Some(h) = h {
        state.huddles.insert(h.conversation.clone(), h);
    }
    cx.new(|_| Src {
        state,
        joined: vec![],
    })
}

#[gpui::test]
fn bar_lists_participants_and_join(cx: &mut TestAppContext) {
    let s = setup(
        cx,
        Some(huddle(
            "C1",
            &["u2", "u3", "u4", "u5", "u6", "u7"],
            NOW - 12 * 60,
        )),
    );
    let s2 = s.clone();
    let (bar, vcx) = cx.add_window_view(move |_, cx| HuddleBar::new(s2, cid("C1"), cx));
    let v = vcx.update(|_, cx| bar.read(cx).view(cx)).expect("live");
    assert_eq!(v.avatars.len(), 5);
    assert_eq!(v.more, 1);
    assert_eq!(v.started, "Started 12 min ago");
    assert!(v.can_join);
    bar.update(vcx, |b, cx| b.join(cx));
    s.read_with(vcx, |s, _| assert_eq!(s.joined, [cid("C1")]));
}

#[gpui::test]
fn bar_hides_join_when_self_in(cx: &mut TestAppContext) {
    let s = setup(cx, Some(huddle("C1", &["u1", "u2"], NOW - 60)));
    let s2 = s.clone();
    let (bar, vcx) = cx.add_window_view(move |_, cx| HuddleBar::new(s2, cid("C1"), cx));
    let v = vcx.update(|_, cx| bar.read(cx).view(cx)).expect("live");
    assert!(!v.can_join);
    assert_eq!(
        v.note.as_deref(),
        Some("You are in this huddle on another device")
    );
    // A join the user cannot make does nothing.
    bar.update(vcx, |b, cx| b.join(cx));
    s.read_with(vcx, |s, _| assert!(s.joined.is_empty()));
}

/// The header's Huddle button asks the bar's own check: Join where a huddle runs
/// and the user is not in it, nothing when they are, Start only where the
/// provider can.
#[gpui::test]
fn header_huddle_follows_the_bars_own_check(cx: &mut TestAppContext) {
    use components::{HuddleAction, huddle_action};
    let live = setup(cx, Some(huddle("C1", &["u2", "u3"], NOW - 60)));
    cx.update(|cx| {
        assert_eq!(
            huddle_action(live.read(cx), &cid("C1"), cx),
            Some(HuddleAction::Join)
        );
    });
    let mine = setup(cx, Some(huddle("C1", &["u1", "u2"], NOW - 60)));
    cx.update(|cx| assert_eq!(huddle_action(mine.read(cx), &cid("C1"), cx), None));
    // No huddle here and the provider cannot start one: no button.
    let none = setup(cx, None);
    cx.update(|cx| assert_eq!(huddle_action(none.read(cx), &cid("C1"), cx), None));
    // Another conversation's huddle does not offer a join here.
    cx.update(|cx| assert_eq!(huddle_action(live.read(cx), &cid("C2"), cx), None));
}

#[gpui::test]
fn no_huddle_no_bar(cx: &mut TestAppContext) {
    let s = setup(cx, None);
    let (bar, vcx) = cx.add_window_view(move |_, cx| HuddleBar::new(s, cid("C1"), cx));
    assert!(vcx.update(|_, cx| bar.read(cx).view(cx)).is_none());
}

#[gpui::test]
fn duration_timer_only_while_painted(cx: &mut TestAppContext) {
    let s = setup(cx, Some(huddle("C1", &["u2"], NOW - 12 * 60)));
    let s2 = s.clone();
    let (bar, vcx) = cx.add_window_view(move |_, cx| HuddleBar::new(s2, cid("C1"), cx));
    vcx.run_until_parked();
    // Painted with a live huddle: exactly one one-shot timer.
    assert!(bar.read_with(vcx, |b, _| b.timer_armed()));
    // It re-arms after each minute, never accumulates.
    vcx.executor().advance_clock(Duration::from_secs(61));
    vcx.run_until_parked();
    assert!(bar.read_with(vcx, |b, _| b.timer_armed()));
    // The huddle ends: the next paint drops the timer.
    s.update(vcx, |s, _| {
        s.state.huddles.clear();
    });
    bar.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert!(!bar.read_with(vcx, |b, _| b.timer_armed()));
}

// call cards

struct Tl {
    state: WorkspaceState,
    caps: Capabilities,
    live: bool,
}

impl TimelineSource for Tl {
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
        &self.caps
    }
    fn load_older(&mut self, _: ConversationId, _: &mut App) {}
    fn edit(&mut self, _: domain::MessageId, _: String, _: &mut App) {}
    fn delete(&mut self, _: domain::MessageId, _: &mut App) {}
    fn react(&mut self, _: domain::MessageId, _: domain::EmojiRef, _: bool, _: &mut App) {}
    fn custom_emoji(&self) -> Vec<domain::CustomEmoji> {
        Vec::new()
    }
    fn huddle_live(&self, _: &ConversationId) -> bool {
        self.live
    }
}

fn tl(live: bool) -> Tl {
    Tl {
        state: state(),
        caps: Capabilities::default(),
        live,
    }
}

fn nodes(s: &Tl, card: CallCard) -> Vec<Node> {
    let mut cx = RichTextCx::new(
        s,
        RichColors::default(),
        Arc::new(common::en()),
        jiff::tz::TimeZone::UTC,
    );
    cx.tz = jiff::tz::TimeZone::UTC;
    cx.message = Some(mid("C1", "1000.000001"));
    build_nodes(
        &RichText {
            blocks: Arc::from(vec![Block::Call(card)]),
        },
        &cx,
    )
}

fn card(kind: CallKind, ended: Option<i64>) -> CallCard {
    CallCard {
        kind,
        title: None,
        join_url: None,
        started: Some(NOW - 600),
        ended,
        participants: Arc::from([uid("u2"), uid("u3")]),
        created_by: None,
    }
}

#[test]
fn live_and_ended_huddle_cards() {
    let live = nodes(&tl(true), card(CallKind::Huddle, None));
    let Node::Call(c) = &live[0] else {
        panic!("{:?}", live[0])
    };
    assert_eq!(c.heading, "Huddle");
    assert_eq!(c.status, "Live");
    assert_eq!(c.people, ["Bob", "Carol"]);
    let (label, action) = c.join.clone().expect("join while live");
    assert_eq!(label, "Join");
    assert_eq!(action, CallAction::JoinHuddle(cid("C1")));

    // Ended: duration shown, no Join.
    let ended = nodes(
        &tl(false),
        card(CallKind::Huddle, Some(NOW - 600 + 15 * 60)),
    );
    let Node::Call(c) = &ended[0] else { panic!() };
    assert_eq!(c.status, "Ended after 15 min");
    assert!(c.join.is_none());

    // A card that never recorded an end, for a huddle that is gone: no Join,
    // no claim that it is live.
    let stale = nodes(&tl(false), card(CallKind::Huddle, None));
    let Node::Call(c) = &stale[0] else { panic!() };
    assert_eq!(c.status, "Ended");
    assert!(c.join.is_none());
}

#[test]
fn huddle_card_names_the_creator_not_the_sender() {
    // `cx.message` is a message by someone else; the card's creator is Bob.
    let mut c = card(CallKind::Huddle, None);
    c.created_by = Some(uid("u2"));
    let n = nodes(&tl(true), c);
    let Node::Call(c) = &n[0] else { panic!() };
    assert_eq!(c.starter.as_deref(), Some("Started by Bob"));
    let n = nodes(&tl(true), card(CallKind::Huddle, None));
    let Node::Call(c) = &n[0] else { panic!() };
    assert_eq!(c.starter, None);
}

#[test]
fn external_call_card_opens_externally() {
    let mut ext = card(
        CallKind::External {
            provider: Arc::from("Meetly"),
        },
        None,
    );
    ext.title = Some(Arc::from("Weekly sync"));
    ext.join_url = Some(Arc::from("https://meet.example.com/abc"));
    let n = nodes(&tl(false), ext.clone());
    let Node::Call(c) = &n[0] else { panic!() };
    assert_eq!(c.heading, "Weekly sync");
    assert_eq!(c.status, "Meetly");
    let (label, action) = c.join.clone().expect("join");
    assert_eq!(label, "Join");
    assert_eq!(
        action,
        CallAction::OpenUrl("https://meet.example.com/abc".into())
    );

    // Only safe links open; anything else gets no button.
    ext.join_url = Some(Arc::from("javascript:alert(1)"));
    let n = nodes(&tl(false), ext);
    let Node::Call(c) = &n[0] else { panic!() };
    assert!(c.join.is_none());
}
