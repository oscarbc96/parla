//! Composer emoji button (CO-03) and edit mode keeping mention and link spans (CO-09).

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use components::{Composer, ComposerEvent, ComposerSource, EmojiPicked, Send, TimelineEvent};
use domain::test_support::{cid, mid, uid};
use domain::{
    Capabilities, ComposedSpan, ComposedText, ConversationId, EmojiRef, Inline, MessageWindow,
    UserId,
};
use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};

struct Src {
    me: UserId,
    caps: Capabilities,
    used: Used,
}

impl ComposerSource for Src {
    fn window(&self, _: &ConversationId) -> Option<&MessageWindow> {
        None
    }
    fn self_user(&self) -> Option<&UserId> {
        Some(&self.me)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn emoji_used(&mut self, e: EmojiRef, _: &mut gpui::App) {
        self.used.borrow_mut().push(e.shortcode.to_string());
    }
    fn decode(&self, markup: &str) -> ComposedText {
        match markup.strip_prefix("ask <@U1> see <https://x.test|docs>") {
            Some("") => ComposedText::new(
                "ask @Ana see docs",
                vec![
                    ComposedSpan {
                        range: 4..8,
                        inline: Inline::User {
                            id: uid("U1"),
                            label: None,
                        },
                    },
                    ComposedSpan {
                        range: 13..17,
                        inline: Inline::Link {
                            url: "https://x.test".into(),
                            label: Some("docs".into()),
                            style: Default::default(),
                        },
                    },
                ],
            ),
            _ => ComposedText::plain(markup),
        }
    }
}

type Events = Rc<RefCell<Vec<ComposerEvent>>>;
type Used = Rc<RefCell<Vec<String>>>;

fn setup(cx: &mut TestAppContext) -> (Entity<Composer<Src>>, Events, Used, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let used = Used::default();
    let u = used.clone();
    let src = cx.new(move |_| Src {
        me: uid("u0"),
        caps: Capabilities::default(),
        used: u,
    });
    let (view, vcx) = cx.add_window_view(move |_w, cx| Composer::new(src, cid("C1"), cx));
    let events: Events = Rc::default();
    let sink = events.clone();
    vcx.update(|w, cx| {
        cx.subscribe(&view, move |_, e: &ComposerEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach();
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx);
    });
    (view, events, used, vcx)
}

fn text(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> String {
    v.read_with(vcx, |c, cx| c.text(cx).to_string())
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.run_until_parked();
    let b = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    vcx.simulate_click(b.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
}

fn smile() -> EmojiRef {
    EmojiRef {
        shortcode: "smile".into(),
        unicode: Some("\u{1f604}".into()),
        skin_tone: None,
    }
}

fn pick(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext, e: EmojiRef) {
    let p = v
        .read_with(vcx, |c, _| c.emoji_picker().cloned())
        .expect("picker is open");
    vcx.update(|_, cx| p.update(cx, |_, cx| cx.emit(EmojiPicked(e))));
    vcx.run_until_parked();
}

#[gpui::test]
fn emoji_button_opens_the_picker_and_a_pick_inserts_at_the_caret(cx: &mut TestAppContext) {
    let (v, _e, used, vcx) = setup(cx);
    vcx.simulate_input("ab");
    vcx.simulate_keystrokes("left");
    click(vcx, "composer-emoji");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_some()));
    pick(&v, vcx, smile());
    assert_eq!(text(&v, vcx), "a\u{1f604}b");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_none()));
    assert_eq!(*used.borrow(), ["smile"]);
    // The caret sits after the emoji, in the input.
    vcx.simulate_input("x");
    assert_eq!(text(&v, vcx), "a\u{1f604}xb");
}

#[gpui::test]
fn a_custom_emoji_is_inserted_as_a_token_span(cx: &mut TestAppContext) {
    let (v, _e, _u, vcx) = setup(cx);
    vcx.simulate_input("hi ");
    click(vcx, "composer-emoji");
    let party = EmojiRef {
        shortcode: "party".into(),
        unicode: None,
        skin_tone: None,
    };
    pick(&v, vcx, party.clone());
    assert_eq!(text(&v, vcx), "hi :party:");
    let c = v.read_with(vcx, |c, cx| c.composed(cx));
    assert_eq!(c.spans.len(), 1);
    assert_eq!(c.spans[0].range, 3..10);
    assert_eq!(c.spans[0].inline, Inline::Emoji(party));
}

#[gpui::test]
fn escape_closes_the_picker_and_returns_focus_to_the_input(cx: &mut TestAppContext) {
    let (v, _e, used, vcx) = setup(cx);
    vcx.simulate_input("ab");
    click(vcx, "composer-emoji");
    let picker_focused = vcx.update(|w, cx| {
        v.read(cx)
            .emoji_picker()
            .is_some_and(|p| p.focus_handle(cx).is_focused(w))
    });
    assert!(picker_focused, "the picker takes focus");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_none()));
    assert_eq!(text(&v, vcx), "ab");
    assert!(used.borrow().is_empty());
    let focused = vcx.update(|w, cx| v.read(cx).focus_handle(cx).is_focused(w));
    assert!(focused, "typing after Escape goes to the input");
    vcx.simulate_input("c");
    assert_eq!(text(&v, vcx), "abc");
}

