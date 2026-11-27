//! The workspace rail: grouping, initials, accents, switching targets, badges.

use std::sync::Arc;

use components::rail::{RailItem, flatten, key_at, key_step, parse_accent, rail_items};
use domain::{
    AccountInfo, Conversation, ConversationId, ConversationKind, OrgInfo, ProviderExt,
    WorkspaceState,
};
use settings::WorkspaceKey;

fn key(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

fn account(id: &str, name: &str, org: Option<&str>) -> AccountInfo {
    AccountInfo {
        key: key(id),
        name: Arc::from(name),
        org: org.map(|o| OrgInfo {
            id: Arc::from(o),
            name: Arc::from(format!("Org {o}")),
        }),
        icon: None,
    }
}

fn conv(id: &str, unread: bool, mentions: u32, muted: bool) -> Conversation {
    Conversation {
        id: ConversationId(Arc::from(id)),
        kind: ConversationKind::Channel,
        name: Arc::from(id),
        peer: None,
        is_member: true,
        is_archived: false,
        is_open: true,
        is_muted: muted,
        last_read: None,
        latest: None,
        unread,
        mentions,
        participants: Arc::from([]),
        member_ids: Arc::from([]),
        topic: Default::default(),
        member_count: None,
        purpose: Default::default(),
        ext: ProviderExt::none(),
    }
}

fn state(id: &str, cs: Vec<Conversation>) -> WorkspaceState {
    let mut s = WorkspaceState::new(key(id), 200);
    for c in cs {
        s.conversations.insert(c.id.clone(), c);
    }
    s
}

fn no_accent(_: &WorkspaceKey) -> Option<gpui::Hsla> {
    None
}

fn three() -> Vec<AccountInfo> {
    vec![
        account("a", "Alpha Team", Some("o1")),
        account("b", "Beta", None),
        account("c", "Gamma", Some("o1")),
    ]
}

#[test]
fn org_members_grouped_in_place() {
    let items = rail_items(&three(), &[], None, &no_accent, 0);
    assert_eq!(items.len(), 2);
    match &items[0] {
        RailItem::Org { name, children } => {
            assert_eq!(&**name, "Org o1");
            let keys: Vec<_> = children.iter().map(|c| c.key.clone()).collect();
            assert_eq!(keys, vec![key("a"), key("c")]);
        }
        other => panic!("expected an org first, got {other:?}"),
    }
    assert!(matches!(&items[1], RailItem::Workspace(w) if w.key == key("b")));
    let order: Vec<_> = flatten(&items).iter().map(|w| w.key.clone()).collect();
    assert_eq!(order, vec![key("a"), key("c"), key("b")]);
}

#[test]
fn initials_when_no_icon() {
    let items = rail_items(&three(), &[], None, &no_accent, 0);
    let flat = flatten(&items);
    assert_eq!(&*flat[0].initials, "AT");
    assert_eq!(&*flat[2].initials, "BE");
}

#[test]
fn accent_parses_hex_only() {
    assert!(parse_accent("#ff8800").is_some());
    assert!(parse_accent("#FF8800").is_some());
    assert_eq!(parse_accent("#ff8800"), parse_accent("#FF8800"));
    for bad in [
        "ff8800", "#ff88", "#ff88001", "#gg0000", "red", "", "#ff 800",
    ] {
        assert!(parse_accent(bad).is_none(), "{bad}");
    }
    let items = rail_items(
        &three(),
        &[],
        None,
        &|k| (k == &key("b")).then(|| parse_accent("#336699")).flatten(),
        0,
    );
    assert!(
        flatten(&items)
            .iter()
            .any(|w| w.key == key("b") && w.accent.is_some())
    );
    assert!(
        flatten(&items)
            .iter()
            .any(|w| w.key == key("a") && w.accent.is_none())
    );
}

#[test]
fn cmd_n_activates_nth() {
    let items = rail_items(&three(), &[], None, &no_accent, 0);
    // The flattened order: a, c, b.
    assert_eq!(key_at(&items, 0), Some(key("a")));
    assert_eq!(key_at(&items, 1), Some(key("c")));
    assert_eq!(key_at(&items, 2), Some(key("b")));
    assert_eq!(key_at(&items, 3), None);
}

#[test]
fn next_previous_wrap() {
    let a = key("a");
    let c = key("c");
    let b = key("b");
    let at = |active: &WorkspaceKey| rail_items(&three(), &[], Some(active), &no_accent, 0);
    assert_eq!(key_step(&at(&a), 1), Some(c.clone()));
    assert_eq!(key_step(&at(&b), 1), Some(a.clone()));
    assert_eq!(key_step(&at(&a), -1), Some(b.clone()));
    assert_eq!(key_step(&at(&c), -1), Some(a));
    // No active workspace: next is the first, previous the last.
    let none = rail_items(&three(), &[], None, &no_accent, 0);
    assert_eq!(key_step(&none, 1), Some(key("a")));
    assert_eq!(key_step(&none, -1), Some(b));
    assert_eq!(key_step(&[], 1), None);
}

#[test]
fn badges_follow_state() {
    let sa = state(
        "a",
        vec![conv("C1", true, 3, false), conv("C2", true, 0, false)],
    );
    let mut sb = state("b", vec![conv("C1", true, 0, true)]);
    sb.session = domain::Session::Expired;
    sb.lean = true;
    let states = [(key("a"), &sa), (key("b"), &sb)];
    let items = rail_items(&three(), &states, Some(&key("a")), &no_accent, 0);
    let flat = flatten(&items);
    let by = |k: &str| flat.iter().find(|w| w.key == key(k)).unwrap();
    assert_eq!(by("a").mentions, 3);
    assert!(by("a").unread && by("a").active);
    assert!(!by("a").expired && !by("a").lean);
    // Muted unread conversations show no dot.
    assert_eq!(by("b").mentions, 0);
    assert!(!by("b").unread && !by("b").active);
    assert!(by("b").expired && by("b").lean);
    // Not started yet: nothing to show.
    assert_eq!(by("c").mentions, 0);
    assert!(!by("c").unread && !by("c").expired);
}

#[test]
fn workspace_name_comes_from_state() {
    let mut sa = state("a", vec![]);
    sa.name = Arc::from("Renamed");
    let states = [(key("a"), &sa)];
    let items = rail_items(&three(), &states, None, &no_accent, 0);
    let flat = flatten(&items);
    let a = flat.iter().find(|w| w.key == key("a")).unwrap();
    assert_eq!(&*a.name, "Renamed");
}

struct RailHost {
    items: Vec<RailItem>,
}

impl gpui::Render for RailHost {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{InteractiveElement as _, ParentElement as _, Styled as _};
        let actions = components::rail::RailActions {
            scroll: Default::default(),
            activate: std::rc::Rc::new(|_, _, _| {}),
            context: std::rc::Rc::new(|_, _, _, _| {}),
            add: std::rc::Rc::new(|_, _| {}),
            aggregate: std::rc::Rc::new(|_, _, _| {}),
            aggregate_open: None,
            settings: None,
            me: None,
        };
        gpui::div()
            .id("host")
            .size_full()
            .children(components::rail::render_rail(&self.items, &actions, cx))
    }
}

