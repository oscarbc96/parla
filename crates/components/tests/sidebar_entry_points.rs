//! Sidebar entry points: collapsible sections, the Drafts entry, New message,
//! the Channels and Direct messages "+" buttons and the workspace menu.

mod common;

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use components::{
    MemberIndex, Sidebar, SidebarEvent, SidebarRow, SidebarSection, SidebarSource,
    build_rows_sections,
};
use domain::{Conversation, ConversationId, ConversationKind, ProviderExt, WorkspaceState};
use gpui::{App, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use settings::{SidebarSettings, WorkspaceKey};

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

fn rows(
    s: &WorkspaceState,
    sel: Option<&ConversationId>,
    collapsed: &[SidebarSection],
) -> Vec<SidebarRow> {
    let settings = SidebarSettings {
        show_unreads_section: false,
        ..SidebarSettings::default()
    };
    build_rows_sections(
        s,
        &MemberIndex::build(s),
        sel,
        None,
        &settings,
        &HashSet::new(),
        &common::en(),
        false,
        &collapsed.iter().copied().collect::<BTreeSet<_>>(),
    )
}

fn labels(rows: &[SidebarRow]) -> Vec<String> {
    rows.iter()
        .map(|r| match r {
            SidebarRow::Header {
                label, collapsed, ..
            } => format!("{}{label}", if *collapsed { ">" } else { "v" }),
            SidebarRow::Conversation { label, .. } => label.to_string(),
            SidebarRow::ShowMore { label, .. } => format!("+{label}"),
        })
        .collect()
}

fn sample() -> WorkspaceState {
    let mut busy = conv("C2", ConversationKind::Channel, "busy");
    busy.unread = true;
    let mut dm = conv("D1", ConversationKind::Direct, "ana");
    dm.unread = true;
    state(vec![
        conv("C1", ConversationKind::Channel, "general"),
        busy,
        conv("C3", ConversationKind::Channel, "random"),
        dm,
        conv("D2", ConversationKind::Direct, "bo"),
    ])
}

#[test]
fn a_collapsed_section_keeps_only_its_unread_and_selected_rows() {
    let s = sample();
    assert_eq!(
        labels(&rows(&s, None, &[])),
        [
            "vChannels",
            "busy",
            "general",
            "random",
            "vDirect messages",
            "ana",
            "bo"
        ]
    );
    let sel = ConversationId(Arc::from("C3"));
    assert_eq!(
        labels(&rows(
            &s,
            Some(&sel),
            &[SidebarSection::Channels, SidebarSection::DirectMessages]
        )),
        [">Channels", "busy", "random", ">Direct messages", "ana"]
    );
}

#[test]
fn section_keys_round_trip() {
    for s in SidebarSection::ALL {
        assert_eq!(SidebarSection::from_key(&s.key()), Some(*s));
    }
    let custom = SidebarSection::Custom(7);
    assert_eq!(&*custom.key(), "section:7");
    assert_eq!(SidebarSection::from_key(&custom.key()), Some(custom));
    assert_eq!(SidebarSection::from_key("nope"), None);
    assert_eq!(SidebarSection::from_key("section:x"), None);
}

#[derive(Default)]
struct Src {
    rows: Vec<SidebarRow>,
    toggled: Vec<SidebarSection>,
    drafts: Option<usize>,
    drafts_opened: usize,
    people: bool,
    people_opened: usize,
}

impl SidebarSource for Src {
    fn rows(&self, _cx: &App) -> Vec<SidebarRow> {
        self.rows.clone()
    }
    fn open(&mut self, _id: ConversationId, _cx: &mut App) {}
    fn workspace_name(&self, _cx: &App) -> Option<gpui::SharedString> {
        Some("Example".into())
    }
    fn toggle_section(&mut self, section: SidebarSection, _cx: &mut App) {
        self.toggled.push(section);
    }
    fn drafts_entry(&self) -> Option<usize> {
        self.drafts
    }
    fn open_drafts(&mut self, _cx: &mut App) {
        self.drafts_opened += 1;
    }
    fn people_entry(&self) -> bool {
        self.people
    }
    fn open_people(&mut self, _cx: &mut App) {
        self.people_opened += 1;
    }
}

type Events = std::rc::Rc<std::cell::RefCell<Vec<SidebarEvent>>>;

fn setup(
    cx: &mut TestAppContext,
    drafts: Option<usize>,
) -> (
    Entity<Src>,
    Entity<Sidebar<Src>>,
    &mut VisualTestContext,
    Events,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(cx, &[components::catalog()], i18n::LocalePref::System);
    });
    let s = sample();
    let src = cx.new(|_| Src {
        rows: rows(&s, None, &[]),
        drafts,
        ..Src::default()
    });
    let src2 = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| Sidebar::new(src2, cx));
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &SidebarEvent, _| {
            log.borrow_mut().push(ev.clone());
        })
        .detach();
    });
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    (src, view, vcx, events)
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    let b = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} not drawn"));
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn clicking_a_section_header_asks_the_source_to_toggle_it(cx: &mut TestAppContext) {
    let (src, _view, vcx, _) = setup(cx, None);
    click(vcx, "sidebar-section-channels");
    click(vcx, "sidebar-section-direct_messages");
    assert_eq!(
        src.read_with(vcx, |s, _| s.toggled.clone()),
        [SidebarSection::Channels, SidebarSection::DirectMessages]
    );
}