#[gpui::test]
fn the_emoji_button_toggles_the_picker(cx: &mut TestAppContext) {
    let (v, _e, _u, vcx) = setup(cx);
    click(vcx, "composer-emoji");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_some()));
    click(vcx, "composer-emoji");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_none()));
}

#[gpui::test]
fn the_thread_composer_has_the_emoji_button_too(cx: &mut TestAppContext) {
    let (v, _e, _u, vcx) = setup(cx);
    vcx.update(|_, cx| v.update(cx, |c, cx| c.set_reply_root(Some(mid("C1", "1.1")), cx)));
    click(vcx, "composer-emoji");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_some()));
}

#[gpui::test]
fn editing_keeps_mention_and_link_spans(cx: &mut TestAppContext) {
    let (v, ev, _u, vcx) = setup(cx);
    let id = mid("C1", "1000.000001");
    vcx.update(|_, cx| {
        v.update(cx, |c, cx| {
            c.handle_timeline_event(
                &TimelineEvent::EditRequested {
                    id: id.clone(),
                    text: "ask <@U1> see <https://x.test|docs>".into(),
                },
                cx,
            )
        })
    });
    assert_eq!(text(&v, vcx), "ask @Ana see docs");
    let c = v.read_with(vcx, |c, cx| c.composed(cx));
    assert_eq!(c.spans.len(), 2);
    // The mention is one unit: a backspace after it removes it whole.
    vcx.update(|_, cx| {
        let input = v.read(cx).input().clone();
        input.update(cx, |i, cx| i.select(8..8, cx));
    });
    vcx.simulate_keystrokes("backspace");
    assert_eq!(text(&v, vcx), "ask  see docs");
    // Sending the edit hands the surviving link span on.
    vcx.dispatch_action(Send);
    let got = ev.borrow().clone();
    let [ComposerEvent::SubmitEdit { id: got_id, body }] = got.as_slice() else {
        panic!("one SubmitEdit, got {got:?}");
    };
    assert_eq!(got_id, &id);
    assert_eq!(body.spans.len(), 1);
    assert!(matches!(body.spans[0].inline, Inline::Link { .. }));
}

#[gpui::test]
fn an_unchanged_edit_sends_no_update(cx: &mut TestAppContext) {
    let (v, ev, _u, vcx) = setup(cx);
    let id = mid("C1", "1000.000001");
    let begin = |vcx: &mut VisualTestContext| {
        vcx.update(|_, cx| {
            v.update(cx, |c, cx| {
                c.handle_timeline_event(
                    &TimelineEvent::EditRequested {
                        id: id.clone(),
                        text: "ask <@U1> see <https://x.test|docs>".into(),
                    },
                    cx,
                )
            })
        });
    };
    begin(vcx);
    vcx.dispatch_action(Send);
    assert_eq!(*ev.borrow(), [ComposerEvent::Cancel], "nothing to update");
    assert!(v.read_with(vcx, |c, _| c.editing().is_none()));
    ev.borrow_mut().clear();
    // A real change still goes out.
    begin(vcx);
    vcx.simulate_input("!");
    vcx.dispatch_action(Send);
    assert!(matches!(
        ev.borrow().as_slice(),
        [ComposerEvent::SubmitEdit { .. }]
    ));
}

#[gpui::test]
fn composer_keys_do_not_edit_the_input_while_the_picker_has_focus(cx: &mut TestAppContext) {
    use components::composer::{Bold, NumberedList};
    let (v, _e, _u, vcx) = setup(cx);
    vcx.simulate_input("ab");
    click(vcx, "composer-emoji");
    vcx.dispatch_action(Bold);
    vcx.dispatch_action(NumberedList);
    vcx.dispatch_action(components::Newline);
    vcx.simulate_keystrokes("tab");
    assert_eq!(text(&v, vcx), "ab");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_some()));
}

#[gpui::test]
fn a_press_outside_closes_the_picker_and_the_button_does_not_reopen_it(cx: &mut TestAppContext) {
    let (v, _e, _u, vcx) = setup(cx);
    click(vcx, "composer-emoji");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_some()));
    vcx.simulate_click(
        gpui::point(gpui::px(1.), gpui::px(1.)),
        gpui::Modifiers::default(),
    );
    vcx.run_until_parked();
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_none()));
    // With it open again, a press on the button itself closes it for good.
    click(vcx, "composer-emoji");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_some()));
    click(vcx, "composer-emoji");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_none()));
    // And the next press opens it again (the guard does not stick).
    click(vcx, "composer-emoji");
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_some()));
}

