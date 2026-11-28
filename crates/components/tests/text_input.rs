//! Composer text input: IME composition, UTF-16 ranges, selection, clipboard, undo, mouse.

use components::text_input::{
    Backspace, Copy, Cut, DeleteWordLeft, DocEnd, Down, Left, LineEnd, Paste, Redo, SelectAll,
    SelectLeft, SelectWordLeft, Selection, TextInput, TextInputEvent, Undo, Up, WordLeft,
    WordRight,
};
use gpui::{
    Bounds, ClipboardItem, Entity, EntityInputHandler as _, Modifiers, Pixels, TestAppContext,
    VisualTestContext, point, px,
};
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

fn setup(cx: &mut TestAppContext) -> (Entity<TextInput>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (input, vcx) = cx.add_window_view(|_w, cx| TextInput::new(cx));
    vcx.update(|w, cx| {
        let handle = input.read(cx).focus_handle_for_test();
        w.focus(&handle, cx);
    });
    (input, vcx)
}

struct FailFetch;

impl components::media::MediaFetcher for FailFetch {
    fn fetch(
        &self,
        _: &domain::ImageRef,
        _: u64,
    ) -> futures::future::BoxFuture<'static, Result<Vec<u8>, components::media::MediaError>> {
        Box::pin(std::future::ready(Err(
            components::media::MediaError::Network("offline".into()),
        )))
    }
}

/// A custom emoji `:partyparrot:` (13 bytes) inserted as a span over `range`.
fn custom_span(i: &Entity<TextInput>, vcx: &mut VisualTestContext, range: Range<usize>) {
    i.update(vcx, |t, cx| {
        t.insert_span(
            range,
            ":partyparrot:",
            domain::Inline::Emoji(domain::EmojiRef {
                shortcode: "partyparrot".into(),
                unicode: None,
                skin_tone: None,
            }),
            cx,
        )
    });
}

fn setup_with_images(cx: &mut TestAppContext) -> (Entity<TextInput>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        let dir = std::env::temp_dir().join(format!("parla-text-input-{}", std::process::id()));
        cx.set_global(components::media::ImageCache::new(
            std::sync::Arc::new(FailFetch),
            dir,
            1 << 20,
        ));
    });
    let (input, vcx) = cx.add_window_view(|_w, cx| TextInput::new(cx));
    vcx.update(|w, cx| {
        let handle = input.read(cx).focus_handle_for_test();
        w.focus(&handle, cx);
    });
    input.update(vcx, |t, _| {
        t.set_span_images(Rc::new(|inline, _| match inline {
            domain::Inline::Emoji(e) if e.unicode.is_none() => Some(domain::ImageRef::Web(
                format!("https://example.invalid/{}.png", e.shortcode).into(),
            )),
            _ => None,
        }))
    });
    (input, vcx)
}

fn caret(i: &Entity<TextInput>, vcx: &mut VisualTestContext, at: usize) -> Bounds<Pixels> {
    let b = input_bounds(vcx);
    vcx.update(|w, cx| i.update(cx, |t, cx| t.bounds_for_range(at..at, b, w, cx)))
        .expect("caret bounds")
}

#[gpui::test]
fn a_custom_emoji_span_paints_one_placeholder(cx: &mut TestAppContext) {
    let (i, vcx) = setup_with_images(cx);
    i.update(vcx, |t, cx| t.set_text("hi ", cx));
    custom_span(&i, vcx, 3..3);
    i.update(vcx, |t, cx| t.replace_range(16..16, "!", cx));
    vcx.run_until_parked();
    assert_eq!(text(&i, vcx), "hi :partyparrot:!");
    let start = caret(&i, vcx, 3);
    let end = caret(&i, vcx, 16);
    // One placeholder, not the thirteen bytes of `:partyparrot:`.
    let width = end.left() - start.left();
    assert!(width > px(0.0) && width < px(40.0), "{width:?}");
    // The buffer, spans and hit testing still speak the real text.
    assert_eq!(i.read_with(vcx, |t, _| t.spans()[0].range.clone()), 3..16);
    let b = input_bounds(vcx);
    let after = vcx
        .update(|w, cx| {
            i.update(cx, |t, cx| {
                t.character_index_for_point(point(end.left(), b.top() + px(6.0)), w, cx)
            })
        })
        .expect("index after the image");
    assert_eq!(after, 16);
}