#[gpui::test]
fn the_drafts_entry_shows_while_drafts_exist_and_opens_the_view(cx: &mut TestAppContext) {
    let (src, _view, vcx, _) = setup(cx, Some(3));
    click(vcx, "sidebar-drafts");
    assert_eq!(src.read_with(vcx, |s, _| s.drafts_opened), 1);
}

#[gpui::test]
fn the_people_entry_shows_when_offered_and_opens_the_directory(cx: &mut TestAppContext) {
    let (src, _view, vcx, _) = setup(cx, None);
    assert!(vcx.debug_bounds("sidebar-people").is_none());
    src.update(vcx, |s, cx| {
        s.people = true;
        cx.notify();
    });
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    click(vcx, "sidebar-people");
    assert_eq!(src.read_with(vcx, |s, _| s.people_opened), 1);
}

#[gpui::test]
fn no_drafts_hides_the_entry(cx: &mut TestAppContext) {
    let (_src, _view, vcx, _) = setup(cx, None);
    assert!(vcx.debug_bounds("sidebar-drafts").is_none());
}

#[gpui::test]
fn the_entry_points_ask_the_host_for_the_right_picker(cx: &mut TestAppContext) {
    let (src, _view, vcx, events) = setup(cx, None);
    click(vcx, "sidebar-new-message");
    // The "+" shows while its header is hovered.
    assert!(vcx.debug_bounds("sidebar-add-channels").is_none());
    for (head, add) in [
        ("sidebar-section-channels", "sidebar-add-channels"),
        (
            "sidebar-section-direct_messages",
            "sidebar-add-direct_messages",
        ),
    ] {
        let b = vcx.debug_bounds(head).unwrap();
        vcx.simulate_mouse_move(b.center(), None, gpui::Modifiers::none());
        vcx.run_until_parked();
        vcx.update(|w, cx| w.draw(cx).clear(cx));
        click(vcx, add);
    }
    click(vcx, "sidebar-workspace-name");
    let got: Vec<_> = events
        .borrow()
        .iter()
        .map(|e| match e {
            SidebarEvent::NewMessage => "new",
            SidebarEvent::BrowseChannels => "channels",
            SidebarEvent::BrowsePeople => "people",
            SidebarEvent::WorkspaceMenu { .. } => "menu",
            _ => "other",
        })
        .collect::<Vec<_>>();
    assert_eq!(got, ["new", "channels", "people", "menu"]);
    // The "+" buttons do not also toggle their section.
    assert!(src.read_with(vcx, |s, _| s.toggled.is_empty()));
}

fn unread_sample() -> WorkspaceState {
    sample()
}

fn rows_with_unreads(s: &WorkspaceState) -> Vec<SidebarRow> {
    build_rows_sections(
        s,
        &MemberIndex::build(s),
        None,
        None,
        &SidebarSettings::default(),
        &HashSet::new(),
        &common::en(),
        false,
        &[SidebarSection::Unreads].into_iter().collect(),
    )
}

#[test]
fn unreads_cannot_be_folded() {
    let s = unread_sample();
    let l = labels(&rows_with_unreads(&s));
    assert_eq!(&l[..3], ["vUnreads", "busy", "ana"]);
    assert!(!SidebarSection::Unreads.collapsible());
    assert!(SidebarSection::Channels.collapsible());
}