#[gpui::test]
fn changing_conversation_closes_the_picker(cx: &mut TestAppContext) {
    let (v, _e, _u, vcx) = setup(cx);
    click(vcx, "composer-emoji");
    v.update(vcx, |c, cx| c.set_conversation(cid("C2"), cx));
    assert!(v.read_with(vcx, |c, _| c.emoji_picker().is_none()));
}

#[gpui::test]
fn the_picker_is_capped_in_height_and_inside_the_window(cx: &mut TestAppContext) {
    let (_v, _e, _u, vcx) = setup(cx);
    click(vcx, "composer-emoji");
    let p = vcx.debug_bounds("composer-emoji-picker").expect("picker");
    assert!(
        p.size.height <= themes::metrics::LAYOUT_POPOVER_MAX_HEIGHT + themes::metrics::SPACE_2,
        "{p:?}"
    );
    let w = vcx.update(|w, _| w.bounds());
    assert!(
        p.origin.y >= gpui::px(0.) && p.bottom() <= w.size.height,
        "{p:?} in {w:?}"
    );
    assert!(vcx.debug_bounds("picker-grid").is_some());
}

#[gpui::test]
fn the_action_row_follows_the_board_order(cx: &mut TestAppContext) {
    let (_v, _e, _u, vcx) = setup(cx);
    vcx.run_until_parked();
    let mut b = |s: &'static str| vcx.debug_bounds(s).unwrap_or_else(|| panic!("{s} drawn"));
    let (attach, emoji, later, send) = (
        b("composer-attach"),
        b("composer-emoji"),
        b("composer-send-later"),
        b("composer-send"),
    );
    assert!(attach.right() <= emoji.left() && emoji.right() <= later.left());
    assert!(later.right() <= send.left());
    // Same size, same line.
    assert_eq!(attach.size, emoji.size);
    assert_eq!(emoji.size, later.size);
    assert!((attach.center().y - send.center().y).abs() < gpui::px(4.));
}

struct Narrow(Entity<Composer<Src>>);

impl gpui::Render for Narrow {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _, div};
        div().w(gpui::px(270.)).child(self.0.clone())
    }
}

#[gpui::test]
fn a_narrow_composer_keeps_the_toolbar_and_the_action_row_on_one_line(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let src = cx.new(|_| Src {
        me: uid("u0"),
        caps: Capabilities::default(),
        used: Used::default(),
    });
    let (_h, vcx) =
        cx.add_window_view(move |_w, cx| Narrow(cx.new(|cx| Composer::new(src, cid("C1"), cx))));
    vcx.run_until_parked();
    let frame = vcx.debug_bounds("composer-frame").expect("frame");
    let link = vcx.debug_bounds("fmt-link").expect("link");
    let bold = vcx.debug_bounds("fmt-bold").expect("bold");
    assert!(
        link.right() <= frame.right(),
        "Link is clipped: {link:?} in {frame:?}"
    );
    assert_eq!(link.top(), bold.top(), "the toolbar wrapped");
    let send = vcx.debug_bounds("composer-send").expect("send");
    let attach = vcx.debug_bounds("composer-attach").expect("attach");
    assert!(
        send.top() < attach.bottom(),
        "the action row wrapped under Send"
    );
    assert!(send.right() <= frame.right());
}

#[gpui::test]
fn the_thread_broadcast_option_is_a_check_box_not_bracket_text(cx: &mut TestAppContext) {
    let (v, _e, _used, vcx) = setup(cx);
    v.update(vcx, |c, cx| {
        c.set_reply_root(Some(mid("C1", "1000.000001")), cx)
    });
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("composer-broadcast-checkbox").is_some(),
        "the option draws the settings check box"
    );
    let loc = common::en();
    let label = loc.format("components", "composer-broadcast", None);
    assert!(
        !label.contains('[') && !label.contains(']'),
        "composer-broadcast renders a literal bracket: {label}"
    );
}

#[gpui::test]
fn the_thread_broadcast_check_box_toggles_on_space(cx: &mut TestAppContext) {
    let (v, _e, _used, vcx) = setup(cx);
    v.update(vcx, |c, cx| {
        c.set_reply_root(Some(mid("C1", "1000.000001")), cx)
    });
    vcx.run_until_parked();
    assert!(!v.read_with(vcx, |c, _| c.broadcast()));
    // Focus the check box, then toggle it with the keyboard.
    let b = vcx
        .debug_bounds("composer-broadcast")
        .expect("broadcast drawn");
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert!(v.read_with(vcx, |c, _| c.broadcast()), "click toggles on");
    vcx.simulate_keystrokes("space");
    vcx.run_until_parked();
    assert!(!v.read_with(vcx, |c, _| c.broadcast()), "space toggles off");
}
