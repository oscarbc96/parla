//! The channel details panel: tabs, the members list and its filter, Esc and
//! the close button, and the header buttons that open it.
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use components::{
    ChannelDetailsEvent, ChannelDetailsPanel, ChannelDetailsSource, DetailsTab, NotifyControls,
    filter_members, member_rows,
};
use domain::test_support::msg;
use domain::test_support::{cid, conv, member, uid};
use domain::{ConversationId, ConversationKind, HistoryPage, Input, WorkspaceState, reduce};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use settings::WorkspaceKey;

struct Src {
    state: WorkspaceState,
    open: Option<ConversationId>,
    admin: bool,
}

impl NotifyControls for Src {}

impl ChannelDetailsSource for Src {
    fn details_state(&self) -> &WorkspaceState {
        &self.state
    }
    fn details_conversation(&self) -> Option<&ConversationId> {
        self.open.as_ref()
    }
    fn details_can_admin(&self) -> bool {
        self.admin
    }
}

fn en() -> i18n::Localizer {
    common::en()
}

/// A private group of five people the provider lists, Ana active.
fn src() -> Src {
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 10);
    let mut c = conv("G1", "team");
    c.kind = ConversationKind::Group;
    c.topic = Arc::from("Release planning");
    c.member_ids = Arc::from([uid("u1"), uid("u2"), uid("u3"), uid("u4"), uid("u5")]);
    state.conversations.insert(c.id.clone(), c);
    for (id, name) in [
        ("u1", "Me"),
        ("u2", "Ana Reyes"),
        ("u3", "Ben Okafor"),
        ("u4", "Carla Vidal"),
        ("u5", "Dan Weber"),
    ] {
        state.members.insert(uid(id), member(id, name));
    }
    state.self_user = Some(uid("u1"));
    state.presence.insert(uid("u2"), true);
    Src {
        state,
        open: Some(cid("G1")),
        admin: false,
    }
}

fn setup(
    cx: &mut TestAppContext,
) -> (
    Entity<Src>,
    Entity<ChannelDetailsPanel<Src>>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let s = cx.new(|_| src());
    let s2 = s.clone();
    let (panel, vcx) = cx.add_window_view(move |_, cx| ChannelDetailsPanel::new(s2, cx));
    (s, panel, vcx)
}

#[test]
fn members_list_puts_you_then_active_people_first() {
    let s = src();
    let list = member_rows(&s.state, &s.state.conversations[&cid("G1")], &en());
    assert!(!list.recent);
    let names: Vec<_> = list.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Me (you)",
            "Ana Reyes",
            "Ben Okafor",
            "Carla Vidal",
            "Dan Weber"
        ]
    );
    assert_eq!(list.rows[1].active, Some(true));
    assert_eq!(list.rows[2].active, None);
}

#[test]
fn members_filter_matches_names_ignoring_case() {
    let s = src();
    let list = member_rows(&s.state, &s.state.conversations[&cid("G1")], &en());
    let hit = |q: &str| -> Vec<&str> {
        filter_members(&list.rows, q)
            .into_iter()
            .map(|r| r.name.as_str())
            .collect()
    };
    assert_eq!(hit("").len(), 5);
    assert_eq!(hit("  "), hit(""));
    assert_eq!(hit("ben"), ["Ben Okafor"]);
    assert_eq!(hit("VIDAL"), ["Carla Vidal"]);
    assert!(hit("zzz").is_empty());
}

#[gpui::test]
fn panel_filters_the_members_as_the_query_changes(cx: &mut TestAppContext) {
    let (_s, panel, vcx) = setup(cx);
    panel.update(vcx, |p, cx| {
        p.set_tab(DetailsTab::Members, cx);
        assert_eq!(p.visible_members(cx).len(), 5);
        p.set_query("an", cx);
    });
    let names: Vec<String> = panel.read_with(vcx, |p, cx| {
        p.visible_members(cx).into_iter().map(|r| r.name).collect()
    });
    assert_eq!(names, ["Ana Reyes", "Dan Weber"]);
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("details-members-search").is_some());
}

