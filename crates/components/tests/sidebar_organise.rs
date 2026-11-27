//! The sidebar settings popover, organise mode, the section menu and dragging
//! rows and sections into place.

mod common;

use std::collections::HashSet;
use std::sync::Arc;

use components::{
    SectionOp, Sidebar, SidebarOption, SidebarRow, SidebarSource, apply_section_op, build_rows,
};
use domain::{Conversation, ConversationId, ConversationKind, ProviderExt, WorkspaceState};
use gpui::{
    App, AppContext as _, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext,
};
use settings::{CustomSection, SidebarSettings, SidebarSort, WorkspaceKey};

fn conv(id: &str, name: &str) -> Conversation {
    Conversation {
        id: ConversationId(Arc::from(id)),
        kind: ConversationKind::Channel,
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

/// A source that keeps its sections in memory, as the host keeps them in
/// the workspace's settings file.
struct Src {
    state: WorkspaceState,
    settings: SidebarSettings,
    ops: Vec<SectionOp>,
    options: Vec<SidebarOption>,
}

impl SidebarSource for Src {
    fn rows(&self, _cx: &App) -> Vec<SidebarRow> {
        build_rows(
            &self.state,
            None,
            &self.settings,
            &HashSet::new(),
            &common::en(),
        )
    }
    fn open(&mut self, _id: ConversationId, _cx: &mut App) {}
    fn workspace_name(&self, _cx: &App) -> Option<gpui::SharedString> {
        Some("Example".into())
    }
    fn section_op(&mut self, op: SectionOp, _cx: &mut App) {
        self.ops.push(op.clone());
        apply_section_op(&mut self.settings.sections, op);
    }
    fn set_sidebar_option(&mut self, option: SidebarOption, _cx: &mut App) {
        self.options.push(option);
    }
}

fn setup(
    cx: &mut TestAppContext,
    sections: Vec<CustomSection>,
) -> (Entity<Src>, Entity<Sidebar<Src>>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(cx, &[components::catalog()], i18n::LocalePref::System);
    });
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    for c in [conv("C1", "general"), conv("C2", "design")] {
        state.conversations.insert(c.id.clone(), c);
    }
    let src = cx.new(|_| Src {
        state,
        settings: SidebarSettings {
            sections,
            ..SidebarSettings::default()
        },
        ops: Vec::new(),
        options: Vec::new(),
    });
    let src2 = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| Sidebar::new(src2, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    (src, view, vcx)
}

fn focus(id: u32, name: &str, convs: &[&str]) -> CustomSection {
    CustomSection {
        id,
        name: name.into(),
        conversations: convs.iter().map(|c| c.to_string()).collect(),
    }
}

fn redraw(vcx: &mut VisualTestContext) {
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
}

fn open_settings(view: &Entity<Sidebar<Src>>, vcx: &mut VisualTestContext) {
    vcx.update(|w, cx| {
        view.update(cx, |v, cx| {
            v.open_settings(gpui::point(gpui::px(40.0), gpui::px(40.0)), w, cx)
        })
    });
    redraw(vcx);
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    let b = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} not drawn"));
    vcx.simulate_click(b.center(), Modifiers::none());
    redraw(vcx);
}

#[gpui::test]
fn the_settings_popover_changes_the_sidebar_options(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, Vec::new());
    open_settings(&view, vcx);
    assert!(view.read_with(vcx, |v, _| v.settings_open()));
    assert!(vcx.debug_bounds("sidebar-settings-popover").is_some());
    click(vcx, "sidebar-settings-unreads");
    click(vcx, "sidebar-settings-sort-recent");
    click(vcx, "sidebar-settings-density-cozy");
    assert_eq!(
        src.read_with(vcx, |s, _| s.options.clone()),
        [
            SidebarOption::ShowUnreads(false),
            SidebarOption::Sort(SidebarSort::Recent),
            SidebarOption::Density(settings::Density::Cozy),
        ]
    );
    // Escape closes it.
    vcx.simulate_keystrokes("escape");
    redraw(vcx);
    assert!(!view.read_with(vcx, |v, _| v.settings_open()));
    assert!(vcx.debug_bounds("sidebar-settings-popover").is_none());
}

#[gpui::test]
fn organise_mode_creates_a_section_from_the_field(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, Vec::new());
    open_settings(&view, vcx);
    click(vcx, "sidebar-settings-organise");
    assert!(view.read_with(vcx, |v, _| v.organising()));
    assert!(vcx.debug_bounds("sidebar-done").is_some());
    assert!(vcx.debug_bounds("sidebar-new-message").is_none());
    // The field has the focus; Space types (it is not the list's Open key).
    vcx.simulate_input("Q4 launch");
    vcx.simulate_keystrokes("enter");
    redraw(vcx);
    assert_eq!(
        src.read_with(vcx, |s, _| s.ops.clone()),
        [SectionOp::Create {
            name: "Q4 launch".into()
        }]
    );
    assert!(vcx.debug_bounds("sidebar-section-custom-1").is_some());
    let left = view.read_with(vcx, |v, cx| {
        v.new_section_input().unwrap().read(cx).text().to_string()
    });
    assert_eq!(left, "");
    click(vcx, "sidebar-done");
    assert!(!view.read_with(vcx, |v, _| v.organising()));
    assert!(vcx.debug_bounds("sidebar-new-section").is_none());
}

