//! The snippet card: language label, line numbers, first lines with Expand and
//! Collapse, Copy, and the bounds on what one card holds.

mod common;

use std::sync::Arc;

use components::files::{Card, card, card_with};
use components::snippet::{self, COLLAPSED_LINES, MAX_BYTES, MAX_LINES};
use components::{Timeline, TimelineSource};
use domain::test_support::{cid, member, msg, uid};
use domain::{
    Capabilities, ConversationId, FileId, FileKind, FileRef, HistoryPage, Input, Member, Message,
    MessageId, MessageWindow, ProviderExt, UserId, WorkspaceState, reduce,
};
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualTestContext};
use settings::{Clock, InlineImages, WorkspaceKey};

fn snippet_file(name: &str, lines: usize) -> FileRef {
    let preview: String = (1..=lines).map(|i| format!("line {i}\n")).collect();
    FileRef {
        id: FileId(Arc::from("F1")),
        name: Arc::from(name),
        title: None,
        mime: Arc::from("text/plain"),
        size: preview.len() as u64,
        kind: FileKind::Snippet,
        thumbs: Arc::from([]),
        download: None,
        preview: Some(Arc::from(preview.as_str())),
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

fn snippet_of(f: &FileRef, expanded: bool) -> snippet::SnippetCard {
    match card_with(f, InlineImages::Always, false, expanded) {
        Card::Snippet(s) => s,
        other => panic!("{other:?}"),
    }
}

#[test]
fn collapsed_card_shows_the_first_lines_with_the_language() {
    let f = snippet_file("main.rs", 30);
    let s = snippet_of(&f, false);
    assert_eq!(s.language.map(|l| l.label), Some("Rust"));
    assert_eq!(s.lines.len(), 30);
    assert_eq!(s.shown().len(), COLLAPSED_LINES);
    assert_eq!(s.shown()[0], "line 1");
    assert_eq!(s.hidden(), 30 - COLLAPSED_LINES);
    // `card` is the collapsed card.
    assert!(matches!(
        card(&f, InlineImages::Always, false),
        Card::Snippet(c) if !c.expanded
    ));
}

#[test]
fn expanded_card_shows_every_line() {
    let s = snippet_of(&snippet_file("main.rs", 30), true);
    assert_eq!(s.shown().len(), 30);
}

#[test]
fn a_short_snippet_has_nothing_to_expand() {
    let s = snippet_of(&snippet_file("a.py", 3), false);
    assert_eq!(s.hidden(), 0);
    assert_eq!(s.language.map(|l| l.id), Some("python"));
}

#[test]
fn unknown_extension_has_no_language_label() {
    assert!(
        snippet_of(&snippet_file("notes.zzz", 3), false)
            .language
            .is_none()
    );
}

#[test]
fn copy_text_is_every_line_joined() {
    let s = snippet_of(&snippet_file("a.txt", 3), false);
    assert_eq!(s.text(), "line 1\nline 2\nline 3");
}

#[test]
fn a_card_never_holds_more_than_the_line_and_byte_caps() {
    let s = snippet_of(&snippet_file("big.txt", MAX_LINES + 50), true);
    assert_eq!(s.lines.len(), MAX_LINES);
    assert!(s.partial);
    let wide = "x".repeat(1000);
    let text: String = (0..200).map(|_| format!("{wide}\n")).collect();
    let (bounded, cut) = snippet::bound(&text);
    assert!(cut && bounded.len() <= MAX_BYTES && bounded.ends_with('\n'));
}

struct Src {
    state: WorkspaceState,
}

impl TimelineSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }
    fn capabilities(&self) -> &Capabilities {
        static CAPS: std::sync::OnceLock<Capabilities> = std::sync::OnceLock::new();
        CAPS.get_or_init(Capabilities::default)
    }
    fn load_older(&mut self, _: ConversationId, _: &mut App) {}
    fn edit(&mut self, _: MessageId, _: String, _: &mut App) {}
    fn delete(&mut self, _: MessageId, _: &mut App) {}
    fn react(&mut self, _: MessageId, _: domain::EmojiRef, _: bool, _: &mut App) {}
    fn custom_emoji(&self) -> Vec<domain::CustomEmoji> {
        Vec::new()
    }
}