#[gpui::test]
fn a_custom_emoji_span_deletes_whole(cx: &mut TestAppContext) {
    let (i, vcx) = setup_with_images(cx);
    i.update(vcx, |t, cx| t.set_text("hi ", cx));
    custom_span(&i, vcx, 3..3);
    vcx.run_until_parked();
    i.update(vcx, |t, cx| t.select(16..16, cx));
    vcx.dispatch_action(Backspace);
    assert_eq!(text(&i, vcx), "hi ");
    assert!(i.read_with(vcx, |t, _| t.spans().is_empty()));
}

#[gpui::test]
fn without_a_resolver_a_span_paints_its_text(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| t.set_text("hi ", cx));
    custom_span(&i, vcx, 3..3);
    vcx.run_until_parked();
    let start = caret(&i, vcx, 3);
    let end = caret(&i, vcx, 16);
    // The whole token is laid out as text.
    assert!(end.left() - start.left() > px(60.0));
    assert_eq!(text(&i, vcx), "hi :partyparrot:");
}

fn text(i: &Entity<TextInput>, vcx: &mut VisualTestContext) -> String {
    i.read_with(vcx, |t, _| t.text().to_string())
}

fn sel(i: &Entity<TextInput>, vcx: &mut VisualTestContext) -> Selection {
    i.read_with(vcx, |t, _| t.selection())
}

fn mark(i: &Entity<TextInput>, vcx: &mut VisualTestContext, s: &str) {
    vcx.update(|w, cx| {
        i.update(cx, |t, cx| {
            t.replace_and_mark_text_in_range(None, s, None, w, cx)
        })
    });
}

fn commit(i: &Entity<TextInput>, vcx: &mut VisualTestContext, s: &str) {
    vcx.update(|w, cx| i.update(cx, |t, cx| t.replace_text_in_range(None, s, w, cx)));
}

#[gpui::test]
fn ime_marked_text_then_commit(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    mark(&i, vcx, "にほ");
    assert_eq!(text(&i, vcx), "にほ");
    assert!(i.read_with(vcx, |t, _| t.is_composing()));
    let marked = vcx.update(|w, cx| i.update(cx, |t, cx| t.marked_text_range(w, cx)));
    assert_eq!(marked, Some(0..2));
    commit(&i, vcx, "日本");
    assert_eq!(text(&i, vcx), "日本");
    assert!(!i.read_with(vcx, |t, _| t.is_composing()));
    let marked = vcx.update(|w, cx| i.update(cx, |t, cx| t.marked_text_range(w, cx)));
    assert_eq!(marked, None);
}

#[gpui::test]
fn dead_key_accent_replaces_the_marked_accent(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    vcx.simulate_input("c");
    mark(&i, vcx, "´");
    assert_eq!(text(&i, vcx), "c´");
    commit(&i, vcx, "á");
    assert_eq!(text(&i, vcx), "cá");
    assert_eq!(sel(&i, vcx), Selection { anchor: 3, head: 3 });
}

#[gpui::test]
fn unmark_commits_the_composition_as_typed(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    mark(&i, vcx, "ab");
    vcx.update(|w, cx| i.update(cx, |t, cx| t.unmark_text(w, cx)));
    assert!(!i.read_with(vcx, |t, _| t.is_composing()));
    assert_eq!(text(&i, vcx), "ab");
}

#[gpui::test]
fn utf16_ranges_with_emoji_and_cjk(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| t.set_text("a👍日", cx));
    let mut adjusted = None;
    let got =
        vcx.update(|w, cx| i.update(cx, |t, cx| t.text_for_range(1..3, &mut adjusted, w, cx)));
    assert_eq!(got.as_deref(), Some("👍"));
    assert_eq!(adjusted, Some(1..3));
    // Bytes 1..5 are the emoji; the platform sees UTF-16 1..3.
    i.update(vcx, |t, cx| t.select(1..5, cx));
    let s = vcx
        .update(|w, cx| i.update(cx, |t, cx| t.selected_text_range(false, w, cx)))
        .unwrap();
    assert_eq!((s.range, s.reversed), (1..3, false));
    // A range in the middle of the surrogate pair snaps to a char boundary.
    vcx.update(|w, cx| i.update(cx, |t, cx| t.replace_text_in_range(Some(1..3), "x", w, cx)));
    assert_eq!(text(&i, vcx), "ax日");
    vcx.update(|w, cx| i.update(cx, |t, cx| t.replace_text_in_range(Some(2..3), "ー", w, cx)));
    assert_eq!(text(&i, vcx), "axー");
}

