//! The toast stack: bounded, one timer per shown toast, action and dismiss.

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use components::{
    MAX_QUEUED_TOASTS, MAX_VISIBLE_TOASTS, TOAST_ACTION_MIN, Toast, ToastKind, ToastStack, Toasts,
};
use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext, point, px};

/// A window that draws the stack at the bottom left, as the app does.
struct Host(Entity<ToastStack>);

impl gpui::Render for Host {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{Styled as _, div, prelude::*};
        div().size_full().relative().child(
            div()
                .absolute()
                .left(px(20.0))
                .bottom(px(20.0))
                .child(self.0.clone()),
        )
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<ToastStack>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let stack = cx.new(ToastStack::new);
    let s = stack.clone();
    let (_host, vcx) = cx.add_window_view(move |_, _| Host(s));
    // Focus events only run in the active window.
    vcx.update(|w, _| w.activate_window());
    (stack, vcx)
}

fn titles(s: &Entity<ToastStack>, cx: &mut VisualTestContext) -> Vec<String> {
    s.read_with(cx, |s, _| {
        s.visible().iter().map(|t| t.title.to_string()).collect()
    })
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn advance(cx: &mut VisualTestContext, d: Duration) {
    cx.executor().advance_clock(d);
    draw(cx);
}

#[gpui::test]
fn pushing_shows_a_toast_with_its_text(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        s.push(Toast::new(ToastKind::Info, "Link copied"), cx)
    });
    draw(vcx);
    assert_eq!(titles(&s, vcx), ["Link copied"]);
    assert!(vcx.debug_bounds("toast-0").is_some());
    assert!(vcx.debug_bounds("toast-dismiss-0").is_some());
}

#[gpui::test]
fn an_empty_stack_draws_nothing_and_holds_no_timer(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    draw(vcx);
    assert!(vcx.debug_bounds("toast-0").is_none());
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 0);
}

#[gpui::test]
fn the_action_runs_once_and_dismisses(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    let ran = Rc::new(Cell::new(0));
    let r = ran.clone();
    s.update(vcx, |s, cx| {
        s.push(
            Toast::new(ToastKind::Success, "Message deleted")
                .action("Undo", move |_| r.set(r.get() + 1)),
            cx,
        )
    });
    draw(vcx);
    let b = vcx.debug_bounds("toast-action-0").expect("action drawn");
    vcx.simulate_click(b.center(), Modifiers::none());
    draw(vcx);
    assert_eq!(ran.get(), 1);
    assert!(titles(&s, vcx).is_empty());
    // Its timer died with it: no timer is left and time passing runs nothing.
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 0);
    advance(vcx, Duration::from_secs(60));
    assert_eq!(ran.get(), 1);
}

#[gpui::test]
fn an_action_may_push_another_toast(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    let target = s.clone();
    s.update(vcx, |s, cx| {
        s.push(
            Toast::new(ToastKind::Success, "Message deleted").action("Undo", move |cx| {
                // Runs outside the stack's update: pushing and dismissing are fine.
                target.update(cx, |s, cx| {
                    s.push(Toast::new(ToastKind::Info, "Restored"), cx);
                });
            }),
            cx,
        )
    });
    draw(vcx);
    let b = vcx.debug_bounds("toast-action-0").unwrap();
    vcx.simulate_click(b.center(), Modifiers::none());
    draw(vcx);
    assert_eq!(titles(&s, vcx), ["Restored"]);
}

#[gpui::test]
fn the_close_button_dismisses(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        s.push(Toast::new(ToastKind::Error, "Upload failed"), cx)
    });
    draw(vcx);
    let b = vcx.debug_bounds("toast-dismiss-0").unwrap();
    vcx.simulate_click(b.center(), Modifiers::none());
    assert!(titles(&s, vcx).is_empty());
}

#[gpui::test]
fn a_toast_goes_after_its_timeout(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        s.push(
            Toast::new(ToastKind::Info, "Saved").timeout(Duration::from_secs(5)),
            cx,
        )
    });
    draw(vcx);
    advance(vcx, Duration::from_millis(4900));
    assert_eq!(titles(&s, vcx), ["Saved"]);
    advance(vcx, Duration::from_millis(200));
    assert!(titles(&s, vcx).is_empty());
}