#[gpui::test]
fn tabs_switch_and_start_on_about(cx: &mut TestAppContext) {
    let (_s, panel, vcx) = setup(cx);
    panel.read_with(vcx, |p, _| assert_eq!(p.tab(), DetailsTab::About));
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let members = vcx.debug_bounds("details-tab-members").expect("tab");
    vcx.simulate_click(members.center(), gpui::Modifiers::none());
    panel.read_with(vcx, |p, _| assert_eq!(p.tab(), DetailsTab::Members));
}

#[gpui::test]
fn escape_and_the_close_button_close_the_panel(cx: &mut TestAppContext) {
    let (_s, panel, vcx) = setup(cx);
    let closed = Arc::new(AtomicUsize::new(0));
    let c = closed.clone();
    let sub = vcx.update(|_, cx| {
        cx.subscribe(&panel, move |_, _: &ChannelDetailsEvent, _| {
            c.fetch_add(1, Ordering::SeqCst);
        })
    });
    std::mem::forget(sub);
    vcx.update(|window, cx| {
        let focus = gpui::Focusable::focus_handle(panel.read(cx), cx);
        window.focus(&focus, cx);
    });
    vcx.simulate_keystrokes("escape");
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let x = vcx.debug_bounds("details-close").expect("close");
    vcx.simulate_click(x.center(), gpui::Modifiers::none());
    assert_eq!(closed.load(Ordering::SeqCst), 2);
}

#[gpui::test]
fn about_shows_the_topic_and_the_kind(cx: &mut TestAppContext) {
    let (_s, panel, vcx) = setup(cx);
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("details-about-topic").is_some());
    panel.read_with(vcx, |p, cx| {
        let a = p.about(cx).expect("about");
        assert_eq!(a.topic.as_deref(), Some("Release planning"));
        assert_eq!(a.members, Some(5));
    });
}

/// A channel the provider does not list members for, with `authors` writing in
/// it oldest first.
fn recent_src(authors: &[&str]) -> Src {
    let mut s = src();
    let mut c = conv("C9", "random");
    c.member_count = Some(900);
    s.state.conversations.insert(c.id.clone(), c);
    for a in authors {
        s.state
            .members
            .entry(uid(a))
            .or_insert_with(|| member(a, &format!("Person {a}")));
    }
    let messages = authors
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let mut m = msg("C9", &format!("{}.000001", 1000 + i), "hello");
            m.author = Some(uid(a));
            m
        })
        .collect();
    reduce(
        &mut s.state,
        Input::HistoryLoaded {
            conversation: cid("C9"),
            page: HistoryPage {
                messages,
                has_more: false,
            },
        },
    );
    s.open = Some(cid("C9"));
    s
}

#[test]
fn without_a_member_list_the_people_are_the_recent_authors() {
    let s = recent_src(&["u2", "u3", "u2", "u4"]);
    let c = &s.state.conversations[&cid("C9")];
    let list = member_rows(&s.state, c, &en());
    assert!(list.recent);
    let names: Vec<_> = list.rows.iter().map(|r| r.name.as_str()).collect();
    // Each author once, listed like members (active first, then by name).
    assert_eq!(names, ["Ana Reyes", "Ben Okafor", "Carla Vidal"]);
}

#[test]
fn header_people_stop_after_the_limit_and_say_when_they_are_recent() {
    let authors: Vec<String> = (10..40).map(|i| format!("u{i}")).collect();
    let refs: Vec<&str> = authors.iter().map(String::as_str).collect();
    let s = recent_src(&refs);
    let c = &s.state.conversations[&cid("C9")];
    let (people, recent) = components::recent_people(&s.state, c, 3);
    assert_eq!(people.len(), 3);
    assert!(recent);
    // Newest first.
    assert_eq!(people[0], uid("u39"));
    let (listed, recent) =
        components::recent_people(&s.state, &s.state.conversations[&cid("G1")], 3);
    assert!(!recent);
    assert_eq!(listed.len(), 3);
}