#[gpui::test]
fn shift_arrows_extend_selection(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    vcx.simulate_input("hello");
    vcx.dispatch_action(SelectLeft);
    vcx.dispatch_action(SelectLeft);
    assert_eq!(sel(&i, vcx), Selection { anchor: 5, head: 3 });
    vcx.dispatch_action(Left);
    assert_eq!(sel(&i, vcx), Selection { anchor: 3, head: 3 });
    vcx.simulate_keystrokes("shift-left shift-left");
    assert_eq!(sel(&i, vcx), Selection { anchor: 3, head: 1 });
}

#[gpui::test]
fn word_moves_and_delete_word(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    vcx.simulate_input("hello big world");
    vcx.dispatch_action(WordLeft);
    assert_eq!(sel(&i, vcx).head, 10);
    vcx.dispatch_action(WordLeft);
    assert_eq!(sel(&i, vcx).head, 6);
    vcx.dispatch_action(WordRight);
    assert_eq!(sel(&i, vcx).head, 9);
    vcx.dispatch_action(DocEnd);
    vcx.dispatch_action(DeleteWordLeft);
    assert_eq!(text(&i, vcx), "hello big ");
    vcx.dispatch_action(SelectWordLeft);
    assert_eq!(
        sel(&i, vcx),
        Selection {
            anchor: 10,
            head: 6
        }
    );
}

#[gpui::test]
fn typing_replaces_selection(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    vcx.simulate_input("hello");
    vcx.dispatch_action(SelectAll);
    vcx.simulate_input("x");
    assert_eq!(text(&i, vcx), "x");
    vcx.simulate_input("yz");
    vcx.dispatch_action(SelectLeft);
    vcx.dispatch_action(Backspace);
    assert_eq!(text(&i, vcx), "xy");
}

#[gpui::test]
fn cut_copy_paste_plain(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    vcx.simulate_input("hello");
    i.update(vcx, |t, cx| t.select(1..4, cx));
    vcx.dispatch_action(Copy);
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("ell")
    );
    assert_eq!(text(&i, vcx), "hello");
    vcx.dispatch_action(Cut);
    assert_eq!(text(&i, vcx), "ho");
    vcx.dispatch_action(DocEnd);
    vcx.dispatch_action(Paste);
    assert_eq!(text(&i, vcx), "hoell");
    // Rich clipboard text pastes as plain text, and a lone caret copies nothing.
    vcx.write_to_clipboard(ClipboardItem::new_string("A\nB".into()));
    vcx.dispatch_action(Paste);
    assert_eq!(text(&i, vcx), "hoellA\nB");
    vcx.write_to_clipboard(ClipboardItem::new_string("keep".into()));
    vcx.dispatch_action(Copy);
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("keep")
    );
}

#[gpui::test]
fn undo_redo_coalesces_typing(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    vcx.simulate_input("abc");
    vcx.dispatch_action(Undo);
    assert_eq!(text(&i, vcx), "");
    vcx.dispatch_action(Redo);
    assert_eq!(text(&i, vcx), "abc");
    // Moving the caret ends a typing run.
    vcx.simulate_keystrokes("left");
    vcx.simulate_input("x");
    assert_eq!(text(&i, vcx), "abxc");
    vcx.dispatch_action(Undo);
    assert_eq!(text(&i, vcx), "abc");
    vcx.dispatch_action(Undo);
    assert_eq!(text(&i, vcx), "");
    // A new edit drops the redo stack.
    vcx.simulate_input("q");
    vcx.dispatch_action(Redo);
    assert_eq!(text(&i, vcx), "q");
}

#[gpui::test]
fn undo_keeps_at_most_100_entries(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    for n in 0..120 {
        vcx.simulate_input("a");
        vcx.simulate_keystrokes("left right");
        let _ = n;
    }
    for _ in 0..150 {
        vcx.dispatch_action(Undo);
    }
    assert_eq!(text(&i, vcx).len(), 20);
}

#[gpui::test]
fn changed_event_fires_on_edit_only(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    let n = Rc::new(RefCell::new(0));
    let sink = n.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&i, move |_, _: &TextInputEvent, _| *sink.borrow_mut() += 1)
            .detach();
    });
    vcx.simulate_input("ab");
    assert_eq!(*n.borrow(), 2);
    vcx.simulate_keystrokes("left");
    assert_eq!(*n.borrow(), 2);
}

