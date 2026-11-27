mod common;

use std::sync::Arc;

use std::collections::HashSet;

use components::{
    MemberIndex, NextUnread, Open, SelectNext, SelectPrev, Sidebar, SidebarRow, SidebarSource,
    build_rows, build_rows_pinned, next_unread,
};
use domain::{
    Conversation, ConversationId, ConversationKind, Member, MessageTs, ProviderExt, UserId,
    WorkspaceState,
};
use gpui::{App, AppContext as _, Entity, Focusable as _, TestAppContext};
use settings::{SidebarSettings, SidebarSort, WorkspaceKey};

fn conv(id: &str, kind: ConversationKind, name: &str) -> Conversation {
    Conversation {
        id: ConversationId(Arc::from(id)),
        kind,
        name: Arc::from(name),
        peer: None,
        is_member: true,
        is_archived: false,
        is_open: true,
        is_muted: false,
        last_read: None,
        latest: None,
        unread: false,
        mentions: 0,
        participants: Arc::from([]),
        member_ids: Arc::from([]),
        topic: Default::default(),
        member_count: None,
        purpose: Default::default(),
        ext: ProviderExt::none(),
    }
}

fn state(cs: Vec<Conversation>) -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    for c in cs {
        s.conversations.insert(c.id.clone(), c);
    }
    s
}

fn rows_for(s: &WorkspaceState, sel: Option<&ConversationId>) -> Vec<SidebarRow> {
    build_rows(
        s,
        sel,
        &SidebarSettings::default(),
        &HashSet::new(),
        &common::en(),
    )
}

fn unread(mut c: Conversation, mentions: u32, latest: &str) -> Conversation {
    c.unread = true;
    c.mentions = mentions;
    c.latest = Some(MessageTs(Arc::from(latest)));
    c
}

fn flags(rows: &[SidebarRow], label: &str) -> Vec<(bool, bool)> {
    rows.iter()
        .filter_map(|r| match r {
            SidebarRow::Conversation {
                label: l,
                muted,
                draft,
                ..
            } if &**l == label => Some((*muted, *draft)),
            _ => None,
        })
        .collect()
}

fn labels(rows: &[SidebarRow]) -> Vec<String> {
    rows.iter()
        .map(|r| match r {
            SidebarRow::Header { label: h, .. } => format!("#{h}"),
            SidebarRow::Conversation { label, .. } => label.to_string(),
            SidebarRow::ShowMore { label, .. } => format!("+{label}"),
        })
        .collect()
}

#[test]
fn rows_grouped_and_sorted() {
    let mut dm_old = conv("D1", ConversationKind::Direct, "");
    dm_old.peer = Some(UserId(Arc::from("U1")));
    dm_old.latest = Some(MessageTs(Arc::from("100.000001")));
    let mut dm_new = conv("D2", ConversationKind::Group, "grp");
    dm_new.latest = Some(MessageTs(Arc::from("200.000001")));
    let mut s = state(vec![
        conv("C2", ConversationKind::Channel, "zeta"),
        conv("C1", ConversationKind::PrivateChannel, "alpha"),
        dm_old,
        dm_new,
    ]);
    let uid = UserId(Arc::from("U1"));
    s.members.insert(
        uid.clone(),
        Member {
            id: uid,
            display_name: Arc::from("Bob"),
            real_name: Arc::from("Bob"),
            handle: Arc::from(""),
            is_bot: false,
            deleted: false,
            status: None,
            title: None,
            tz: None,
            avatar: None,
            ext: ProviderExt::none(),
        },
    );
    let rows = rows_for(&s, Some(&ConversationId(Arc::from("C2"))));
    assert_eq!(
        labels(&rows),
        [
            "#Channels",
            "alpha",
            "zeta",
            "#Direct messages",
            "grp",
            "Bob"
        ]
    );
    assert!(matches!(
        &rows[2],
        SidebarRow::Conversation { selected: true, .. }
    ));
}

#[test]
fn archived_and_non_member_hidden() {
    let mut a = conv("C1", ConversationKind::Channel, "archived");
    a.is_archived = true;
    let mut n = conv("C2", ConversationKind::Channel, "notmember");
    n.is_member = false;
    let s = state(vec![a, n, conv("C3", ConversationKind::Channel, "ok")]);
    assert_eq!(labels(&rows_for(&s, None)), ["#Channels", "ok"]);
    assert!(rows_for(&state(vec![]), None).is_empty());
}

struct Src {
    rows: Vec<SidebarRow>,
    opened: Vec<ConversationId>,
    calls: std::cell::Cell<usize>,
    toggles: usize,
    workspace_name: Option<String>,
}

impl SidebarSource for Src {
    fn rows(&self, _cx: &App) -> Vec<SidebarRow> {
        self.calls.set(self.calls.get() + 1);
        self.rows.clone()
    }
    fn open(&mut self, id: ConversationId, _cx: &mut App) {
        self.opened.push(id);
    }
    fn toggle_direct_messages(&mut self, _cx: &mut App) {
        self.toggles += 1;
    }
    fn workspace_name(&self, _cx: &App) -> Option<gpui::SharedString> {
        self.workspace_name
            .as_ref()
            .map(|n| gpui::SharedString::from(n.clone()))
    }
}

