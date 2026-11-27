use components::composer::{
    Bold, BulletList, Code, CodeBlock, Italic, Link, NumberedList, Quote, Strike,
};
use components::format_toolbar::{Format, format_edit};
use components::text_input::SelectAll;
use components::{Composer, ComposerSource};
use domain::test_support::{cid, uid};
use domain::{Capabilities, ComposedSpan, ConversationId, Inline, MessageWindow, UserId};
use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};

struct Src {
    me: UserId,
    caps: Capabilities,
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
}

fn setup(cx: &mut TestAppContext) -> (Entity<Composer<Src>>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let src = cx.new(|_| Src {
        me: uid("u0"),
        caps: Capabilities::default(),
    });
    let (view, vcx) = cx.add_window_view(move |_w, cx| Composer::new(src, cid("C1"), cx));
    vcx.update(|w, cx| {
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx);
    });
    (view, vcx)
}

fn text(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> String {
    v.read_with(vcx, |c, cx| c.text(cx).to_string())
}

fn select(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext, r: std::ops::Range<usize>) {
    let input = v.read_with(vcx, |c, _| c.input().clone());
    vcx.update(|_, cx| input.update(cx, |i, cx| i.select(r, cx)));
}

fn sel(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> std::ops::Range<usize> {
    v.read_with(vcx, |c, cx| c.input().read(cx).selection().range())
}

#[gpui::test]
fn bold_wraps_selection(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("hello world");
    select(&v, vcx, 6..11);
    vcx.dispatch_action(Bold);
    assert_eq!(text(&v, vcx), "hello *world*");
    assert_eq!(sel(&v, vcx), 7..12);
    // Again: the markers come off.
    vcx.dispatch_action(Bold);
    assert_eq!(text(&v, vcx), "hello world");
    assert_eq!(sel(&v, vcx), 6..11);
}

#[gpui::test]
fn bold_without_selection_places_caret_inside(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("ab ");
    vcx.dispatch_action(Bold);
    assert_eq!(text(&v, vcx), "ab **");
    assert_eq!(sel(&v, vcx), 4..4);
    vcx.simulate_input("x");
    assert_eq!(text(&v, vcx), "ab *x*");
}

#[gpui::test]
fn inline_markers(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("w");
    vcx.dispatch_action(SelectAll);
    vcx.dispatch_action(Italic);
    assert_eq!(text(&v, vcx), "_w_");
    vcx.dispatch_action(Italic);
    vcx.dispatch_action(Strike);
    assert_eq!(text(&v, vcx), "~w~");
    vcx.dispatch_action(Strike);
    vcx.dispatch_action(Code);
    assert_eq!(text(&v, vcx), "`w`");
}

#[gpui::test]
fn code_block_fences_the_selection(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("let x = 1;");
    vcx.dispatch_action(SelectAll);
    vcx.dispatch_action(CodeBlock);
    assert_eq!(text(&v, vcx), "```\nlet x = 1;\n```");
    assert_eq!(sel(&v, vcx), 4..14);

    let (v, vcx) = setup(cx);
    vcx.dispatch_action(CodeBlock);
    assert_eq!(text(&v, vcx), "```\n\n```");
    assert_eq!(sel(&v, vcx), 4..4);
}

#[gpui::test]
fn quote_and_list_prefix_lines(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("a");
    vcx.dispatch_action(components::Newline);
    vcx.simulate_input("b");
    vcx.dispatch_action(SelectAll);
    vcx.dispatch_action(Quote);
    assert_eq!(text(&v, vcx), "> a\n> b");
    vcx.dispatch_action(Quote);
    assert_eq!(text(&v, vcx), "a\nb");
    vcx.dispatch_action(BulletList);
    assert_eq!(text(&v, vcx), "\u{2022} a\n\u{2022} b");
    vcx.dispatch_action(BulletList);
    assert_eq!(text(&v, vcx), "a\nb");
}

#[gpui::test]
fn quote_on_the_caret_line_only(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("one");
    vcx.dispatch_action(components::Newline);
    vcx.simulate_input("two");
    select(&v, vcx, 1..1);
    vcx.dispatch_action(Quote);
    assert_eq!(text(&v, vcx), "> one\ntwo");
    assert_eq!(sel(&v, vcx), 3..3);
    // A line break right after the selection is not part of it.
    select(&v, vcx, 0..6);
    vcx.dispatch_action(Quote);
    assert_eq!(text(&v, vcx), "one\ntwo");
}

#[gpui::test]
fn shortcut_keys_apply_formats(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("a");
    vcx.dispatch_action(SelectAll);
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    vcx.simulate_keystrokes(&format!("{m}-b"));
    assert_eq!(text(&v, vcx), "*a*");
}

#[gpui::test]
fn link_inserts_span(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("see docs now");
    select(&v, vcx, 4..8);
    vcx.dispatch_action(Link);
    assert!(v.read_with(vcx, |c, _| c.link_prompt_open()));
    assert!(vcx.update(|_, cx| v.update(cx, |c, cx| c.apply_link("example.com/a", cx))));
    assert!(!v.read_with(vcx, |c, _| c.link_prompt_open()));
    let t = v.read_with(vcx, |c, cx| c.composed(cx));
    assert_eq!(&*t.text, "see docs now");
    assert_eq!(
        t.spans,
        [ComposedSpan {
            range: 4..8,
            inline: Inline::Link {
                url: "https://example.com/a".into(),
                label: Some("docs".into()),
                style: Default::default(),
            },
        }]
    );
}

#[gpui::test]
fn link_without_selection_inserts_the_url(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("go ");
    vcx.dispatch_action(Link);
    assert!(vcx.update(|_, cx| v.update(cx, |c, cx| c.apply_link("https://example.com", cx))));
    let t = v.read_with(vcx, |c, cx| c.composed(cx));
    assert_eq!(&*t.text, "go https://example.com");
    assert_eq!(t.spans.len(), 1);
    assert_eq!(t.spans[0].range, 3..22);
    assert!(matches!(
        &t.spans[0].inline,
        Inline::Link { url, label: None, .. } if &**url == "https://example.com"
    ));
}

#[gpui::test]
fn link_refuses_unsafe_schemes(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("x");
    vcx.dispatch_action(SelectAll);
    vcx.dispatch_action(Link);
    for bad in ["javascript:alert(1)", "  ", "file:///etc/passwd"] {
        assert!(
            !vcx.update(|_, cx| v.update(cx, |c, cx| c.apply_link(bad, cx))),
            "{bad}"
        );
    }
    assert!(v.read_with(vcx, |c, cx| c.composed(cx).spans.is_empty()));
}

#[test]
fn format_edit_is_pure() {
    let e = format_edit(Format::Bold, "ab cd", 3..5);
    assert_eq!(e.apply_to("ab cd"), "ab *cd*");
    assert_eq!(e.select, 4..6);
    let e = format_edit(Format::Quote, "x", 1..1);
    assert_eq!(e.apply_to("x"), "> x");
    assert_eq!(e.select, 3..3);
}

#[gpui::test]
fn formatting_keeps_mentions_and_undoes_in_one_step(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("hi ");
    let input = v.read_with(vcx, |c, _| c.input().clone());
    vcx.update(|_, cx| {
        input.update(cx, |i, cx| {
            i.insert_span(
                3..3,
                "@Ana",
                Inline::User {
                    id: uid("U1"),
                    label: None,
                },
                cx,
            )
        })
    });
    vcx.dispatch_action(SelectAll);
    vcx.dispatch_action(Bold);
    let t = v.read_with(vcx, |c, cx| c.composed(cx));
    assert_eq!(&*t.text, "*hi @Ana*");
    assert_eq!(t.spans.len(), 1);
    assert_eq!(t.spans[0].range, 4..8);
    vcx.dispatch_action(components::text_input::Undo);
    let t = v.read_with(vcx, |c, cx| c.composed(cx));
    assert_eq!(&*t.text, "hi @Ana");
    assert_eq!(t.spans[0].range, 3..7);
}

#[gpui::test]
fn set_composed_restores_spans_as_units(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    let draft = domain::ComposedText::new(
        "ask @Ana",
        vec![ComposedSpan {
            range: 4..8,
            inline: Inline::User {
                id: uid("U1"),
                label: None,
            },
        }],
    );
    vcx.update(|_, cx| v.update(cx, |c, cx| c.set_composed(&draft, cx)));
    assert_eq!(v.read_with(vcx, |c, cx| c.composed(cx)), draft);
    // One backspace removes the whole mention.
    vcx.simulate_keystrokes("backspace");
    assert_eq!(text(&v, vcx), "ask ");
}

#[gpui::test]
fn composer_shortcuts_stay_out_of_the_focused_link_field(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("keep");
    vcx.dispatch_action(Link);
    assert!(v.read_with(vcx, |c, _| c.link_prompt_open()));
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("https://x.test".into()));
    vcx.dispatch_action(components::composer::Paste);
    vcx.dispatch_action(components::composer::Newline);
    vcx.dispatch_action(Bold);
    assert_eq!(text(&v, vcx), "keep");
}

#[gpui::test]
fn numbered_list_prefixes_lines_and_toggles(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("a");
    vcx.dispatch_action(components::Newline);
    vcx.simulate_input("b");
    vcx.dispatch_action(SelectAll);
    vcx.dispatch_action(NumberedList);
    assert_eq!(text(&v, vcx), "1. a\n2. b");
    vcx.dispatch_action(NumberedList);
    assert_eq!(text(&v, vcx), "a\nb");
    let e = format_edit(Format::NumberedList, "x", 1..1);
    assert_eq!(e.apply_to("x"), "1. x");
    // Three lines count up; any `N. ` prefix toggles the list off, numbers of two digits too.
    let t = "a\nb\nc";
    let on = format_edit(Format::NumberedList, t, 0..t.len());
    assert_eq!(on.apply_to(t), "1. a\n2. b\n3. c");
    let t2 = "9. a\n10. b\n1. c";
    let off = format_edit(Format::NumberedList, t2, 0..t2.len());
    assert_eq!(off.apply_to(t2), "a\nb\nc");
    assert_eq!(off.select, 0..5);
    assert!(Format::ALL.contains(&Format::NumberedList));
}

#[gpui::test]
fn numbered_list_shortcut_is_cmd_shift_7(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("a");
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    vcx.simulate_keystrokes(&format!("{m}-shift-7"));
    assert_eq!(text(&v, vcx), "1. a");
}

#[gpui::test]
fn numbered_list_button_is_drawn_and_formats(cx: &mut TestAppContext) {
    let (v, vcx) = setup(cx);
    vcx.simulate_input("a");
    vcx.run_until_parked();
    let b = vcx
        .debug_bounds("fmt-numbered")
        .expect("numbered list button");
    vcx.simulate_click(b.center(), gpui::Modifiers::default());
    assert_eq!(text(&v, vcx), "1. a");
}