fn setup(cx: &mut TestAppContext, lines: usize) -> (Entity<Timeline<Src>>, &mut VisualTestContext) {
    setup_with(cx, lines, None)
}

fn setup_with(
    cx: &mut TestAppContext,
    lines: usize,
    total: Option<u32>,
) -> (Entity<Timeline<Src>>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    state.self_user = Some(uid("u1"));
    state.members.insert(uid("u2"), member("u2", "Bob"));
    let mut m: Message = msg("C1", "1000.000001", "shared a snippet");
    let mut f = snippet_file("main.rs", lines);
    f.lines = total;
    m.files = Arc::from(vec![f]);
    reduce(
        &mut state,
        Input::HistoryLoaded {
            conversation: cid("C1"),
            page: HistoryPage {
                messages: vec![m],
                has_more: false,
            },
        },
    );
    let src = cx.new(move |_| Src { state });
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    (view, vcx)
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    let at = vcx.debug_bounds(selector).expect(selector).center();
    vcx.simulate_click(at, gpui::Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
}

#[gpui::test]
fn timeline_draws_numbered_lines_and_expands_and_collapses(cx: &mut TestAppContext) {
    let (_view, vcx) = setup(cx, 30);
    let card = vcx.debug_bounds("snippet-card").expect("snippet card");
    assert!(vcx.debug_bounds("snippet-line-8").is_some(), "8th line");
    assert!(
        vcx.debug_bounds("snippet-line-9").is_none(),
        "9th is hidden"
    );
    click(vcx, "snippet-toggle");
    let open = vcx.debug_bounds("snippet-card").unwrap();
    assert!(
        open.size.height > card.size.height,
        "expanded card is taller"
    );
    assert!(vcx.debug_bounds("snippet-line-30").is_some(), "last line");
    click(vcx, "snippet-toggle");
    assert!(
        vcx.debug_bounds("snippet-line-9").is_none(),
        "collapsed again"
    );
}

#[gpui::test]
fn a_short_snippet_has_no_expand_button(cx: &mut TestAppContext) {
    let (_view, vcx) = setup(cx, 3);
    assert!(vcx.debug_bounds("snippet-card").is_some());
    assert!(vcx.debug_bounds("snippet-toggle").is_none());
}

#[gpui::test]
fn copy_puts_the_snippet_on_the_clipboard(cx: &mut TestAppContext) {
    let (_view, vcx) = setup(cx, 3);
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("stale".into()));
    click(vcx, "snippet-copy");
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()),
        Some("line 1\nline 2\nline 3".to_string())
    );
}

#[test]
fn language_is_guessed_for_simple_cases_and_defaults_to_plain_text() {
    let id = |t: &str| snippet::detect_language(t).id;
    assert_eq!(id("#!/usr/bin/env python3\nprint(1)"), "python");
    assert_eq!(id("#!/bin/bash\necho hi"), "shell");
    assert_eq!(id("pub fn main() {\n}"), "rust");
    assert_eq!(id("{\n  \"a\": 1\n}"), "json");
    assert_eq!(id("diff --git a/x b/x\n--- a/x"), "diff");
    assert_eq!(id("def f():\n    pass"), "python");
    assert_eq!(id("function f() {}"), "javascript");
    assert_eq!(id("SELECT 1 FROM t"), "sql");
    assert_eq!(id("just some prose\nacross lines"), "text");
}

#[test]
fn snippet_file_name_uses_the_title_or_a_default() {
    let rust = snippet::language_by_id("rust").unwrap();
    assert_eq!(snippet::file_name("", rust), "snippet.rs");
    assert_eq!(snippet::file_name("check", rust), "check.rs");
    assert_eq!(snippet::file_name("check.py", rust), "check.py");
    assert_eq!(snippet::file_name("../a/b", rust), ".._a_b.rs");
}