fn setup(
    cx: &mut TestAppContext,
    n: usize,
) -> (
    Entity<Src>,
    Entity<Sidebar<Src>>,
    &mut gpui::VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let s = state(
        (0..n)
            .map(|i| {
                conv(
                    &format!("C{i:05}"),
                    ConversationKind::Channel,
                    &format!("chan{i:05}"),
                )
            })
            .collect(),
    );
    let src = cx.new(|_| Src {
        rows: rows_for(&s, None),
        opened: vec![],
        calls: Default::default(),
        toggles: 0,
        workspace_name: None,
    });
    let src2 = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| Sidebar::new(src2, cx));
    (src, view, vcx)
}

#[gpui::test]
fn select_next_and_open(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, 3);
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    // The "Channels" header is a stop too.
    for _ in 0..3 {
        vcx.dispatch_action(SelectNext);
    }
    vcx.dispatch_action(Open);
    let opened = src.read_with(vcx, |s, _| s.opened.clone());
    assert_eq!(opened, vec![ConversationId(Arc::from("C00001"))]);
}

#[gpui::test]
fn click_leaves_no_focus_ring_only_the_keyboard_does(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, 3);
    vcx.run_until_parked();
    let b = vcx.debug_bounds("sidebar-row-C00001").expect("row drawn");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    assert_eq!(vcx.update(|w, cx| view.read(cx).ring_row(w).cloned()), None);
    // The first stop is the "Channels" header, the next a conversation.
    vcx.dispatch_action(SelectNext);
    assert!(view.read_with(vcx, |v, _| v.cursor_section().is_some()));
    vcx.dispatch_action(SelectNext);
    let ring = vcx.update(|w, cx| view.read(cx).ring_row(w).cloned());
    assert!(ring.is_some(), "keyboard cursor shows the ring");
}

#[gpui::test]
fn only_visible_rows_render(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, 2000);
    vcx.run_until_parked();
    let n = view.read_with(vcx, |v, _| v.rendered_rows());
    assert!(n > 0 && n < 100, "rendered {n}");
}

#[test]
fn unreads_section_mentions_first() {
    let s = state(vec![
        conv("C1", ConversationKind::Channel, "alpha"),
        unread(
            conv("C2", ConversationKind::Channel, "old"),
            0,
            "100.000001",
        ),
        unread(
            conv("C3", ConversationKind::Channel, "new"),
            0,
            "200.000001",
        ),
        unread(
            conv("C4", ConversationKind::Channel, "ping"),
            2,
            "50.000001",
        ),
    ]);
    let l = labels(&rows_for(&s, None));
    assert_eq!(
        l,
        [
            "#Unreads",
            "ping",
            "new",
            "old",
            "#Channels",
            "alpha",
            "new",
            "old",
            "ping"
        ]
    );
}

#[test]
fn unreads_section_respects_setting() {
    let s = state(vec![unread(
        conv("C1", ConversationKind::Channel, "a"),
        0,
        "1.000001",
    )]);
    let off = SidebarSettings {
        show_unreads_section: false,
        ..Default::default()
    };
    let l = labels(&build_rows(&s, None, &off, &HashSet::new(), &common::en()));
    assert_eq!(l, ["#Channels", "a"]);
}

#[test]
fn muted_rows_dimmed_not_in_unreads() {
    let mut c = unread(
        conv("C1", ConversationKind::Channel, "quiet"),
        0,
        "1.000001",
    );
    c.is_muted = true;
    let rows = rows_for(&state(vec![c]), None);
    assert_eq!(labels(&rows), ["#Channels", "quiet"]);
    assert_eq!(flags(&rows, "quiet"), [(true, false)]);
}

#[test]
fn draft_marker_shown() {
    let s = state(vec![
        conv("C1", ConversationKind::Channel, "a"),
        conv("C2", ConversationKind::Channel, "b"),
    ]);
    let drafts: HashSet<ConversationId> = [ConversationId(Arc::from("C2"))].into();
    let rows = build_rows(
        &s,
        None,
        &SidebarSettings::default(),
        &drafts,
        &common::en(),
    );
    assert_eq!(flags(&rows, "a"), [(false, false)]);
    assert_eq!(flags(&rows, "b"), [(false, true)]);
}

#[test]
fn recent_sort_orders_by_latest() {
    let mut a = conv("C1", ConversationKind::Channel, "alpha");
    a.latest = Some(MessageTs(Arc::from("1.000001")));
    let mut z = conv("C2", ConversationKind::Channel, "zeta");
    z.latest = Some(MessageTs(Arc::from("9.000001")));
    let s = state(vec![a, z]);
    let recent = SidebarSettings {
        sort: SidebarSort::Recent,
        ..Default::default()
    };
    let l = labels(&build_rows(
        &s,
        None,
        &recent,
        &HashSet::new(),
        &common::en(),
    ));
    assert_eq!(l, ["#Channels", "zeta", "alpha"]);
}

#[test]
fn next_unread_picks_after_selected_and_wraps() {
    let s = state(vec![
        unread(conv("C1", ConversationKind::Channel, "a"), 0, "1.000001"),
        conv("C2", ConversationKind::Channel, "b"),
        unread(conv("C3", ConversationKind::Channel, "c"), 0, "3.000001"),
    ]);
    let id = |x: &str| ConversationId(Arc::from(x));
    let off = SidebarSettings {
        show_unreads_section: false,
        ..Default::default()
    };
    let rows = build_rows(&s, None, &off, &HashSet::new(), &common::en());
    assert_eq!(next_unread(&rows, Some(&id("C1"))), Some(id("C3")));
    assert_eq!(next_unread(&rows, Some(&id("C3"))), Some(id("C1")));
    assert_eq!(next_unread(&rows, Some(&id("C2"))), Some(id("C3")));
    assert_eq!(next_unread(&rows, None), Some(id("C1")));
}

