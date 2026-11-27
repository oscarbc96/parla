//! The message row: the avatar belongs to the first message of a group only.

mod common;

use components::message_row::MessageRow;
use domain::DeliveryState;
use gpui::ParentElement as _;
use gpui::{Context, IntoElement, Render, TestAppContext, Window, div};
use settings::Density;
use themes::ActiveTheme as _;

struct Host {
    grouped: bool,
    pending: bool,
    queued: bool,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        div().child(
            MessageRow {
                author: "Bob".into(),
                status: None,
                author_link: None,
                time: "10:00".into(),
                text: "hello".into(),
                rich: None,
                edited: false,
                state: if self.pending {
                    DeliveryState::Pending
                } else {
                    DeliveryState::Delivered
                },
                queued: self.queued,
                failed_actions: None,
                grouped: self.grouped,
                selected: false,
                density: Density::Compact,
                show_avatar: true,
                avatar: None,
            }
            .render(&theme, &common::en()),
        )
    }
}

fn has_avatar(cx: &mut TestAppContext, grouped: bool) -> bool {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_v, vcx) = cx.add_window_view(move |_, _| Host {
        grouped,
        pending: false,
        queued: false,
    });
    vcx.run_until_parked();
    vcx.debug_bounds("message-avatar").is_some()
}

#[gpui::test]
fn first_message_of_a_group_has_an_avatar(cx: &mut TestAppContext) {
    assert!(has_avatar(cx, false));
}

#[gpui::test]
fn grouped_rows_have_no_avatar(cx: &mut TestAppContext) {
    assert!(!has_avatar(cx, true));
}

type Log = std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>;

struct FailedHost {
    log: Log,
    reason: &'static str,
}

impl Render for FailedHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let handler = |log: &Log, name: &'static str| -> components::message_row::FailedHandler {
            let log = log.clone();
            std::rc::Rc::new(move |_, _| log.borrow_mut().push(name))
        };
        div().child(
            MessageRow {
                author: "Bob".into(),
                status: None,
                author_link: None,
                time: "10:00".into(),
                text: "hello".into(),
                rich: None,
                edited: false,
                state: DeliveryState::Failed(self.reason.into()),
                queued: false,
                failed_actions: Some(components::message_row::FailedActions {
                    key: "1.0".into(),
                    retry: handler(&self.log, "retry"),
                    delete: handler(&self.log, "delete"),
                    copy: handler(&self.log, "copy"),
                }),
                grouped: false,
                selected: false,
                density: Density::Compact,
                show_avatar: true,
                avatar: None,
            }
            .render(&theme, &common::en()),
        )
    }
}

fn failed_window(cx: &mut TestAppContext) -> (Log, &mut gpui::VisualTestContext) {
    failed_window_with(cx, "boom")
}

fn failed_window_with<'a>(
    cx: &'a mut TestAppContext,
    reason: &'static str,
) -> (Log, &'a mut gpui::VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let log: Log = Default::default();
    let host = log.clone();
    let (_v, vcx) = cx.add_window_view(move |_, _| FailedHost { log: host, reason });
    vcx.run_until_parked();
    (log, vcx)
}

#[gpui::test]
fn failed_row_actions_run_on_click(cx: &mut TestAppContext) {
    let (log, vcx) = failed_window(cx);
    for (sel, name) in [
        ("message-failed-retry-1.0", "retry"),
        ("message-failed-delete-1.0", "delete"),
        ("message-failed-copy-1.0", "copy"),
    ] {
        let b = vcx
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} is shown"));
        vcx.simulate_click(b.center(), gpui::Modifiers::none());
        assert_eq!(log.borrow().last().copied(), Some(name));
    }
    assert_eq!(log.borrow().len(), 3);
}

#[gpui::test]
fn failed_row_actions_are_keyboard_reachable(cx: &mut TestAppContext) {
    let (log, vcx) = failed_window(cx);
    for key in ["enter", "space", "enter"] {
        vcx.update(|w, cx| w.focus_next(cx));
        vcx.simulate_keystrokes(key);
    }
    assert_eq!(*log.borrow(), ["retry", "delete", "copy"]);
}

#[gpui::test]
fn restart_failure_warns_the_message_may_have_been_sent(cx: &mut TestAppContext) {
    let (_, vcx) = failed_window_with(cx, domain::RESTART_REASON);
    assert!(vcx.debug_bounds("message-failed-note").is_some());
}

#[gpui::test]
fn other_failures_show_no_may_have_been_sent_note(cx: &mut TestAppContext) {
    let (_, vcx) = failed_window_with(cx, "boom");
    assert!(vcx.debug_bounds("message-failed-note").is_none());
}