fn input_bounds(vcx: &mut VisualTestContext) -> Bounds<Pixels> {
    vcx.debug_bounds("text-input").expect("text input painted")
}

#[gpui::test]
fn double_click_selects_word_and_triple_click_the_line(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| t.set_text("one two\nthree", cx));
    vcx.run_until_parked();
    let b = input_bounds(vcx);
    // The second word of the first line: left edge plus a few characters, first row.
    let on_two = point(b.left() + px(40.0), b.top() + px(6.0));
    vcx.simulate_click(on_two, Modifiers::none());
    vcx.simulate_event(gpui::MouseDownEvent {
        position: on_two,
        modifiers: Modifiers::none(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    vcx.simulate_event(gpui::MouseUpEvent {
        position: on_two,
        modifiers: Modifiers::none(),
        button: gpui::MouseButton::Left,
        click_count: 2,
    });
    let s = sel(&i, vcx);
    let (a, h) = (s.anchor.min(s.head), s.anchor.max(s.head));
    assert!(a >= 3 && h <= 7 && h > a, "{s:?}");
    vcx.simulate_event(gpui::MouseDownEvent {
        position: on_two,
        modifiers: Modifiers::none(),
        button: gpui::MouseButton::Left,
        click_count: 3,
        first_mouse: false,
    });
    vcx.simulate_event(gpui::MouseUpEvent {
        position: on_two,
        modifiers: Modifiers::none(),
        button: gpui::MouseButton::Left,
        click_count: 3,
    });
    let s = sel(&i, vcx);
    assert_eq!((s.anchor.min(s.head), s.anchor.max(s.head)), (0, 7));
}

#[gpui::test]
fn drag_selects_a_range(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| t.set_text("abcdefgh", cx));
    vcx.run_until_parked();
    let b = input_bounds(vcx);
    let y = b.top() + px(6.0);
    vcx.simulate_mouse_down(
        point(b.left() + px(2.0), y),
        gpui::MouseButton::Left,
        Modifiers::none(),
    );
    vcx.simulate_mouse_move(
        point(b.left() + px(200.0), y),
        gpui::MouseButton::Left,
        Modifiers::none(),
    );
    vcx.simulate_mouse_up(
        point(b.left() + px(200.0), y),
        gpui::MouseButton::Left,
        Modifiers::none(),
    );
    assert_eq!(sel(&i, vcx), Selection { anchor: 0, head: 8 });
}

#[gpui::test]
fn bounds_for_range_inside_element(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| t.set_text("hello", cx));
    vcx.run_until_parked();
    let b = input_bounds(vcx);
    let r = vcx
        .update(|w, cx| i.update(cx, |t, cx| t.bounds_for_range(1..3, b, w, cx)))
        .expect("bounds for the range");
    assert!(b.contains(&r.origin), "{r:?} not in {b:?}");
    assert!(r.size.width > px(0.0));
    let idx =
        vcx.update(|w, cx| i.update(cx, |t, cx| t.character_index_for_point(r.center(), w, cx)));
    assert!(matches!(idx, Some(1..=3)), "{idx:?}");
}

#[gpui::test]
fn up_and_down_move_between_lines(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| t.set_text("ab\ncd", cx));
    vcx.run_until_parked();
    vcx.dispatch_action(Up);
    assert_eq!(sel(&i, vcx).head, 2);
    vcx.dispatch_action(Down);
    assert_eq!(sel(&i, vcx).head, 5);
}

#[gpui::test]
fn bounds_for_empty_range_is_the_caret(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| t.set_text("hello", cx));
    vcx.run_until_parked();
    let b = input_bounds(vcx);
    let r = vcx
        .update(|w, cx| i.update(cx, |t, cx| t.bounds_for_range(2..2, b, w, cx)))
        .expect("caret box for an empty range");
    assert!(b.contains(&r.origin), "{r:?} not in {b:?}");
    assert!(r.size.height > px(0.0));
}