#[gpui::test]
fn a_toast_with_an_action_stays_at_least_ten_seconds(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        s.push(
            Toast::new(ToastKind::Info, "Sent")
                .action("Undo", |_| {})
                .timeout(Duration::from_secs(2)),
            cx,
        )
    });
    draw(vcx);
    advance(vcx, TOAST_ACTION_MIN - Duration::from_millis(100));
    assert_eq!(titles(&s, vcx), ["Sent"]);
    advance(vcx, Duration::from_millis(200));
    assert!(titles(&s, vcx).is_empty());
}

#[gpui::test]
fn the_pointer_on_a_toast_pauses_its_timer(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        s.push(
            Toast::new(ToastKind::Info, "Saved").timeout(Duration::from_secs(5)),
            cx,
        )
    });
    draw(vcx);
    let b = vcx.debug_bounds("toast-0").unwrap();
    vcx.simulate_mouse_move(b.center(), None, Modifiers::none());
    draw(vcx);
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 0, "held: no timer");
    advance(vcx, Duration::from_secs(60));
    assert_eq!(titles(&s, vcx), ["Saved"]);
    // Pointer away: the full stay starts again.
    vcx.simulate_mouse_move(point(px(900.0), px(700.0)), None, Modifiers::none());
    draw(vcx);
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 1);
    advance(vcx, Duration::from_millis(4900));
    assert_eq!(titles(&s, vcx), ["Saved"]);
    advance(vcx, Duration::from_millis(200));
    assert!(titles(&s, vcx).is_empty());
}

#[gpui::test]
fn focus_on_a_toast_pauses_its_timer(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        s.push(
            Toast::new(ToastKind::Info, "Saved")
                .action("Undo", |_| {})
                .timeout(Duration::from_secs(5)),
            cx,
        )
    });
    draw(vcx);
    vcx.update(|w, cx| w.focus_next(cx));
    draw(vcx);
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 0, "focused: no timer");
    advance(vcx, Duration::from_secs(120));
    assert_eq!(titles(&s, vcx), ["Saved"]);
    vcx.update(|w, cx| w.blur(cx));
    draw(vcx);
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 1);
    advance(vcx, TOAST_ACTION_MIN + Duration::from_millis(100));
    assert!(titles(&s, vcx).is_empty());
}

#[gpui::test]
fn focus_stays_with_its_toast_when_an_older_one_leaves(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        s.push(Toast::new(ToastKind::Info, "A").action("Undo", |_| {}), cx);
        s.push(Toast::new(ToastKind::Info, "B").action("Undo", |_| {}), cx);
    });
    draw(vcx);
    // Tab stops in order: A's action, A's close, B's action.
    for _ in 0..3 {
        vcx.update(|w, cx| w.focus_next(cx));
        draw(vcx);
    }
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 1, "only A is free");
    let a = s.read_with(vcx, |s, _| s.visible()[0].id);
    s.update(vcx, |s, cx| s.dismiss(a, cx));
    draw(vcx);
    assert_eq!(titles(&s, vcx), ["B"]);
    assert!(vcx.update(|w, cx| w.focused(cx).is_some()), "focus kept");
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 0, "B is still held");
    assert!(vcx.debug_bounds("toast-1").is_some());
}

#[gpui::test]
fn only_the_cap_is_visible_and_the_rest_wait(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        for i in 0..MAX_VISIBLE_TOASTS + 2 {
            s.push(
                Toast::new(ToastKind::Info, format!("t{i}")).timeout(Duration::from_secs(5)),
                cx,
            );
        }
    });
    draw(vcx);
    assert_eq!(titles(&s, vcx).len(), MAX_VISIBLE_TOASTS);
    assert_eq!(titles(&s, vcx)[0], "t0");
    // A queued toast starts its own timeout only when it is shown.
    advance(vcx, Duration::from_secs(6));
    assert_eq!(
        titles(&s, vcx),
        [
            format!("t{}", MAX_VISIBLE_TOASTS),
            format!("t{}", MAX_VISIBLE_TOASTS + 1)
        ]
    );
    advance(vcx, Duration::from_secs(6));
    assert!(titles(&s, vcx).is_empty());
}