#[gpui::test]
fn next_unread_wraps(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, 3);
    let mut s = state(vec![
        unread(conv("C1", ConversationKind::Channel, "a"), 0, "1.000001"),
        conv("C2", ConversationKind::Channel, "b"),
        unread(conv("C3", ConversationKind::Channel, "c"), 0, "3.000001"),
    ]);
    s.conversations.remove(&ConversationId(Arc::from("C00000")));
    let sel = ConversationId(Arc::from("C3"));
    src.update(vcx, |src, cx| {
        src.rows = rows_for(&s, Some(&sel));
        cx.notify();
    });
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    vcx.dispatch_action(NextUnread);
    let opened = src.read_with(vcx, |s, _| s.opened.clone());
    assert_eq!(opened, vec![ConversationId(Arc::from("C1"))]);
}

#[test]
fn sidebar_sections_in_catalan() {
    let ca = common::localizer("ca");
    let mut chan = conv("C1", ConversationKind::Channel, "general");
    chan.unread = true;
    chan.latest = Some(MessageTs(Arc::from("1.0")));
    let dm = conv("D1", ConversationKind::Group, "team");
    let s = state(vec![chan, dm]);
    let rows = build_rows(&s, None, &SidebarSettings::default(), &HashSet::new(), &ca);
    let headers: Vec<String> = labels(&rows)
        .into_iter()
        .filter(|l| l.starts_with('#'))
        .collect();
    assert_eq!(headers, ["#Sense llegir", "#Canals", "#Missatges directes"]);
}

fn member_with_handle(id: &str, name: &str, handle: &str) -> Member {
    let uid = UserId(Arc::from(id));
    Member {
        id: uid,
        display_name: Arc::from(name),
        real_name: Arc::from(name),
        handle: Arc::from(handle),
        is_bot: false,
        deleted: false,
        status: None,
        title: None,
        tz: None,
        avatar: None,
        ext: ProviderExt::none(),
    }
}

fn group(id: &str, handles: &[&str]) -> Conversation {
    let mut g = conv(id, ConversationKind::Group, "mpdm-raw-name-1");
    g.participants = handles.iter().map(|h| Arc::<str>::from(*h)).collect();
    g
}

fn group_label(s: &WorkspaceState) -> String {
    let rows = rows_for(s, None);
    labels(&rows)
        .into_iter()
        .find(|l| !l.starts_with('#'))
        .unwrap()
}

#[test]
fn group_shows_member_display_names_without_self() {
    let mut s = state(vec![group("G1", &["amy", "bob", "me"])]);
    for (id, name, handle) in [
        ("U1", "Amy A", "amy"),
        ("U2", "Bob B", "bob"),
        ("U3", "Me", "me"),
    ] {
        let m = member_with_handle(id, name, handle);
        s.members.insert(m.id.clone(), m);
    }
    s.self_user = Some(UserId(Arc::from("U3")));
    assert_eq!(group_label(&s), "Amy A, Bob B");
}

#[test]
fn group_falls_back_to_handles_until_members_arrive() {
    let mut s = state(vec![group("G1", &["amy", "bob"])]);
    assert_eq!(group_label(&s), "amy, bob");
    let m = member_with_handle("U1", "Amy A", "amy");
    s.members.insert(m.id.clone(), m);
    assert_eq!(group_label(&s), "Amy A, bob");
}

#[test]
fn group_without_participants_keeps_its_name() {
    let s = state(vec![conv("G1", ConversationKind::Group, "team chat")]);
    assert_eq!(group_label(&s), "team chat");
}

fn big_source(cx: &mut TestAppContext, selected: Option<&str>) -> Entity<Src> {
    let s = state(
        (0..2000)
            .map(|i| {
                conv(
                    &format!("C{i:05}"),
                    ConversationKind::Channel,
                    &format!("chan{i:05}"),
                )
            })
            .collect(),
    );
    let sel = selected.map(|x| ConversationId(Arc::from(x)));
    cx.new(|_| Src {
        rows: rows_for(&s, sel.as_ref()),
        opened: vec![],
        calls: Default::default(),
        toggles: 0,
        workspace_name: None,
    })
}

#[gpui::test]
fn opens_at_the_top_with_a_header_when_rows_arrive_later(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let full = big_source(cx, None).read_with(cx, |s, _| s.rows.clone());
    let src = cx.new(|_| Src {
        rows: vec![],
        opened: vec![],
        calls: Default::default(),
        toggles: 0,
        workspace_name: None,
    });
    let src2 = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| Sidebar::new(src2, cx));
    vcx.run_until_parked();
    src.update(vcx, |s, cx| {
        s.rows = full;
        cx.notify();
    });
    vcx.run_until_parked();
    vcx.update(|w, _| w.refresh());
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.first_rendered_row()), 0);
    assert!(
        vcx.debug_bounds("sidebar-header-0").is_some()
            || view.read_with(vcx, |v, _| v.rendered_rows()) > 0
    );
}

#[gpui::test]
fn opens_at_the_selected_conversation(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let src = big_source(cx, Some("C01500"));
    let src2 = src;
    let (view, vcx) = cx.add_window_view(move |_w, cx| Sidebar::new(src2, cx));
    vcx.run_until_parked();
    vcx.update(|w, _| w.refresh());
    vcx.run_until_parked();
    let first = view.read_with(vcx, |v, _| v.first_rendered_row());
    assert!(first > 1000, "first rendered row {first}");
}