#[gpui::test]
fn every_foldable_header_has_its_chevron_and_unreads_none(cx: &mut TestAppContext) {
    let (src, _view, vcx, _) = setup(cx, None);
    src.update(vcx, |s, cx| {
        s.rows = rows_with_unreads(&unread_sample());
        cx.notify();
    });
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    use components::{IconName, header_chevron};
    assert_eq!(
        header_chevron(SidebarSection::Channels, false),
        Some(IconName::ChevDown)
    );
    assert_eq!(
        header_chevron(SidebarSection::DirectMessages, true),
        Some(IconName::ChevRight)
    );
    assert_eq!(
        header_chevron(SidebarSection::Starred, false),
        Some(IconName::ChevDown)
    );
    assert_eq!(header_chevron(SidebarSection::Unreads, false), None);
    click(vcx, "sidebar-section-unreads");
    assert!(src.read_with(vcx, |s, _| s.toggled.is_empty()));
}

#[gpui::test]
fn the_keyboard_lands_on_headers_and_enter_folds(cx: &mut TestAppContext) {
    let (src, view, vcx, events) = setup(cx, None);
    vcx.update(|w, cx| w.focus(&view.read(cx).focus_handle(cx), cx));
    vcx.dispatch_action(components::SelectNext);
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert_eq!(
        view.read_with(vcx, |v, _| v.cursor_section()),
        Some(SidebarSection::Channels)
    );
    // The "+" shows for the header the keyboard is on.
    assert!(vcx.debug_bounds("sidebar-add-channels").is_some());
    vcx.dispatch_action(components::Open);
    vcx.run_until_parked();
    assert_eq!(
        src.read_with(vcx, |s, _| s.toggled.clone()),
        [SidebarSection::Channels]
    );
    assert!(events.borrow().is_empty());
}

#[gpui::test]
fn a_focused_header_folds_on_space_and_its_plus_is_a_tab_stop(cx: &mut TestAppContext) {
    let (src, view, vcx, events) = setup(cx, None);
    let head = view.read_with(vcx, |v, _| v.section_focus(SidebarSection::Channels));
    vcx.update(|w, cx| w.focus(&head, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(
        vcx.debug_bounds("sidebar-add-channels").is_some(),
        "focus within shows +"
    );
    vcx.simulate_keystrokes("space");
    assert_eq!(
        src.read_with(vcx, |s, _| s.toggled.clone()),
        [SidebarSection::Channels]
    );
    // Tab order is the window's: the "+" is the next stop after its header.
    vcx.update(|window, cx| window.focus_next(cx));
    vcx.simulate_keystrokes("enter");
    assert!(matches!(
        events.borrow().last(),
        Some(SidebarEvent::BrowseChannels)
    ));
}

#[gpui::test]
fn the_hovered_header_follows_its_section_when_rows_move(cx: &mut TestAppContext) {
    let (src, _view, vcx, _) = setup(cx, None);
    let b = vcx.debug_bounds("sidebar-section-channels").unwrap();
    vcx.simulate_mouse_move(b.center(), None, gpui::Modifiers::none());
    vcx.run_until_parked();
    // A Starred section appears above: Channels moves down a few rows.
    let mut s = sample();
    s.starred.insert(ConversationId(Arc::from("C1")));
    src.update(vcx, |src, cx| {
        src.rows = rows(&s, None, &[]);
        cx.notify();
    });
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("sidebar-add-direct_messages").is_none());
    assert!(vcx.debug_bounds("sidebar-section-starred").is_some());
}

#[gpui::test]
fn the_drafts_entry_has_its_icon_and_count(cx: &mut TestAppContext) {
    let (_src, _view, vcx, _) = setup(cx, Some(3));
    assert!(vcx.debug_bounds("sidebar-drafts-icon").is_some());
    assert!(vcx.debug_bounds("sidebar-drafts-count").is_some());
}

#[gpui::test]
fn a_draft_row_draws_the_pencil_icon(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(cx, &[components::catalog()], i18n::LocalePref::System);
    });
    let s = state(vec![conv("C1", ConversationKind::Channel, "general")]);
    let mut row_list = rows(&s, None, &[]);
    for r in &mut row_list {
        if let SidebarRow::Conversation { draft, .. } = r {
            *draft = true;
        }
    }
    assert_eq!(row_list.len(), 2, "a header and one conversation row");
    let src = cx.new(|_| Src {
        rows: row_list,
        ..Src::default()
    });
    let (_view, vcx) = cx.add_window_view(move |_, cx| Sidebar::new(src, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(
        vcx.debug_bounds("sidebar-draft-icon-1").is_some(),
        "the draft marker is an icon element, not a text glyph"
    );
}