#[gpui::test]
fn masked_input_never_copies_or_reveals_its_value(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, _| t.set_masked(true));
    commit(&i, vcx, "s3cret pass");
    vcx.write_to_clipboard(ClipboardItem::new_string("keep".into()));
    vcx.dispatch_action(SelectAll);
    vcx.dispatch_action(Copy);
    vcx.dispatch_action(Cut);
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("keep")
    );
    assert_eq!(text(&i, vcx), "s3cret pass");
    let read = vcx.update(|w, cx| {
        i.update(cx, |t, cx| {
            let mut adjusted = None;
            t.text_for_range(0..3, &mut adjusted, w, cx)
        })
    });
    assert_eq!(read, None);
}

#[gpui::test]
fn masked_input_edits_and_moves_by_character_without_word_stops(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, _| t.set_masked(true));
    commit(&i, vcx, "ab cd");
    vcx.run_until_parked();
    vcx.dispatch_action(WordLeft);
    assert_eq!(sel(&i, vcx).head, 0);
    vcx.dispatch_action(WordRight);
    assert_eq!(sel(&i, vcx).head, 5);
    vcx.dispatch_action(Left);
    vcx.dispatch_action(Backspace);
    assert_eq!(text(&i, vcx), "ab d");
    vcx.dispatch_action(DeleteWordLeft);
    assert_eq!(text(&i, vcx), "d");
}

#[gpui::test]
fn single_line_input_flattens_pasted_newlines(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, _| t.set_single_line(true));
    vcx.write_to_clipboard(ClipboardItem::new_string("a\nb\r\nc\n".into()));
    vcx.dispatch_action(Paste);
    assert_eq!(text(&i, vcx), "a b c");
}

#[gpui::test]
fn masked_clicks_map_bullets_back_to_the_real_text(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| {
        t.set_masked(true);
        t.set_text("h\u{e9}llo", cx);
    });
    vcx.run_until_parked();
    let b = input_bounds(vcx);
    // Far right of the field: the caret lands at the end of the real text (6 bytes).
    vcx.simulate_click(
        point(b.right() - px(2.0), b.top() + px(6.0)),
        Modifiers::none(),
    );
    assert_eq!(sel(&i, vcx).head, 6);
    // Far left: the start.
    vcx.simulate_click(
        point(b.left() + px(1.0), b.top() + px(6.0)),
        Modifiers::none(),
    );
    assert_eq!(sel(&i, vcx).head, 0);
    // A bullet is narrower than the text, so a click inside lands on a char boundary.
    vcx.simulate_click(
        point(b.left() + px(20.0), b.top() + px(6.0)),
        Modifiers::none(),
    );
    let h = sel(&i, vcx).head;
    assert!("h\u{e9}llo".is_char_boundary(h), "{h}");
}

/// A paragraph that soft-wraps in the test window.
fn wrapped(cx: &mut TestAppContext) -> (Entity<TextInput>, &mut VisualTestContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| t.set_text(&"word ".repeat(400), cx));
    vcx.run_until_parked();
    (i, vcx)
}

#[gpui::test]
fn end_on_a_wrapped_row_stays_on_that_row(cx: &mut TestAppContext) {
    let (i, vcx) = wrapped(cx);
    // Where Down lands, from the start, is the start of the third row.
    i.update(vcx, |t, cx| t.select(0..0, cx));
    vcx.dispatch_action(Down);
    vcx.dispatch_action(Down);
    let third_row = sel(&i, vcx).head;
    i.update(vcx, |t, cx| t.select(0..0, cx));
    vcx.run_until_parked();

    vcx.dispatch_action(LineEnd);
    let end_of_first = sel(&i, vcx).head;
    assert!(
        end_of_first > 0 && end_of_first < third_row,
        "{end_of_first}"
    );
    // The caret sits at the end of the first row, not the start of the second.
    let caret = i.read_with(vcx, |t, _| t.caret_row_for_test());
    assert_eq!(caret, Some(0));
    // A second End does not jump rows.
    vcx.dispatch_action(LineEnd);
    assert_eq!(sel(&i, vcx).head, end_of_first);
    // Down moves exactly one row: into the second, not past it.
    vcx.dispatch_action(Down);
    let head = sel(&i, vcx).head;
    assert!(
        head > end_of_first && head <= third_row,
        "{head} vs {third_row}"
    );
    assert_eq!(i.read_with(vcx, |t, _| t.caret_row_for_test()), Some(1));
    // Up goes back to the first row.
    vcx.dispatch_action(Up);
    assert_eq!(i.read_with(vcx, |t, _| t.caret_row_for_test()), Some(0));
}