#[gpui::test]
fn the_section_menu_renames_moves_and_deletes(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(
        cx,
        vec![focus(1, "Focus", &["C2"]), focus(2, "Clients", &[])],
    );
    // Right click on a custom header opens its menu.
    let b = vcx.debug_bounds("sidebar-section-custom-1").unwrap();
    vcx.simulate_mouse_down(b.center(), MouseButton::Right, Modifiers::none());
    vcx.simulate_mouse_up(b.center(), MouseButton::Right, Modifiers::none());
    redraw(vcx);
    assert_eq!(view.read_with(vcx, |v, _| v.section_menu()), Some(1));
    // The first section cannot move up.
    assert!(vcx.debug_bounds("sidebar-section-move-up").is_none());
    click(vcx, "sidebar-section-rename");
    let (id, input) = view.read_with(vcx, |v, _| {
        let (id, i) = v.rename_input().unwrap();
        (id, i.clone())
    });
    assert_eq!(id, 1);
    assert_eq!(input.read_with(vcx, |i, _| i.text().to_string()), "Focus");
    vcx.update(|_, cx| input.update(cx, |i, cx| i.set_text("Deep work", cx)));
    vcx.simulate_keystrokes("enter");
    redraw(vcx);
    assert!(view.read_with(vcx, |v, _| v.rename_input().is_none()));
    // Move it down, then delete it.
    let b = vcx.debug_bounds("sidebar-section-custom-1").unwrap();
    vcx.simulate_mouse_down(b.center(), MouseButton::Right, Modifiers::none());
    redraw(vcx);
    click(vcx, "sidebar-section-move-down");
    let b = vcx.debug_bounds("sidebar-section-custom-1").unwrap();
    vcx.simulate_mouse_down(b.center(), MouseButton::Right, Modifiers::none());
    redraw(vcx);
    click(vcx, "sidebar-section-delete");
    assert_eq!(
        src.read_with(vcx, |s, _| s.ops.clone()),
        [
            SectionOp::Rename {
                id: 1,
                name: "Deep work".into()
            },
            SectionOp::Reorder { id: 1, to: 1 },
            SectionOp::Delete { id: 1 },
        ]
    );
    assert!(vcx.debug_bounds("sidebar-section-custom-1").is_none());
    assert!(vcx.debug_bounds("sidebar-row-C2").is_some());
}

fn drag(vcx: &mut VisualTestContext, from: &'static str, to: &'static str) {
    let a = vcx.debug_bounds(from).unwrap().center();
    let b = vcx.debug_bounds(to).unwrap().center();
    vcx.simulate_mouse_down(a, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        gpui::point(a.x, a.y + gpui::px(6.0)),
        MouseButton::Left,
        Modifiers::none(),
    );
    redraw(vcx);
    vcx.simulate_mouse_move(b, MouseButton::Left, Modifiers::none());
    redraw(vcx);
    vcx.simulate_mouse_up(b, MouseButton::Left, Modifiers::none());
    redraw(vcx);
}

#[gpui::test]
fn organise_mode_drags_rows_and_sections(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, vec![focus(1, "Focus", &[]), focus(2, "Clients", &[])]);
    vcx.update(|w, cx| view.update(cx, |v, cx| v.set_organising(true, w, cx)));
    redraw(vcx);
    // A row onto a custom section's header moves it there.
    drag(vcx, "sidebar-row-C1", "sidebar-section-custom-2");
    // A section onto another moves it to that place.
    drag(vcx, "sidebar-section-custom-2", "sidebar-section-custom-1");
    // A row onto Channels sends it back.
    drag(vcx, "sidebar-row-C1", "sidebar-section-channels");
    assert_eq!(
        src.read_with(vcx, |s, _| s.ops.clone()),
        [
            SectionOp::MoveConversation {
                conversation: "C1".into(),
                to: Some(2)
            },
            SectionOp::Reorder { id: 2, to: 0 },
            SectionOp::MoveConversation {
                conversation: "C1".into(),
                to: None
            },
        ]
    );
}

#[gpui::test]
fn rows_do_not_drag_outside_organise_mode(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, vec![focus(1, "Focus", &[])]);
    drag(vcx, "sidebar-row-C1", "sidebar-section-custom-1");
    assert!(src.read_with(vcx, |s, _| s.ops.is_empty()));
}