/// `(unread bar, mention badge)` painted for one workspace.
fn markers(
    cx: &mut gpui::TestAppContext,
    mentions: u32,
    unread: bool,
    active: bool,
) -> (bool, bool) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let w = components::rail::RailWorkspace {
        key: key("a"),
        name: "Alpha".into(),
        icon: None,
        initials: "A".into(),
        mentions,
        unread,
        active,
        expired: false,
        lean: false,
        paused: false,
        accent: None,
    };
    let (_v, vcx) = cx.add_window_view(move |_, _| RailHost {
        items: vec![RailItem::Workspace(w)],
    });
    vcx.run_until_parked();
    (
        vcx.debug_bounds("rail-unread").is_some(),
        vcx.debug_bounds("rail-mentions").is_some(),
    )
}

#[gpui::test]
fn unread_bar_sits_beside_the_mention_badge(cx: &mut gpui::TestAppContext) {
    assert_eq!(markers(cx, 3, true, false), (true, true));
}

#[gpui::test]
fn no_unread_bar_on_the_active_tile(cx: &mut gpui::TestAppContext) {
    assert_eq!(markers(cx, 3, true, true), (false, true));
}

#[gpui::test]
fn unread_alone_shows_the_bar_only(cx: &mut gpui::TestAppContext) {
    assert_eq!(markers(cx, 0, true, false), (true, false));
}

fn many(n: usize) -> Vec<RailItem> {
    (0..n)
        .map(|i| {
            RailItem::Workspace(components::rail::RailWorkspace {
                key: key(&format!("w{i}")),
                name: format!("Workspace {i}").into(),
                icon: None,
                initials: "W".into(),
                mentions: 0,
                unread: false,
                active: i == 0,
                expired: false,
                lean: false,
                paused: false,
                accent: None,
            })
        })
        .collect()
}

/// The rail column as the main window lays it out: a fixed-height flex column.
struct ShortRail {
    items: Vec<RailItem>,
    full: bool,
    scroll: components::rail::RailScroll,
}