#[gpui::test]
fn long_names_stay_inside_the_row(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let s = state(vec![conv(
        "C1",
        ConversationKind::Channel,
        &"very-long-channel-name-".repeat(20),
    )]);
    let src = cx.new(|_| Src {
        rows: rows_for(&s, None),
        opened: vec![],
        calls: Default::default(),
        toggles: 0,
        workspace_name: None,
    });
    let (_view, vcx) = cx.add_window_view(move |_w, cx| Sidebar::new(src, cx));
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    let window = vcx.update(|w, _| w.bounds());
    let label = vcx.debug_bounds("sidebar-label-1").expect("label painted");
    assert!(
        label.right() <= window.size.width,
        "label {label:?} overflows {window:?}"
    );
    assert!(label.size.height < gpui::px(40.0), "one line only");
}

#[gpui::test]
fn idle_sidebar_does_not_render_again_after_a_scroll(cx: &mut TestAppContext) {
    use gpui::{Modifiers, ScrollDelta, ScrollEvent, ScrollSource, TouchPhase, point, px};
    let (src, view, vcx) = setup(cx, 2000);
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    for _ in 0..20 {
        vcx.simulate_event(ScrollEvent {
            position: point(px(50.0), px(100.0)),
            delta: ScrollDelta::Pixels(point(px(0.0), px(-40.0))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
            source: ScrollSource::Pointer,
        });
        vcx.update(|w, cx| w.draw(cx).clear(cx));
    }
    vcx.run_until_parked();
    let (renders, calls) = (
        view.read_with(vcx, |v, _| v.renders()),
        src.read_with(vcx, |s, _| s.calls.get()),
    );
    // Idle: nothing re-renders and nothing rebuilds the rows.
    for _ in 0..10 {
        vcx.run_until_parked();
    }
    assert_eq!(view.read_with(vcx, |v, _| v.renders()), renders);
    assert_eq!(src.read_with(vcx, |s, _| s.calls.get()), calls);
}

#[gpui::test]
fn unrelated_session_notification_does_not_repaint_the_sidebar(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, 50);
    vcx.run_until_parked();
    let before = view.read_with(vcx, |v, _| v.renders());
    for _ in 0..10 {
        src.update(vcx, |_, cx| cx.notify());
        vcx.run_until_parked();
    }
    assert_eq!(view.read_with(vcx, |v, _| v.renders()), before);
}

/// 500 conversations (a tenth are group DMs of 3 to 6 people) and 10,000 members.
fn big_account() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    for i in 0..10_000 {
        let id = UserId(Arc::from(format!("U{i:05}")));
        s.members.insert(
            id.clone(),
            Member {
                id,
                display_name: Arc::from(format!("Member {i}")),
                real_name: Arc::from(format!("Member {i}")),
                handle: Arc::from(format!("member.{i}")),
                is_bot: false,
                deleted: false,
                status: None,
                title: None,
                tz: None,
                avatar: None,
                ext: ProviderExt::none(),
            },
        );
    }
    for i in 0..500 {
        let kind = match i % 10 {
            0..=6 => ConversationKind::Channel,
            7 | 8 => ConversationKind::Direct,
            _ => ConversationKind::Group,
        };
        let mut c = conv(&format!("C{i:05}"), kind, &format!("chan{i}"));
        if kind == ConversationKind::Group {
            c.participants = (0..3 + i % 4)
                .map(|k| Arc::<str>::from(format!("member.{}", (i * 131 + k * 977) % 10_000)))
                .collect();
        }
        c.unread = i % 3 == 0;
        s.conversations.insert(c.id.clone(), c);
    }
    s
}

#[test]
#[ignore = "timing-sensitive; run with `mise run test:perf`"]
fn build_rows_with_a_member_index_is_fast_on_a_large_account() {
    let s = big_account();
    let l = common::en();
    let index = components::MemberIndex::build(&s);
    let (settings, drafts) = (SidebarSettings::default(), HashSet::new());
    let mut times = Vec::new();
    for _ in 0..21 {
        let t = std::time::Instant::now();
        let rows = components::build_rows_indexed(&s, &index, None, &settings, &drafts, &l);
        times.push(t.elapsed());
        assert!(rows.len() > 400);
    }
    times.sort();
    let median = times[10];
    // Budget 2 ms in release; debug test builds are an order of magnitude slower.
    let budget = std::time::Duration::from_millis(if cfg!(debug_assertions) { 40 } else { 2 });
    assert!(median < budget, "median {median:?} over {budget:?}");
}

#[test]
fn group_labels_use_the_member_index() {
    let s = big_account();
    let rows = rows_for(&s, None);
    let group = rows
        .iter()
        .find_map(|r| match r {
            SidebarRow::Conversation {
                kind: ConversationKind::Group,
                label,
                ..
            } => Some(label),
            _ => None,
        })
        .expect("a group row");
    assert!(group.starts_with("Member "), "{group}");
}

