//! The gpui-base wrappers: popover dismissal, focus return and edge flip;
//! dialog focus trapping.

use std::cell::RefCell;
use std::rc::Rc;

use components::dialog::Dialog;
use components::popover::Popover;
use gpui::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, Modifiers, ParentElement as _,
    Render, Styled as _, TestAppContext, VisualTestContext, Window, div, px,
};

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| window.draw(cx).clear(cx));
}

struct PopoverHarness {
    trigger_focus: FocusHandle,
    placement: gpui_base::Placement,
    changes: Rc<RefCell<Vec<bool>>>,
}

impl Render for PopoverHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let changes = self.changes.clone();
        div().size_full().flex().flex_col().justify_end().child(
            Popover::new("test-popover")
                .placement(self.placement)
                .trigger_focus(self.trigger_focus.clone())
                .trigger(
                    div()
                        .debug_selector(|| "test-popover-trigger".into())
                        .w(px(120.))
                        .h(px(32.))
                        .child("Open"),
                )
                .content(|_, _, _| {
                    div()
                        .debug_selector(|| "test-popover-content".into())
                        .w(px(160.))
                        .h(px(80.))
                        .child("Menu")
                })
                .on_open_change(move |open, _, _| changes.borrow_mut().push(*open)),
        )
    }
}

fn popover(
    cx: &mut TestAppContext,
    placement: gpui_base::Placement,
) -> (FocusHandle, Rc<RefCell<Vec<bool>>>, &mut VisualTestContext) {
    init(cx);
    let changes = Rc::new(RefCell::new(Vec::new()));
    let trigger_focus = cx.update(|cx| cx.focus_handle());
    let (_, vcx) = cx.add_window_view({
        let changes = changes.clone();
        let trigger_focus = trigger_focus.clone();
        move |_, _| PopoverHarness {
            trigger_focus,
            placement,
            changes,
        }
    });
    draw(vcx);
    (trigger_focus, changes, vcx)
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    let bounds = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
}

#[gpui::test]
fn escape_closes_the_popover_and_returns_focus_to_the_trigger(cx: &mut TestAppContext) {
    let (trigger_focus, changes, vcx) = popover(cx, gpui_base::Placement::Bottom);
    click(vcx, "test-popover-trigger");
    draw(vcx);

    assert!(vcx.debug_bounds("test-popover-content").is_some());
    assert_eq!(&*changes.borrow(), &[true]);
    assert_ne!(
        vcx.update(|window, cx| window.focused(cx)),
        Some(trigger_focus.clone()),
        "the popover content takes the focus"
    );

    vcx.simulate_keystrokes("escape");
    draw(vcx);

    assert!(vcx.debug_bounds("test-popover-content").is_none());
    assert_eq!(&*changes.borrow(), &[true, false]);
    assert_eq!(
        vcx.update(|window, cx| window.focused(cx)),
        Some(trigger_focus),
        "focus returns to the trigger"
    );
}

#[gpui::test]
fn a_click_outside_closes_the_popover(cx: &mut TestAppContext) {
    let (_, changes, vcx) = popover(cx, gpui_base::Placement::Bottom);
    click(vcx, "test-popover-trigger");
    draw(vcx);
    assert!(vcx.debug_bounds("test-popover-content").is_some());

    vcx.simulate_click(gpui::point(px(380.), px(280.)), Modifiers::none());
    draw(vcx);

    assert!(vcx.debug_bounds("test-popover-content").is_none());
    assert_eq!(&*changes.borrow(), &[true, false]);
}

#[gpui::test]
fn a_popover_near_the_bottom_edge_flips_above_the_trigger(cx: &mut TestAppContext) {
    let (_, _, vcx) = popover(cx, gpui_base::Placement::Bottom);
    click(vcx, "test-popover-trigger");
    draw(vcx);

    let trigger = vcx
        .debug_bounds("test-popover-trigger")
        .expect("trigger drawn");
    let content = vcx
        .debug_bounds("test-popover-content")
        .expect("content drawn");
    assert!(
        content.bottom() <= trigger.top(),
        "content {content:?} should flip above trigger {trigger:?}"
    );
}

struct DialogHarness {
    first: FocusHandle,
    second: FocusHandle,
    behind: FocusHandle,
}

impl Render for DialogHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (first, second) = (self.first.clone(), self.second.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .debug_selector(|| "dialog-behind".into())
                    .track_focus(&self.behind)
                    .tab_stop(true)
                    .child("Behind"),
            )
            .child(
                Dialog::new(cx).open(true).title("Delete message?").body(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            div()
                                .debug_selector(|| "dialog-first".into())
                                .track_focus(&first)
                                .tab_stop(true)
                                .child("Cancel"),
                        )
                        .child(
                            div()
                                .debug_selector(|| "dialog-second".into())
                                .track_focus(&second)
                                .tab_stop(true)
                                .child("Delete"),
                        ),
                ),
            )
    }
}

fn dialog(
    cx: &mut TestAppContext,
) -> (
    FocusHandle,
    FocusHandle,
    FocusHandle,
    &mut VisualTestContext,
) {
    init(cx);
    let (first, second, behind) = cx.update(|cx| {
        (
            cx.focus_handle().tab_stop(true),
            cx.focus_handle().tab_stop(true),
            cx.focus_handle().tab_stop(true),
        )
    });
    let (_, vcx) = cx.add_window_view({
        let (first, second, behind) = (first.clone(), second.clone(), behind.clone());
        move |_, _| DialogHarness {
            first,
            second,
            behind,
        }
    });
    draw(vcx);
    (first, second, behind, vcx)
}

#[gpui::test]
fn tab_cycles_inside_the_dialog_and_never_reaches_the_window_behind(cx: &mut TestAppContext) {
    let (first, second, behind, vcx) = dialog(cx);
    vcx.update(|window, cx| window.focus(&first, cx));
    draw(vcx);

    vcx.simulate_keystrokes("tab");
    assert_eq!(
        vcx.update(|window, cx| window.focused(cx)),
        Some(second.clone())
    );

    vcx.simulate_keystrokes("tab");
    assert_eq!(
        vcx.update(|window, cx| window.focused(cx)),
        Some(first.clone())
    );

    vcx.simulate_keystrokes("shift-tab");
    assert_eq!(
        vcx.update(|window, cx| window.focused(cx)),
        Some(second.clone())
    );

    let focused = vcx.update(|window, cx| window.focused(cx));
    assert_ne!(focused, Some(behind), "focus never leaves the dialog");
}