impl gpui::Render for ShortRail {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _};
        let actions = components::rail::RailActions {
            scroll: self.scroll.clone(),
            activate: std::rc::Rc::new(|_, _, _| {}),
            context: std::rc::Rc::new(|_, _, _, _| {}),
            add: std::rc::Rc::new(|_, _| {}),
            aggregate: std::rc::Rc::new(|_, _, _| {}),
            aggregate_open: None,
            settings: self
                .full
                .then(|| std::rc::Rc::new(|_: &mut gpui::Window, _: &mut gpui::App| {}) as _),
            me: self.full.then(|| {
                components::rail::RailSelf::new(
                    "Marta Vidal".into(),
                    Some(true),
                    std::rc::Rc::new(|_, _| {}),
                )
            }),
        };
        gpui::div()
            .flex()
            .flex_col()
            .items_center()
            .w(themes::metrics::LAYOUT_RAIL)
            .h_full()
            .pt(themes::metrics::SPACE_5)
            .pb(themes::metrics::SPACE_6)
            .gap(themes::metrics::SPACE_5)
            .children(components::rail::render_rail(&self.items, &actions, cx))
    }
}

#[gpui::test]
fn add_button_stays_reachable_with_many_workspaces(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_v, vcx) = cx.add_window_view(|_, _| ShortRail {
        items: many(20),
        full: false,
        scroll: Default::default(),
    });
    vcx.simulate_resize(gpui::size(gpui::px(68.0), gpui::px(400.0)));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let add = vcx.debug_bounds("rail-add").expect("add drawn");
    assert!(
        add.bottom() <= gpui::px(400.0),
        "rail-add bottom {:?} is below the 400px window",
        add.bottom()
    );
    let list = vcx.debug_bounds("rail-list").expect("list drawn");
    assert!(list.bottom() <= add.top(), "list overlaps the add button");
}

struct SelfRail {
    log: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
}

impl gpui::Render for SelfRail {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _};
        let (a, b) = (self.log.clone(), self.log.clone());
        let actions = components::rail::RailActions {
            scroll: Default::default(),
            activate: std::rc::Rc::new(|_, _, _| {}),
            context: std::rc::Rc::new(|_, _, _, _| {}),
            add: std::rc::Rc::new(|_, _| {}),
            aggregate: std::rc::Rc::new(|_, _, _| {}),
            aggregate_open: None,
            settings: Some(std::rc::Rc::new(move |_, _| {
                a.borrow_mut().push("settings")
            })),
            me: Some(components::rail::RailSelf::new(
                "Marta Vidal".into(),
                Some(true),
                std::rc::Rc::new(move |_, _| b.borrow_mut().push("me")),
            )),
        };
        gpui::div()
            .flex()
            .flex_col()
            .h(gpui::px(600.0))
            .children(components::rail::render_rail(&many(2), &actions, cx))
    }
}

/// Settings and the own avatar (with presence) sit at the bottom of the rail
/// and run their actions.
#[gpui::test]
fn settings_and_own_avatar_sit_at_the_bottom(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let l = log.clone();
    let (_v, vcx) = cx.add_window_view(move |_, _| SelfRail { log: l });
    vcx.simulate_resize(gpui::size(gpui::px(68.0), gpui::px(600.0)));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let add = vcx.debug_bounds("rail-add").expect("add drawn");
    let settings = vcx.debug_bounds("rail-settings").expect("settings drawn");
    let me = vcx.debug_bounds("rail-self").expect("own avatar drawn");
    assert!(vcx.debug_bounds("rail-self-presence-active").is_some());
    assert!(
        settings.top() > add.bottom() + gpui::px(100.0),
        "pushed down"
    );
    assert!(me.top() >= settings.bottom(), "avatar under settings");
    assert!(me.bottom() <= gpui::px(600.0));
    vcx.simulate_click(settings.center(), gpui::Modifiers::none());
    vcx.simulate_click(me.center(), gpui::Modifiers::none());
    assert_eq!(*log.borrow(), ["settings", "me"]);
}

/// Settings, the own avatar, add and the workspace tiles are tab stops that
/// Enter activates.
#[gpui::test]
fn rail_buttons_activate_from_the_keyboard(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let l = log.clone();
    let (_v, vcx) = cx.add_window_view(move |_, _| SelfRail { log: l });
    vcx.simulate_resize(gpui::size(gpui::px(68.0), gpui::px(600.0)));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    for _ in 0..8 {
        vcx.update(|w, cx| w.focus_next(cx));
        vcx.simulate_keystrokes("enter");
    }
    let got = log.borrow().clone();
    assert!(got.contains(&"settings") && got.contains(&"me"), "{got:?}");
}

const SELS: [&str; 8] = [
    "rail-workspace-w0",
    "rail-workspace-w1",
    "rail-workspace-w2",
    "rail-workspace-w3",
    "rail-workspace-w4",
    "rail-workspace-w5",
    "rail-workspace-w6",
    "rail-workspace-w7",
];