#[gpui::test]
fn list_keys_do_not_reach_the_list_from_the_popover(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, Vec::new());
    open_settings(&view, vcx);
    // Up, Down and Enter on the popover leave the list alone.
    vcx.simulate_keystrokes("down down enter up");
    redraw(vcx);
    assert!(view.read_with(vcx, |v, _| v.settings_open()));
    assert!(view.read_with(vcx, |v, _| v.cursor_section().is_none()));
    assert!(src.read_with(vcx, |s, _| s.options.is_empty()));
    // Tab reaches the switch; Space flips it.
    vcx.update(|w, cx| w.focus_next(cx));
    vcx.simulate_keystrokes("space");
    redraw(vcx);
    assert_eq!(
        src.read_with(vcx, |s, _| s.options.clone()),
        [SidebarOption::ShowUnreads(false)]
    );
    // Enter on the next control (A to Z) picks it, not a list row.
    vcx.update(|w, cx| w.focus_next(cx));
    vcx.simulate_keystrokes("enter");
    redraw(vcx);
    assert_eq!(
        src.read_with(vcx, |s, _| s.options.last().cloned()),
        Some(SidebarOption::Sort(SidebarSort::Alphabetical))
    );
}

#[gpui::test]
fn the_keyboard_opens_the_section_menu_and_picks_an_entry(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, vec![focus(1, "Focus", &["C2"])]);
    let head = view.read_with(vcx, |v, _| {
        v.section_focus(components::SidebarSection::Custom(1))
    });
    vcx.update(|w, cx| w.focus(&head, cx));
    redraw(vcx);
    // The header's "…" shows while the keyboard is on it, and is its next stop.
    assert!(vcx.debug_bounds("sidebar-section-more-1").is_some());
    vcx.update(|w, cx| w.focus_next(cx));
    vcx.simulate_keystrokes("enter");
    redraw(vcx);
    assert_eq!(view.read_with(vcx, |v, _| v.section_menu()), Some(1));
    assert!(vcx.debug_bounds("sidebar-section-menu").is_some());
    // Down does not move the list; Tab reaches Rename, Enter starts it.
    vcx.simulate_keystrokes("down");
    assert_eq!(view.read_with(vcx, |v, _| v.section_menu()), Some(1));
    vcx.update(|w, cx| w.focus_next(cx));
    vcx.simulate_keystrokes("enter");
    redraw(vcx);
    assert_eq!(
        view.read_with(vcx, |v, _| v.rename_input().map(|r| r.0)),
        Some(1)
    );
    assert!(src.read_with(vcx, |s, _| s.ops.is_empty()));
}

#[gpui::test]
fn a_rename_commits_when_the_field_loses_focus(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, vec![focus(1, "Focus", &[])]);
    // Focus events reach listeners only in the active window.
    vcx.update(|w, _| w.activate_window());
    redraw(vcx);
    vcx.update(|w, cx| view.update(cx, |v, cx| v.start_rename(1, w, cx)));
    redraw(vcx);
    let input = view.read_with(vcx, |v, _| v.rename_input().unwrap().1.clone());
    let focused =
        vcx.update(|w, cx| gpui::Focusable::focus_handle(input.read(cx), cx).is_focused(w));
    assert!(focused, "the rename field has the keyboard");

    vcx.update(|_, cx| input.update(cx, |i, cx| i.set_text("Deep work", cx)));
    let list = view.read_with(vcx, gpui::Focusable::focus_handle);
    vcx.update(|w, cx| w.focus(&list, cx));
    redraw(vcx);
    assert!(view.read_with(vcx, |v, _| v.rename_input().is_none()));
    assert_eq!(
        src.read_with(vcx, |s, _| s.ops.clone()),
        [SectionOp::Rename {
            id: 1,
            name: "Deep work".into()
        }]
    );
}

#[gpui::test]
fn escape_in_the_new_section_field_clears_it_then_ends_organising(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, Vec::new());
    vcx.update(|w, cx| view.update(cx, |v, cx| v.set_organising(true, w, cx)));
    redraw(vcx);
    vcx.simulate_input("Half");
    vcx.simulate_keystrokes("escape");
    redraw(vcx);
    assert!(view.read_with(vcx, |v, _| v.organising()));
    let text = view.read_with(vcx, |v, cx| {
        v.new_section_input().unwrap().read(cx).text().to_string()
    });
    assert_eq!(text, "");
    vcx.simulate_keystrokes("escape");
    redraw(vcx);
    assert!(!view.read_with(vcx, |v, _| v.organising()));
}

#[gpui::test]
fn a_row_dropped_on_a_row_of_a_custom_section_joins_it(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, vec![focus(1, "Focus", &["C2"])]);
    vcx.update(|w, cx| view.update(cx, |v, cx| v.set_organising(true, w, cx)));
    redraw(vcx);
    drag(vcx, "sidebar-row-C1", "sidebar-row-C2");
    assert_eq!(
        src.read_with(vcx, |s, _| s.ops.clone()),
        [SectionOp::MoveConversation {
            conversation: "C1".into(),
            to: Some(1)
        }]
    );
}