#[test]
fn member_index_patches_one_member_without_a_rebuild() {
    let s = big_account();
    let mut index = components::MemberIndex::build(&s);
    let old = s
        .members
        .values()
        .find(|m| &*m.handle == "member.5")
        .unwrap();
    // Same name and handle (a presence or profile update): nothing to redo.
    assert!(!index.update(Some(old), &old.clone(), false));
    let mut renamed = old.clone();
    renamed.display_name = Arc::from("Renamed");
    assert!(index.update(Some(old), &renamed, false));
    let mut g = conv("CG", ConversationKind::Group, "g");
    g.participants = Arc::from([Arc::<str>::from("member.5")]);
    g.latest = Some(MessageTs(Arc::from("9999999999.000001")));
    let mut s2 = s.clone();
    s2.conversations.insert(g.id.clone(), g);
    let rows = components::build_rows_indexed(
        &s2,
        &index,
        None,
        &SidebarSettings::default(),
        &HashSet::new(),
        &common::en(),
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r, SidebarRow::Conversation { label, .. } if &**label == "Renamed"))
    );
}

#[test]
fn pinned_unread_stays_in_unreads_while_selected() {
    let mut a = unread(conv("C1", ConversationKind::Channel, "a"), 0, "1.000001");
    let b = unread(conv("C2", ConversationKind::Channel, "b"), 0, "1.000002");
    // "a" was opened and is now read: without the pin it would leave "Unreads".
    a.unread = false;
    let s = state(vec![a, b]);
    let id = ConversationId(Arc::from("C1"));
    let rows = |pin: Option<&ConversationId>, sel: Option<&ConversationId>| {
        build_rows_pinned(
            &s,
            &MemberIndex::build(&s),
            sel,
            pin,
            &SidebarSettings::default(),
            &HashSet::new(),
            &common::en(),
        )
    };
    assert_eq!(
        labels(&rows(None, Some(&id))),
        ["#Unreads", "b", "#Channels", "a", "b"]
    );
    let pinned = rows(Some(&id), Some(&id));
    assert_eq!(
        labels(&pinned),
        ["#Unreads", "b", "a", "#Channels", "a", "b"]
    );
    let sel: Vec<_> = pinned
        .iter()
        .filter_map(|r| match r {
            SidebarRow::Conversation {
                label,
                selected: true,
                ..
            } => Some(label.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(sel, ["a", "a"]);
    // Once another conversation is selected the pin lapses.
    let other = ConversationId(Arc::from("C2"));
    assert_eq!(
        labels(&rows(Some(&id), Some(&other))),
        ["#Unreads", "b", "#Channels", "a", "b"]
    );
}

fn person(id: &str, handle: &str, display: &str, real: &str) -> Member {
    Member {
        id: UserId(Arc::from(id)),
        display_name: Arc::from(display),
        real_name: Arc::from(real),
        handle: Arc::from(handle),
        is_bot: false,
        deleted: false,
        status: None,
        title: None,
        tz: None,
        avatar: None,
        ext: ProviderExt::none(),
    }
}

fn dm_with(id: &str, peer: &str, name: &str, open: bool) -> Conversation {
    let mut c = conv(id, ConversationKind::Direct, name);
    c.peer = Some(UserId(Arc::from(peer)));
    c.is_open = open;
    c
}

#[test]
fn closed_direct_messages_are_hidden_unless_on_screen() {
    let mut s = state(vec![
        conv("C1", ConversationKind::Channel, "general"),
        dm_with("D1", "U1", "", true),
        dm_with("D2", "U2", "", false),
    ]);
    for (u, n) in [("U1", "Open Person"), ("U2", "Closed Person")] {
        let m = person(u, "", n, n);
        s.members.insert(m.id.clone(), m);
    }
    assert_eq!(
        labels(&rows_for(&s, None)),
        ["#Channels", "general", "#Direct messages", "Open Person"]
    );
    let sel = ConversationId(Arc::from("D2"));
    assert!(labels(&rows_for(&s, Some(&sel))).contains(&"Closed Person".to_string()));
}

#[test]
fn unresolved_direct_message_peer_reads_unknown_never_the_raw_id() {
    // The conversation name carries the raw id (an older cache did).
    let s = state(vec![dm_with("D1", "U0111Q6G5B4", "U0111Q6G5B4", true)]);
    assert_eq!(labels(&rows_for(&s, None)), ["#Direct messages", "Unknown"]);
}

#[test]
fn people_are_named_display_then_real_then_handle_in_dms_and_groups() {
    let mut g = conv("G1", ConversationKind::Group, "mpdm-a--b--c--me-1");
    g.participants = Arc::from([
        Arc::<str>::from("a.display"),
        Arc::from("b.real"),
        Arc::from("c.only"),
        Arc::from("me.self"),
    ]);
    let mut s = state(vec![g, dm_with("D1", "U2", "", true)]);
    for m in [
        person("U1", "a.display", "Ann Display", "Ann Real"),
        person("U2", "b.real", "", "Bea Real"),
        person("U3", "c.only", "", ""),
        person("U9", "me.self", "Me", "Me"),
    ] {
        s.members.insert(m.id.clone(), m);
    }
    s.self_user = Some(UserId(Arc::from("U9")));
    let l = labels(&rows_for(&s, None));
    assert!(l.contains(&"Bea Real".to_string()), "{l:?}");
    assert!(
        l.contains(&"Ann Display, Bea Real, c.only".to_string()),
        "{l:?}"
    );
}

#[test]
fn group_members_are_named_by_id_even_without_a_handle() {
    let mut g = group("G1", &["amy", "bob", "me"]);
    g.member_ids = ["U1", "U2", "U3"]
        .iter()
        .map(|u| UserId(Arc::from(*u)))
        .collect();
    let mut s = state(vec![g]);
    // Bob's handle is empty and Amy's differs from the one in the group name.
    for (id, name, handle) in [
        ("U1", "Amy A", "amy.a"),
        ("U2", "Bob B", ""),
        ("U3", "Me", ""),
    ] {
        let m = member_with_handle(id, name, handle);
        s.members.insert(m.id.clone(), m);
    }
    s.self_user = Some(UserId(Arc::from("U3")));
    assert_eq!(group_label(&s), "Amy A, Bob B");
}

/// `n` DMs "dm00".."dm{n-1}", the higher the number the newer.
fn many_dms(n: usize) -> Vec<Conversation> {
    (0..n)
        .map(|i| {
            let mut c = conv(
                &format!("D{i:03}"),
                ConversationKind::Group,
                &format!("dm{i:02}"),
            );
            c.latest = Some(MessageTs(Arc::from(format!("{}.000001", 1_000 + i))));
            c
        })
        .collect()
}

fn dm_rows(s: &WorkspaceState, sel: Option<&ConversationId>, expanded: bool) -> Vec<SidebarRow> {
    components::build_rows_with(
        s,
        &MemberIndex::build(s),
        sel,
        None,
        &SidebarSettings::default(),
        &HashSet::new(),
        &common::en(),
        expanded,
    )
}

#[test]
fn direct_messages_are_newest_first_and_capped_with_show_more() {
    let s = state(many_dms(30));
    let l = labels(&dm_rows(&s, None, false));
    assert_eq!(l.len(), 1 + domain::DIRECT_MESSAGES_SHOWN + 1);
    assert_eq!(l[0], "#Direct messages");
    assert_eq!(l[1], "dm29");
    assert_eq!(l[domain::DIRECT_MESSAGES_SHOWN], "dm10");
    assert_eq!(l.last().unwrap(), "+Show more (10)");
}

#[test]
fn unread_and_selected_direct_messages_stay_visible_when_collapsed() {
    let mut cs = many_dms(30);
    cs[0].unread = true;
    cs[1].mentions = 2;
    cs[1].unread = true;
    let s = state(cs);
    let sel = ConversationId(Arc::from("D002"));
    let l = labels(&dm_rows(&s, Some(&sel), false));
    for name in ["dm00", "dm01", "dm02"] {
        assert!(l.iter().any(|x| x == name), "{name} missing: {l:?}");
    }
    // Order stays newest first, and the hidden count excludes the kept ones.
    assert_eq!(l.last().unwrap(), "+Show more (7)");
    let dm_at = l.iter().position(|x| x == "#Direct messages").unwrap();
    let pos = |n: &str| l[dm_at..].iter().position(|x| x == n).unwrap();
    assert!(pos("dm10") < pos("dm02") && pos("dm02") < pos("dm01"));
}

#[test]
fn expanding_lists_every_direct_message_with_show_fewer() {
    let s = state(many_dms(30));
    let l = labels(&dm_rows(&s, None, true));
    assert_eq!(l.len(), 1 + 30 + 1);
    assert_eq!(l.last().unwrap(), "+Show fewer");
}

#[test]
fn a_short_direct_message_list_has_no_show_more_row() {
    let s = state(many_dms(domain::DIRECT_MESSAGES_SHOWN));
    for expanded in [false, true] {
        let l = labels(&dm_rows(&s, None, expanded));
        assert_eq!(l.len(), 1 + domain::DIRECT_MESSAGES_SHOWN);
    }
}

#[test]
fn direct_messages_without_activity_sort_after_active_ones() {
    let mut cs = many_dms(3);
    cs[2].latest = None;
    let s = state(cs);
    let l = labels(&dm_rows(&s, None, false));
    assert_eq!(l, ["#Direct messages", "dm01", "dm00", "dm02"]);
}

#[test]
fn show_more_row_is_localized() {
    let s = state(many_dms(30));
    let ca = common::localizer("ca");
    let rows = components::build_rows_with(
        &s,
        &MemberIndex::build(&s),
        None,
        None,
        &SidebarSettings::default(),
        &HashSet::new(),
        &ca,
        false,
    );
    let SidebarRow::ShowMore { label, .. } = rows.last().unwrap() else {
        panic!("no show more row")
    };
    assert_eq!(&**label, "Mostra'n més (10)");
}

#[gpui::test]
fn clicking_show_more_asks_the_source_to_toggle(cx: &mut TestAppContext) {
    cx.update(themes::init_default);
    cx.update(|cx| {
        i18n::gpui::init(cx, &[components::catalog()], i18n::LocalePref::System);
    });
    let s = state(many_dms(30));
    let src = cx.new(|_| Src {
        rows: dm_rows(&s, None, false),
        opened: vec![],
        calls: Default::default(),
        toggles: 0,
        workspace_name: None,
    });
    let src2 = src.clone();
    let (_view, vcx) = cx.add_window_view(move |_w, cx| Sidebar::new(src2, cx));
    vcx.run_until_parked();
    let b = vcx.debug_bounds("sidebar-show-more").expect("row painted");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(src.read_with(vcx, |s, _| s.toggles), 1);
}

#[test]
fn direct_rows_carry_the_peers_presence() {
    let mut a = conv("D1", ConversationKind::Direct, "");
    a.peer = Some(UserId(Arc::from("U1")));
    let mut b = conv("D2", ConversationKind::Direct, "");
    b.peer = Some(UserId(Arc::from("U2")));
    let mut s = state(vec![a, b, conv("C1", ConversationKind::Channel, "general")]);
    s.presence.insert(UserId(Arc::from("U1")), true);
    s.presence.insert(UserId(Arc::from("U2")), false);
    let presence = |rows: &[SidebarRow], id: &str| {
        rows.iter().find_map(|r| match r {
            SidebarRow::Conversation {
                id: i, presence, ..
            } if &*i.0 == id => Some(*presence),
            _ => None,
        })
    };
    let rows = rows_for(&s, None);
    assert_eq!(presence(&rows, "D1"), Some(Some(true)));
    assert_eq!(presence(&rows, "D2"), Some(Some(false)));
    // Channels have none; an unknown peer shows no dot.
    assert_eq!(presence(&rows, "C1"), Some(None));
    s.presence.clear();
    assert_eq!(presence(&rows_for(&s, None), "D1"), Some(None));
}

#[test]
fn direct_rows_reserve_the_dot_only_when_the_provider_reports_presence() {
    let mut a = conv("D1", ConversationKind::Direct, "");
    a.peer = Some(UserId(Arc::from("U1")));
    let mut s = state(vec![a, conv("C1", ConversationKind::Channel, "general")]);
    let slot = |s: &WorkspaceState, id: &str| {
        rows_for(s, None).iter().find_map(|r| match r {
            SidebarRow::Conversation {
                id: i, dot_slot, ..
            } if &*i.0 == id => Some(*dot_slot),
            _ => None,
        })
    };
    assert_eq!(slot(&s, "D1"), Some(false));
    s.caps.presence_watch = true;
    assert_eq!(slot(&s, "D1"), Some(true));
    assert_eq!(slot(&s, "C1"), Some(false));
}

#[gpui::test]
fn workspace_header_shows_name(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let s = state(vec![conv("C1", ConversationKind::Channel, "general")]);
    let src = cx.new(|_| Src {
        rows: rows_for(&s, None),
        opened: vec![],
        calls: Default::default(),
        toggles: 0,
        workspace_name: Some("Example Workspace".to_string()),
    });
    let (_view, vcx) = cx.add_window_view(move |_w, cx| Sidebar::new(src, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(
        vcx.debug_bounds("sidebar-workspace-name").is_some(),
        "workspace name header not rendered"
    );
}

#[gpui::test]
fn clicking_the_search_field_asks_for_the_quick_switcher(cx: &mut TestAppContext) {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (_src, view, vcx) = setup(cx, 3);
    let log = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &components::SidebarEvent, _| {
            log.borrow_mut()
                .push(matches!(ev, components::SidebarEvent::QuickSwitch));
        })
        .detach();
    });
    vcx.run_until_parked();
    let b = vcx
        .debug_bounds("sidebar-search")
        .expect("search field painted");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert_eq!(*events.borrow(), vec![true]);
}

#[test]
fn starred_section_comes_first_and_leaves_its_normal_section() {
    let mut s = state(vec![
        conv("C1", ConversationKind::Channel, "general"),
        conv("C2", ConversationKind::Channel, "design"),
        conv("D1", ConversationKind::Group, "team"),
    ]);
    s.starred.insert(ConversationId(Arc::from("C2")));
    s.starred.insert(ConversationId(Arc::from("D1")));
    let rows = rows_for(&s, None);
    assert_eq!(
        labels(&rows),
        ["#Starred", "design", "team", "#Channels", "general"]
    );
}

#[test]
fn starred_section_is_hidden_when_empty_and_skips_archived() {
    let mut gone = conv("C2", ConversationKind::Channel, "old");
    gone.is_archived = true;
    let mut s = state(vec![conv("C1", ConversationKind::Channel, "general"), gone]);
    s.starred.insert(ConversationId(Arc::from("C2")));
    s.starred.insert(ConversationId(Arc::from("missing")));
    assert_eq!(labels(&rows_for(&s, None)), ["#Channels", "general"]);
}

#[test]
fn starred_rows_sort_by_name_and_stay_in_unreads() {
    let mut s = state(vec![
        unread(conv("C1", ConversationKind::Channel, "zeta"), 0, "2.0"),
        conv("C2", ConversationKind::Channel, "alpha"),
    ]);
    s.starred.insert(ConversationId(Arc::from("C1")));
    s.starred.insert(ConversationId(Arc::from("C2")));
    assert_eq!(
        labels(&rows_for(&s, None)),
        ["#Starred", "alpha", "zeta", "#Unreads", "zeta"]
    );
}

#[test]
fn starred_header_in_catalan() {
    let mut s = state(vec![conv("C1", ConversationKind::Channel, "general")]);
    s.starred.insert(ConversationId(Arc::from("C1")));
    let rows = build_rows(
        &s,
        None,
        &SidebarSettings::default(),
        &HashSet::new(),
        &common::localizer("ca"),
    );
    assert_eq!(labels(&rows)[0], "#Destacats");
}

/// A conversation listed twice (Starred and Unreads) is two stops for the
/// arrow keys; the cursor is a row, not an id.
#[gpui::test]
fn arrow_keys_pass_through_both_copies_of_a_conversation(cx: &mut TestAppContext) {
    let mut s = state(vec![
        unread(conv("C1", ConversationKind::Channel, "zeta"), 0, "2.0"),
        conv("C2", ConversationKind::Channel, "alpha"),
    ]);
    s.starred.insert(ConversationId(Arc::from("C1")));
    let rows = rows_for(&s, None);
    assert_eq!(
        labels(&rows),
        ["#Starred", "zeta", "#Unreads", "zeta", "#Channels", "alpha"]
    );
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let src = cx.new(|_| Src {
        rows,
        opened: vec![],
        calls: Default::default(),
        toggles: 0,
        workspace_name: None,
    });
    let (view, vcx) = cx.add_window_view(move |_w, cx| Sidebar::new(src, cx));
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    // Headers are stops too: Starred, zeta, Unreads, zeta, Channels, alpha.
    for _ in 0..6 {
        vcx.dispatch_action(SelectNext);
    }
    let ring = vcx.update(|w, cx| view.read(cx).ring_row(w).cloned());
    assert_eq!(ring, Some(ConversationId(Arc::from("C2"))));
    vcx.dispatch_action(SelectPrev);
    vcx.dispatch_action(SelectPrev);
    let ring = vcx.update(|w, cx| view.read(cx).ring_row(w).cloned());
    assert_eq!(ring, Some(ConversationId(Arc::from("C1"))));
}

#[test]
fn starred_hides_closed_direct_messages_like_the_dm_section() {
    let mut closed = conv("D1", ConversationKind::Group, "closed");
    closed.is_open = false;
    let mut s = state(vec![
        closed,
        conv("C1", ConversationKind::Channel, "general"),
    ]);
    s.starred.insert(ConversationId(Arc::from("D1")));
    assert_eq!(labels(&rows_for(&s, None)), ["#Channels", "general"]);
    // The one on screen stays.
    let id = ConversationId(Arc::from("D1"));
    assert_eq!(
        labels(&rows_for(&s, Some(&id))),
        ["#Starred", "closed", "#Channels", "general"]
    );
}

fn custom(id: u32, name: &str, convs: &[&str]) -> settings::CustomSection {
    settings::CustomSection {
        id,
        name: name.into(),
        conversations: convs.iter().map(|c| c.to_string()).collect(),
    }
}

fn with_sections(sections: Vec<settings::CustomSection>) -> SidebarSettings {
    SidebarSettings {
        sections,
        ..SidebarSettings::default()
    }
}

#[test]
fn custom_sections_sit_above_channels_and_take_their_conversations() {
    let s = state(vec![
        conv("C1", ConversationKind::Channel, "general"),
        conv("C2", ConversationKind::Channel, "design"),
        conv("C3", ConversationKind::Channel, "random"),
        conv("D1", ConversationKind::Group, "team"),
    ]);
    let settings = with_sections(vec![
        custom(1, "Focus", &["C2", "D1", "missing"]),
        // An empty section still shows its header, to drop rows into.
        custom(2, "Clients", &[]),
    ]);
    let rows = build_rows(&s, None, &settings, &HashSet::new(), &common::en());
    assert_eq!(
        labels(&rows),
        [
            "#Focus",
            "design",
            "team",
            "#Clients",
            "#Channels",
            "general",
            "random"
        ]
    );
    assert!(matches!(
        rows[0],
        SidebarRow::Header {
            section: components::SidebarSection::Custom(1),
            ..
        }
    ));
}

#[test]
fn starred_wins_over_a_custom_section_and_first_section_wins_a_duplicate() {
    let mut s = state(vec![
        conv("C1", ConversationKind::Channel, "general"),
        conv("C2", ConversationKind::Channel, "design"),
    ]);
    s.starred.insert(ConversationId(Arc::from("C1")));
    let settings = with_sections(vec![custom(1, "A", &["C1", "C2"]), custom(2, "B", &["C2"])]);
    let rows = build_rows(&s, None, &settings, &HashSet::new(), &common::en());
    assert_eq!(labels(&rows), ["#Starred", "general", "#A", "design", "#B"]);
}

#[test]
fn a_folded_custom_section_keeps_unread_rows() {
    let s = state(vec![
        conv("C1", ConversationKind::Channel, "general"),
        unread(conv("C2", ConversationKind::Channel, "design"), 0, "5.0"),
    ]);
    let settings = with_sections(vec![custom(4, "Focus", &["C1", "C2"])]);
    let mut settings_off = settings.clone();
    settings_off.show_unreads_section = false;
    let folded: std::collections::BTreeSet<_> = [components::SidebarSection::Custom(4)]
        .into_iter()
        .collect();
    let rows = components::build_rows_sections(
        &s,
        &MemberIndex::build(&s),
        None,
        None,
        &settings_off,
        &HashSet::new(),
        &common::en(),
        false,
        &folded,
    );
    assert_eq!(labels(&rows), ["#Focus", "design"]);
    assert!(matches!(
        rows[0],
        SidebarRow::Header {
            collapsed: true,
            ..
        }
    ));
}

#[test]
fn a_previewed_channel_is_not_a_sidebar_row() {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.conversations.insert(
        ConversationId(Arc::from("c1")),
        conv("c1", ConversationKind::Channel, "general"),
    );
    let found = conv("c9", ConversationKind::Channel, "design-crit");
    domain::reduce(&mut s, domain::Input::Preview(found));
    let c9 = ConversationId(Arc::from("c9"));
    let rows = build_rows(
        &s,
        Some(&c9),
        &SidebarSettings::default(),
        &HashSet::new(),
        &common::en(),
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r, SidebarRow::Conversation { id, .. } if *id == c9))
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r, SidebarRow::Conversation { id, .. } if id.0.as_ref() == "c1"))
    );
}