/// The main window's rail: settings and own avatar included, the list scrolls
/// between the aggregate entries and the pinned add, settings and avatar.
#[gpui::test]
fn list_scrolls_between_the_top_and_the_pinned_buttons(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut items = many(8);
    for (i, it) in items.iter_mut().enumerate() {
        if let RailItem::Workspace(w) = it {
            w.active = i == 5;
        }
    }
    let (v, vcx) = cx.add_window_view(move |_, _| ShortRail {
        items,
        full: true,
        scroll: Default::default(),
    });
    vcx.simulate_resize(gpui::size(gpui::px(68.0), gpui::px(420.0)));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    v.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let list = vcx.debug_bounds("rail-list").expect("list");
    let add = vcx.debug_bounds("rail-add").expect("add");
    let settings = vcx.debug_bounds("rail-settings").expect("settings");
    let me = vcx.debug_bounds("rail-self").expect("me");
    assert!(
        list.bottom() <= add.top(),
        "list {list:?} overlaps add {add:?}"
    );
    assert!(add.bottom() <= settings.top(), "add overlaps settings");
    assert!(settings.bottom() <= me.top(), "settings overlap avatar");
    assert!(me.bottom() <= gpui::px(420.0), "avatar below the window");
    // The selected tile is scrolled into the list's view, clear of the add button.
    let active = vcx.debug_bounds(SELS[5]).expect("active tile");
    assert!(
        active.top() >= list.top() && active.bottom() <= list.bottom(),
        "active tile {active:?} is not inside the list {list:?}"
    );
}

/// Selecting another tile moves no tile and keeps every tile's size.
#[gpui::test]
fn selecting_a_tile_moves_no_tile(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mk = |sel: usize| {
        let mut items = many(5);
        for (i, it) in items.iter_mut().enumerate() {
            if let RailItem::Workspace(w) = it {
                w.active = i == sel;
            }
        }
        items
    };
    let (v, vcx) = cx.add_window_view(|_, _| ShortRail {
        items: many(5),
        full: true,
        scroll: Default::default(),
    });
    vcx.simulate_resize(gpui::size(gpui::px(68.0), gpui::px(900.0)));
    let snap = |vcx: &mut gpui::VisualTestContext| {
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        (0..5)
            .map(|i| vcx.debug_bounds(SELS[i]).expect("tile"))
            .collect::<Vec<_>>()
    };
    v.update(vcx, |r, _| r.items = mk(1));
    let a = snap(vcx);
    v.update(vcx, |r, _| r.items = mk(3));
    let b = snap(vcx);
    assert_eq!(a, b, "tiles moved or resized when the selection changed");
}

/// The selection ring is a border that is always reserved (transparent when
/// not selected), so selecting changes neither a tile's size nor its offset.
#[gpui::test]
fn selection_ring_does_not_change_the_tile(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let one = |active: bool| {
        let mut items = many(1);
        if let RailItem::Workspace(w) = &mut items[0] {
            w.active = active;
        }
        items
    };
    let mut bounds = Vec::new();
    for active in [false, true] {
        let items = one(active);
        let (_v, vcx) = cx.add_window_view(move |_, _| ShortRail {
            items,
            full: false,
            scroll: Default::default(),
        });
        vcx.simulate_resize(gpui::size(gpui::px(68.0), gpui::px(400.0)));
        vcx.run_until_parked();
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        bounds.push((
            vcx.debug_bounds("rail-badge").expect("badge"),
            vcx.debug_bounds("rail-workspace-w0").expect("tile"),
        ));
    }
    assert_eq!(bounds[0], bounds[1], "the ring moved or resized the tile");
}

/// Tiles keep their size however many there are: the list scrolls, its rows
/// never shrink (they are `flex_none`), so height and pitch are equal.
#[gpui::test]
fn tiles_keep_equal_height_and_pitch_when_overflowing(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_v, vcx) = cx.add_window_view(|_, _| ShortRail {
        items: many(8),
        full: true,
        scroll: Default::default(),
    });
    vcx.simulate_resize(gpui::size(gpui::px(68.0), gpui::px(300.0)));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    let tiles: Vec<_> = SELS
        .iter()
        .map(|s| vcx.debug_bounds(s).expect("tile"))
        .collect();
    let list = vcx.debug_bounds("rail-list").expect("list");
    assert!(
        tiles.last().unwrap().bottom() > list.bottom(),
        "the tiles must overflow the list for this test"
    );
    for t in &tiles {
        assert_eq!(
            t.size.height, tiles[0].size.height,
            "unequal heights {tiles:?}"
        );
    }
    for w in tiles.windows(2) {
        assert_eq!(
            w[1].top() - w[0].top(),
            tiles[1].top() - tiles[0].top(),
            "unequal pitch {tiles:?}"
        );
    }
}