#[test]
fn a_preview_shorter_than_the_file_is_partial_and_states_the_whole_count() {
    let mut f = snippet_file("main.rs", 5);
    f.lines = Some(40);
    let s = snippet_of(&f, false);
    assert!(s.partial, "the provider's preview is not the whole file");
    assert_eq!(s.line_count(), 40);
    assert_eq!(s.lines.len(), 5);
    // The whole file in the preview: not partial.
    let mut whole = snippet_file("main.rs", 5);
    whole.lines = Some(5);
    assert!(!snippet_of(&whole, false).partial);
}

#[test]
fn highlighting_colours_keywords_strings_comments_numbers_and_calls() {
    use components::snippet_highlight::{Class, highlight};
    let rust = snippet::language_by_id("rust").unwrap();
    let line = r#"let n = parse("a", 42); // done"#;
    let got: Vec<_> = highlight(rust, line)
        .into_iter()
        .map(|(r, c)| (&line[r], c))
        .collect();
    assert_eq!(
        got,
        [
            ("let", Class::Keyword),
            ("parse", Class::Function),
            ("\"a\"", Class::String),
            ("42", Class::Number),
            ("// done", Class::Comment),
        ]
    );
    let py = snippet::language_by_id("python").unwrap();
    assert_eq!(highlight(py, "x = 'y'  # c").len(), 2);
    // Work per line is bounded.
    let long = "a ".repeat(components::snippet_highlight::MAX_LINE_BYTES);
    assert!(
        highlight(rust, &long)
            .iter()
            .all(|(r, _)| r.end <= components::snippet_highlight::MAX_LINE_BYTES)
    );
}

#[test]
fn a_paste_is_classified_with_bounded_work() {
    use snippet::{MAX_SNIPPET_BYTES, Paste, classify_paste};
    assert_eq!(classify_paste("short"), Paste::Short);
    assert_eq!(
        classify_paste(&"x".repeat(MAX_SNIPPET_BYTES + 1)),
        Paste::TooLarge
    );
    match classify_paste(&"y\n".repeat(500)) {
        Paste::Snippet { lines, .. } => assert_eq!(lines, 500),
        other => panic!("{other:?}"),
    }
    // Detection reads only the start: Rust after 200 lines of prose is not seen.
    let late = format!("{}pub fn main() {{}}\n", "prose\n".repeat(300));
    assert_eq!(snippet::detect_language(&late).id, "text");
}

#[gpui::test]
fn a_partial_snippet_says_so_in_the_card(cx: &mut TestAppContext) {
    let (_view, vcx) = setup_with(cx, 5, Some(40));
    assert!(vcx.debug_bounds("snippet-cut-note").is_some());
    assert!(vcx.debug_bounds("snippet-copy").is_some());
}

#[gpui::test]
fn a_whole_snippet_has_no_cut_note(cx: &mut TestAppContext) {
    let (_view, vcx) = setup_with(cx, 5, Some(5));
    assert!(vcx.debug_bounds("snippet-cut-note").is_none());
}

#[gpui::test]
fn repaints_reuse_the_built_card(cx: &mut TestAppContext) {
    let (view, vcx) = setup(cx, 30);
    let built = view.read_with(vcx, |t, _| t.snippet_card_builds());
    assert_eq!(built, 1);
    for _ in 0..3 {
        view.update(vcx, |_, cx| cx.notify());
        vcx.update(|w, cx| w.draw(cx).clear(cx));
    }
    click(vcx, "snippet-toggle");
    view.read_with(vcx, |t, _| {
        assert_eq!(
            t.snippet_card_builds(),
            1,
            "no rebuild on repaint or expand"
        );
        assert!(t.snippet_cache_len() <= 1);
    });
}