#[gpui::test]
fn the_queue_is_bounded_and_drops_the_oldest_waiting(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    let total = MAX_VISIBLE_TOASTS + MAX_QUEUED_TOASTS + 3;
    s.update(vcx, |s, cx| {
        for i in 0..total {
            s.push(Toast::new(ToastKind::Info, format!("t{i}")), cx);
        }
    });
    assert_eq!(s.read_with(vcx, |s, _| s.queued()), MAX_QUEUED_TOASTS);
    s.update(vcx, |s, cx| {
        for _ in 0..MAX_VISIBLE_TOASTS {
            let id = s.visible()[0].id;
            s.dismiss(id, cx);
        }
    });
    assert_eq!(titles(&s, vcx)[0], format!("t{}", MAX_VISIBLE_TOASTS + 3));
}

#[gpui::test]
fn a_full_queue_drops_a_plain_notice_before_an_action(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        for i in 0..MAX_VISIBLE_TOASTS {
            s.push(Toast::new(ToastKind::Info, format!("shown{i}")), cx);
        }
        // The oldest waiting toast has an action; the rest are plain.
        s.push(
            Toast::new(ToastKind::Info, "keep").action("Undo", |_| {}),
            cx,
        );
        for i in 1..MAX_QUEUED_TOASTS {
            s.push(Toast::new(ToastKind::Info, format!("plain{i}")), cx);
        }
        s.push(Toast::new(ToastKind::Info, "newest"), cx);
        assert_eq!(s.queued(), MAX_QUEUED_TOASTS);
        for _ in 0..MAX_VISIBLE_TOASTS {
            let id = s.visible()[0].id;
            s.dismiss(id, cx);
        }
    });
    let mut all = Vec::new();
    s.update(vcx, |s, cx| {
        while let Some(t) = s.visible().first() {
            let id = t.id;
            all.push(t.title.to_string());
            s.dismiss(id, cx);
        }
    });
    assert_eq!(all[0], "keep", "the actionable toast survived");
    assert!(!all.contains(&"plain1".to_string()), "oldest plain dropped");
    assert!(all.contains(&"newest".to_string()));
}

#[gpui::test]
fn the_same_message_again_does_not_stack(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        s.push(Toast::new(ToastKind::Info, "Link copied"), cx);
        s.push(Toast::new(ToastKind::Info, "Link copied"), cx);
    });
    assert_eq!(titles(&s, vcx), ["Link copied"]);
}

#[gpui::test]
fn no_timer_is_left_when_nothing_shows(cx: &mut TestAppContext) {
    let (s, vcx) = setup(cx);
    s.update(vcx, |s, cx| {
        s.push(Toast::new(ToastKind::Info, "a"), cx);
        s.push(Toast::new(ToastKind::Info, "b"), cx);
    });
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 2);
    s.update(vcx, |s, cx| {
        let id = s.visible()[0].id;
        s.dismiss(id, cx);
    });
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 1);
    advance(vcx, Duration::from_secs(60));
    assert_eq!(s.read_with(vcx, |s, _| s.timers()), 0);
}

#[gpui::test]
fn the_stack_is_created_on_the_first_push_not_before(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        assert!(Toasts::target(cx).is_none(), "nothing exists yet");
        Toasts::push(cx, Toast::new(ToastKind::Success, "Done"));
        let stack = Toasts::target(cx).expect("created by the push");
        assert_eq!(stack.read(cx).visible().len(), 1);
        Toasts::push(cx, Toast::new(ToastKind::Success, "Again"));
        assert_eq!(
            Toasts::target(cx).unwrap().read(cx).visible().len(),
            2,
            "the same stack"
        );
    });
}

#[gpui::test]
fn a_toast_shows_in_the_window_that_was_active_when_it_was_pushed(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let wa = cx.add_window(|_, _| gpui::Empty);
    let wb = cx.add_window(|_, _| gpui::Empty);
    let a_stack = wa
        .update(cx, |_, window, cx| Toasts::host(window, cx))
        .unwrap();
    let b_stack = wb
        .update(cx, |_, window, cx| Toasts::host(window, cx))
        .unwrap();
    assert_ne!(a_stack, b_stack);
    wb.update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.update(|cx| {
        Toasts::push(cx, Toast::new(ToastKind::Info, "here"));
    });
    assert_eq!(b_stack.read_with(cx, |s, _| s.visible().len()), 1);
    assert_eq!(a_stack.read_with(cx, |s, _| s.visible().len()), 0);
}