#[gpui::test]
fn held_enter_does_not_refire_a_failed_action(cx: &mut TestAppContext) {
    use gpui::{KeyDownEvent, Keystroke};
    let (log, vcx) = failed_window(cx);
    vcx.update(|w, cx| w.focus_next(cx));
    for held in [false, true, true] {
        vcx.simulate_event(KeyDownEvent {
            keystroke: Keystroke::parse("enter").unwrap(),
            is_held: held,
            prefer_character_input: false,
        });
    }
    assert_eq!(*log.borrow(), ["retry"]);
    vcx.simulate_keystrokes("shift-enter");
    assert_eq!(*log.borrow(), ["retry"]);
}

fn queued_label_grouped(cx: &mut TestAppContext, grouped: bool) -> (bool, bool) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_v, vcx) = cx.add_window_view(move |_, _| Host {
        grouped,
        pending: true,
        queued: true,
    });
    vcx.run_until_parked();
    (
        vcx.debug_bounds("message-queued").is_some(),
        vcx.debug_bounds("message-queued-icon").is_some(),
    )
}

#[gpui::test]
fn a_queued_row_shows_the_clock_icon_and_label_grouped_or_not(cx: &mut TestAppContext) {
    assert_eq!(queued_label_grouped(cx, false), (true, true));
    assert_eq!(queued_label_grouped(cx, true), (true, true));
}

#[gpui::test]
fn a_queued_row_is_drawn_at_full_opacity_and_a_plain_pending_row_is_dimmed(
    _cx: &mut TestAppContext,
) {
    use components::message_row::row_opacity;
    assert_eq!(row_opacity(&DeliveryState::Pending, true), 1.0);
    assert!(row_opacity(&DeliveryState::Pending, false) < 1.0);
    assert_eq!(row_opacity(&DeliveryState::Delivered, false), 1.0);
}

fn lin(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn lum(c: gpui::Hsla) -> f32 {
    let c = themes::color::hsla_to_rgba(c);
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

fn ratio(a: gpui::Hsla, b: gpui::Hsla) -> f32 {
    let (x, y) = (lum(a), lum(b));
    let (hi, lo) = if x >= y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

#[gpui::test]
fn queued_text_meets_aa_on_every_row_ground_in_both_themes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        let reg = themes::ThemeRegistry::global(cx);
        for name in [themes::DEFAULT_LIGHT, themes::DEFAULT_DARK] {
            let theme = reg.get(name).unwrap();
            let dark = name == themes::DEFAULT_DARK;
            let fg = components::message_row::queued_color(&theme);
            let c = theme.colors();
            for bg in [c.background, c.surface_background, c.editor_background] {
                let r = ratio(fg, bg);
                println!("queued contrast dark={dark}: {r:.2}:1");
                assert!(r >= 4.5, "dark={dark}: queued text {r:.2}:1");
            }
        }
    });
}

fn queued_label(cx: &mut TestAppContext, pending: bool, queued: bool) -> bool {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_v, vcx) = cx.add_window_view(move |_, _| Host {
        grouped: false,
        pending,
        queued,
    });
    vcx.run_until_parked();
    vcx.debug_bounds("message-queued").is_some()
}

#[gpui::test]
fn a_pending_message_waiting_for_the_connection_says_queued(cx: &mut TestAppContext) {
    assert!(queued_label(cx, true, true));
}

#[gpui::test]
fn a_pending_message_while_online_does_not_say_queued(cx: &mut TestAppContext) {
    assert!(!queued_label(cx, true, false));
}

#[gpui::test]
fn a_delivered_message_never_says_queued(cx: &mut TestAppContext) {
    assert!(!queued_label(cx, false, true));
}

#[gpui::test]
fn a_pending_rows_rich_colours_are_faded_over_its_ground(cx: &mut TestAppContext) {
    use components::RichColors;
    use components::message_row::rich_colors;
    cx.update(|cx| {
        themes::init_default(cx);
        let reg = themes::ThemeRegistry::global(cx);
        for name in [themes::DEFAULT_LIGHT, themes::DEFAULT_DARK] {
            let theme = reg.get(name).unwrap();
            let base = RichColors::from_theme(&theme);
            // The timeline and the thread panel paint messages on this ground.
            let ground = theme.colors().editor_background;
            let pending = rich_colors(base, &DeliveryState::Pending, false, ground);
            assert_eq!(
                pending.link,
                themes::color::faded(base.link, ground, 0.5),
                "{name}: a pending row's links fade with the row"
            );
            assert_ne!(pending.link, base.link, "{name}: link at full strength");
            assert_eq!(
                pending.muted,
                themes::color::faded(base.muted, ground, 0.5),
                "{name}: muted context fades too"
            );
            assert_eq!(
                pending.mention_bg,
                themes::color::faded(base.mention_bg, ground, 0.5),
                "{name}: the mention ground fades"
            );
            // A queued row and a delivered one keep the full-strength colours.
            assert_eq!(
                rich_colors(base, &DeliveryState::Pending, true, ground),
                base,
                "{name}: a queued row is not dimmed"
            );
            assert_eq!(
                rich_colors(base, &DeliveryState::Delivered, false, ground),
                base,
                "{name}: a delivered row is not dimmed"
            );
        }
    });
}
