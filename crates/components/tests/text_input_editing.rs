//! Standard text-field editing keys: delete to line start and end, word
//! right, Cocoa control keys and insert-key clipboard keys.

#[cfg(target_os = "macos")]
use components::text_input::Selection;
use components::text_input::TextInput;
use gpui::{Entity, TestAppContext, VisualTestContext};

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

fn text(i: &Entity<TextInput>, vcx: &mut VisualTestContext) -> String {
    i.read_with(vcx, |t, _| t.text().to_string())
}

#[cfg(target_os = "macos")]
fn sel(i: &Entity<TextInput>, vcx: &mut VisualTestContext) -> Selection {
    i.read_with(vcx, |t, _| t.selection())
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn cmd_backspace_deletes_only_the_current_line(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| {
        t.set_text("one\ntwo\nthree", cx);
        t.select(7..7, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-backspace");
    assert_eq!(text(&i, vcx), "one\n\nthree");
    assert_eq!(sel(&i, vcx), Selection { anchor: 4, head: 4 });
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn cmd_backspace_at_a_line_start_joins_the_lines(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| {
        t.set_text("one\ntwo", cx);
        t.select(4..4, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-backspace");
    assert_eq!(text(&i, vcx), "onetwo");
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn cmd_backspace_with_a_selection_deletes_only_it(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| {
        t.set_text("one\ntwo\nthree", cx);
        t.select(4..7, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-backspace");
    assert_eq!(text(&i, vcx), "one\n\nthree");
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn cmd_backspace_on_a_wrapped_row_keeps_the_other_rows(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    let all = "word ".repeat(400);
    i.update(vcx, |t, cx| {
        t.set_text(&all, cx);
        t.select(0..0, cx);
    });
    vcx.run_until_parked();
    // The start of the second visual row.
    vcx.simulate_keystrokes("down");
    let row_start = sel(&i, vcx).head;
    assert!(row_start > 0, "the text wraps");
    for _ in 0..3 {
        vcx.simulate_keystrokes("right");
    }
    let caret = sel(&i, vcx).head;
    vcx.simulate_keystrokes("cmd-backspace");
    let after = text(&i, vcx);
    assert_eq!(after.len(), all.len() - (caret - row_start));
    assert_eq!(&after[..row_start], &all[..row_start]);
    assert_eq!(&after[row_start..], &all[caret..]);
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn cmd_delete_deletes_to_the_end_of_the_line(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| {
        t.set_text("one\ntwo\nthree", cx);
        t.select(4..4, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-delete");
    assert_eq!(text(&i, vcx), "one\n\nthree");
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn ctrl_k_deletes_to_the_end_of_the_line(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| {
        t.set_text("one\ntwo\nthree", cx);
        t.select(5..5, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("ctrl-k");
    assert_eq!(text(&i, vcx), "one\nt\nthree");
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn alt_delete_deletes_the_word_on_the_right(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| {
        t.set_text("hello big world", cx);
        t.select(5..5, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("alt-delete");
    assert_eq!(text(&i, vcx), "hello world");
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn cocoa_control_keys_move_like_a_text_view(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| t.set_text("one\ntwo", cx));
    vcx.run_until_parked();
    // End of the second line.
    assert_eq!(sel(&i, vcx).head, 7);
    vcx.simulate_keystrokes("ctrl-a");
    assert_eq!(sel(&i, vcx), Selection { anchor: 4, head: 4 });
    vcx.simulate_keystrokes("ctrl-e");
    assert_eq!(sel(&i, vcx), Selection { anchor: 7, head: 7 });
    vcx.simulate_keystrokes("ctrl-p");
    assert_eq!(sel(&i, vcx).head, 3);
    vcx.simulate_keystrokes("ctrl-n");
    assert_eq!(sel(&i, vcx).head, 7);
    vcx.simulate_keystrokes("ctrl-b");
    assert_eq!(sel(&i, vcx).head, 6);
    vcx.simulate_keystrokes("ctrl-f");
    assert_eq!(sel(&i, vcx).head, 7);
    vcx.simulate_keystrokes("ctrl-b");
    assert_eq!(sel(&i, vcx).head, 6);
    vcx.simulate_keystrokes("ctrl-d");
    assert_eq!(text(&i, vcx), "one\ntw");
    vcx.simulate_keystrokes("ctrl-h");
    assert_eq!(text(&i, vcx), "one\nt");
}

#[gpui::test]
fn shift_backspace_deletes_the_character_before_the_caret(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    vcx.simulate_input("hello");
    vcx.simulate_keystrokes("shift-backspace");
    assert_eq!(text(&i, vcx), "hell");
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn undo_restores_a_delete_to_line_start(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| {
        t.set_text("one\ntwo\nthree", cx);
        t.select(7..7, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-backspace");
    assert_eq!(text(&i, vcx), "one\n\nthree");
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&i, vcx), "one\ntwo\nthree");
    assert_eq!(sel(&i, vcx), Selection { anchor: 7, head: 7 });
}

#[gpui::test]
fn shift_delete_cuts_and_ctrl_insert_copies_on_linux_and_windows(cx: &mut TestAppContext) {
    if cfg!(target_os = "macos") {
        return;
    }
    let (i, vcx) = setup(cx);
    i.update(vcx, |t, cx| {
        t.set_text("hello", cx);
        t.select(1..4, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("ctrl-insert");
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("ell")
    );
    vcx.simulate_keystrokes("shift-delete");
    assert_eq!(text(&i, vcx), "ho");
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("ey".into()));
    vcx.simulate_keystrokes("shift-insert");
    assert_eq!(text(&i, vcx), "heyo");
}

#[gpui::test]
fn the_input_handles_typing_after_the_new_keys(cx: &mut TestAppContext) {
    let (i, vcx) = setup(cx);
    vcx.simulate_input("ab");
    vcx.simulate_keystrokes("shift-backspace");
    vcx.simulate_input("c");
    assert_eq!(text(&i, vcx), "ac");
}