#[gpui::test]
fn the_recent_authors_tab_says_so(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let s = cx.new(|_| recent_src(&["u2", "u3"]));
    let (panel, vcx) = cx.add_window_view(move |_, cx| ChannelDetailsPanel::new(s, cx));
    panel.update(vcx, |p, cx| p.set_tab(DetailsTab::Members, cx));
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("details-members-recent").is_some());
    assert!(vcx.debug_bounds("details-members-more").is_none());
}

#[gpui::test]
fn a_long_member_list_stops_at_the_cap_with_a_more_line(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut base = src();
    let ids: Vec<_> = (0..260).map(|i| uid(&format!("m{i}"))).collect();
    for (i, id) in ids.iter().enumerate() {
        base.state
            .members
            .insert(id.clone(), member(&id.0, &format!("Member {i:03}")));
    }
    let mut c = base.state.conversations[&cid("G1")].clone();
    c.member_ids = ids.into();
    base.state.conversations.insert(c.id.clone(), c);
    let s = cx.new(|_| base);
    let (panel, vcx) = cx.add_window_view(move |_, cx| ChannelDetailsPanel::new(s, cx));
    panel.update(vcx, |p, cx| p.set_tab(DetailsTab::Members, cx));
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("details-members-more").is_some());
    panel.read_with(vcx, |p, cx| assert_eq!(p.visible_members(cx).len(), 260));
}

#[gpui::test]
fn a_different_conversation_starts_on_about_with_no_filter(cx: &mut TestAppContext) {
    let (s, panel, vcx) = setup(cx);
    panel.update(vcx, |p, cx| {
        p.set_tab(DetailsTab::Members, cx);
        p.set_query("ana", cx);
    });
    // Same conversation touched again: the tab and filter stay.
    s.update(vcx, |_, cx| cx.notify());
    panel.read_with(vcx, |p, cx| {
        assert_eq!(p.tab(), DetailsTab::Members);
        assert_eq!(p.visible_members(cx).len(), 1);
    });
    s.update(vcx, |src, cx| {
        let mut c = conv("G2", "other");
        c.kind = ConversationKind::Group;
        src.state.conversations.insert(c.id.clone(), c);
        src.open = Some(cid("G2"));
        cx.notify();
    });
    panel.read_with(vcx, |p, _| assert_eq!(p.tab(), DetailsTab::About));
    panel.update(vcx, |p, cx| {
        p.set_tab(DetailsTab::Members, cx);
    });
    panel.read_with(vcx, |p, cx| {
        // The filter was cleared: the (empty) list is not filtered to nothing.
        assert!(p.visible_members(cx).is_empty());
    });
}

#[gpui::test]
fn with_admin_about_shows_the_description_and_offers_edit_and_add_people(cx: &mut TestAppContext) {
    let (s, panel, vcx) = setup(cx);
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    // Without admin neither button shows.
    assert!(vcx.debug_bounds("details-edit").is_none());
    s.update(vcx, |s, cx| {
        s.admin = true;
        let mut c = conv("C1", "launch");
        c.purpose = Arc::from("Plans for the launch");
        s.state.conversations.insert(c.id.clone(), c);
        s.open = Some(cid("C1"));
        cx.notify();
    });
    vcx.run_until_parked();
    panel.read_with(vcx, |p, cx| {
        assert_eq!(
            p.about(cx).and_then(|a| a.purpose).as_deref(),
            Some("Plans for the launch")
        );
    });
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let sub = vcx.update(|_, cx| {
        cx.subscribe(&panel, move |_, ev: &ChannelDetailsEvent, _| {
            sink.lock().unwrap().push(*ev);
        })
    });
    std::mem::forget(sub);
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("details-about-purpose").is_some());
    let b = vcx.debug_bounds("details-edit").expect("edit");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    panel.update(vcx, |p, cx| p.set_tab(DetailsTab::Members, cx));
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let b = vcx.debug_bounds("details-add-people").expect("add people");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(
        *seen.lock().unwrap(),
        [ChannelDetailsEvent::Edit, ChannelDetailsEvent::Invite]
    );
}
