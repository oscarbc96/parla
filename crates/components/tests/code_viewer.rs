//! The code viewer's model: which files it opens, the document's lines, find
//! with next and previous, and the snippet card's Open in viewer button.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::code_viewer::{
    CodeDoc, Find, MAX_CODE_BYTES, MAX_DRAWN_LINE_BYTES, MAX_MATCHES, is_code, merge_styles,
    whole_preview,
};
use components::files_tab::{FileOpen, open_for};
use components::media_viewer::Gallery;
use components::{Timeline, TimelineSource};
use domain::test_support::{cid, member, msg, uid};
use domain::{
    Capabilities, ConversationId, FileHit, FileId, FileKind, FileRef, HistoryPage, Input, Member,
    Message, MessageId, MessageWindow, ProviderExt, RemoteUrl, UserId, WorkspaceState, reduce,
};
use gpui::{App, AppContext as _, HighlightStyle, TestAppContext};
use settings::{Clock, WorkspaceKey};

fn file(name: &str, mime: &str, kind: FileKind, size: u64) -> FileRef {
    FileRef {
        id: FileId(Arc::from("F1")),
        name: Arc::from(name),
        title: None,
        mime: Arc::from(mime),
        size,
        kind,
        thumbs: Arc::from([]),
        download: Some(RemoteUrl(Arc::from("https://files.example/F1/dl"))),
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

#[test]
fn snippets_source_files_and_text_open_in_the_viewer_images_and_binaries_do_not() {
    assert!(is_code(&file("a.rs", "text/plain", FileKind::Snippet, 10)));
    assert!(is_code(&file(
        "theme_loader.rs",
        "application/octet-stream",
        FileKind::Other,
        6000
    )));
    assert!(is_code(&file("notes", "text/plain", FileKind::Other, 10)));
    assert!(!is_code(&file("a.png", "image/png", FileKind::Image, 10)));
    assert!(!is_code(&file(
        "a.zip",
        "application/zip",
        FileKind::Other,
        10
    )));
    assert!(
        !is_code(&file(
            "huge.rs",
            "text/plain",
            FileKind::Other,
            MAX_CODE_BYTES + 1
        )),
        "too large to show"
    );
    let mut gone = file("a.rs", "text/plain", FileKind::Snippet, 10);
    gone.tombstone = true;
    assert!(!is_code(&gone));
}

#[test]
fn a_document_splits_lines_drops_line_ends_and_knows_its_language() {
    let d = CodeDoc::new("fn a() {}\r\n\nlet b = 1;\n", "main.rs");
    assert_eq!(d.line_count(), 3);
    assert_eq!(d.line(0), "fn a() {}");
    assert_eq!(d.line(1), "");
    assert_eq!(d.line(2), "let b = 1;");
    assert_eq!(d.language().map(|l| l.label), Some("Rust"));
    assert_eq!(d.text(), "fn a() {}\r\n\nlet b = 1;\n");
    assert_eq!(CodeDoc::new("", "a.txt").line_count(), 1, "one empty line");
    assert_eq!(CodeDoc::new("x", "a.zzz").language(), None);
}

#[test]
fn the_longest_line_sizes_the_unwrapped_view() {
    let d = CodeDoc::new("a\nlongest line\nmid\n", "a.txt");
    assert_eq!(d.longest_line(), 1);
}

#[test]
fn bytes_that_are_not_text_are_refused() {
    assert!(CodeDoc::from_bytes(b"\x00\x01\x02binary".to_vec(), "a.bin").is_none());
    let d = CodeDoc::from_bytes("caf\u{e9}\n".as_bytes().to_vec(), "a.txt").unwrap();
    assert_eq!(d.line(0), "caf\u{e9}");
    let bad = CodeDoc::from_bytes(b"ok \xff\n".to_vec(), "a.txt").unwrap();
    assert_eq!(bad.line(0), "ok \u{fffd}", "invalid UTF-8 is replaced");
}

#[test]
fn fetched_text_is_kept_in_place_and_copy_shares_it() {
    let bytes = b"fn a() {}\nfn b() {}\n".to_vec();
    let at = bytes.as_ptr();
    let d = CodeDoc::from_bytes(bytes, "a.rs").unwrap();
    assert_eq!(d.text().as_ptr(), at, "decoded in place, not copied");
    assert_eq!(d.shared_text().as_ptr(), at, "Copy shares the same buffer");
}

#[test]
fn a_preview_of_unknown_size_and_lines_may_be_partial() {
    let mut f = file("a.rs", "text/plain", FileKind::Snippet, 0);
    f.preview = Some(Arc::from("fn a() {}\n"));
    assert_eq!(whole_preview(&f), None, "size 0 and no line count: fetch");
    f.size = 10;
    assert!(whole_preview(&f).is_some());
    f.size = 0;
    f.lines = Some(1);
    assert!(
        whole_preview(&f).is_some(),
        "the line count says it is whole"
    );
}

#[test]
fn highlighting_reaches_the_drawn_length() {
    let pad = " ".repeat(3_000);
    let d = CodeDoc::new(format!("{pad}fn x() {{}}\n"), "a.rs");
    assert!(d.drawn_line(0).len() > 3_000 && MAX_DRAWN_LINE_BYTES > 3_000);
    assert!(
        d.spans(0).iter().any(|(r, _)| r.start >= 3_000),
        "a keyword past 2,000 bytes is coloured"
    );
}

#[test]
fn find_searches_whole_lines_past_the_drawn_length() {
    let d = CodeDoc::new(format!("{}needle\nneedle\n", "a".repeat(5_000)), "a.txt");
    let mut f = Find::default();
    f.search(&d, "needle");
    assert_eq!(f.len(), 2);
    assert_eq!(
        f.current().map(|m| (m.line, m.range.start)),
        Some((0, 5_000))
    );
}

#[test]
fn a_capped_search_starts_where_the_view_is() {
    let n = MAX_MATCHES + 50;
    let d = CodeDoc::new("a\n".repeat(n), "a.txt");
    let mut f = Find::default();
    f.search_from(&d, "a", n - 10);
    assert!(f.capped());
    assert_eq!(f.len(), MAX_MATCHES);
    assert_eq!(
        f.current().map(|m| m.line),
        Some(n - 10),
        "no jump to the top"
    );
    let lines: Vec<usize> = (0..f.len()).map(|_| f.step_next().unwrap().line).collect();
    assert!(lines.contains(&0), "wraps round to the top");
}

#[test]
fn a_fifty_thousand_line_file_is_indexed_not_copied_per_line() {
    let text: String = (0..50_000).map(|i| format!("line {i}\n")).collect();
    let d = CodeDoc::new(text, "big.txt");
    assert_eq!(d.line_count(), 50_000);
    assert_eq!(d.line(49_999), "line 49999");
}

#[test]
fn find_lists_every_match_ignoring_ascii_case_and_steps_with_wrap() {
    let d = CodeDoc::new("Theme a\nno\ntheme theme\n", "a.txt");
    let mut f = Find::default();
    f.search(&d, "theme");
    assert_eq!(f.len(), 3);
    assert_eq!(f.current_index(), Some(0));
    assert_eq!(
        f.current().map(|m| (m.line, m.range.clone())),
        Some((0, 0..5))
    );
    assert_eq!(f.on_line(2).len(), 2);
    assert!(f.on_line(1).is_empty());
    assert_eq!(f.step_next().map(|m| (m.line, m.range.start)), Some((2, 0)));
    assert_eq!(f.step_next().map(|m| (m.line, m.range.start)), Some((2, 6)));
    assert_eq!(f.step_next().map(|m| m.line), Some(0), "wraps to the first");
    assert_eq!(
        f.step_prev().map(|m| (m.line, m.range.start)),
        Some((2, 6)),
        "wraps back"
    );
    f.search(&d, "");
    assert_eq!(f.len(), 0);
    assert_eq!(f.current(), None);
}

#[test]
fn find_stops_counting_at_the_cap() {
    let text = "a\n".repeat(MAX_MATCHES + 10);
    let d = CodeDoc::new(text, "a.txt");
    let mut f = Find::default();
    f.search(&d, "a");
    assert_eq!(f.len(), MAX_MATCHES);
    assert!(f.capped());
}

#[test]
fn find_starts_at_the_first_match_at_or_after_a_line() {
    let d = CodeDoc::new("x\ny\nx\nx\n", "a.txt");
    let mut f = Find::default();
    f.search_from(&d, "x", 2);
    assert_eq!(f.current().map(|m| m.line), Some(2));
    f.search_from(&d, "x", 9);
    assert_eq!(
        f.current().map(|m| m.line),
        Some(0),
        "none after: the first"
    );
}

#[test]
fn match_marks_split_syntax_colours_without_overlap() {
    let red = HighlightStyle {
        color: Some(gpui::red()),
        ..Default::default()
    };
    let mark = HighlightStyle {
        background_color: Some(gpui::yellow()),
        ..Default::default()
    };
    let out = merge_styles(&[(0..6, red)], &[(4..9, mark)]);
    let ranges: Vec<_> = out.iter().map(|(r, _)| r.clone()).collect();
    assert_eq!(ranges, vec![0..4, 4..6, 6..9]);
    assert_eq!(out[0].1.background_color, None);
    assert_eq!(out[1].1.color, Some(gpui::red()));
    assert_eq!(out[1].1.background_color, Some(gpui::yellow()));
    assert_eq!(out[2].1.color, None);
}

fn hit(f: FileRef) -> FileHit {
    FileHit {
        file: f,
        conversation: Some(cid("C1")),
        author: Some(uid("u2")),
        ts: Some(domain::MessageTs(Arc::from("1000.000001"))),
    }
}

#[test]
fn a_code_file_in_the_files_tab_opens_the_code_viewer() {
    let hits = vec![hit(file(
        "theme_loader.rs",
        "text/plain",
        FileKind::Other,
        6000,
    ))];
    match open_for(&hits, 0, |_| "Bob".into()) {
        Some(FileOpen::Code(g)) => {
            assert_eq!(&*g.current().unwrap().file.name, "theme_loader.rs");
        }
        _ => panic!("expected the code viewer"),
    }
}

struct Src {
    state: WorkspaceState,
    opened: Rc<RefCell<Vec<String>>>,
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
    fn download(&mut self, f: &FileRef, _: &mut App) {
        self.opened
            .borrow_mut()
            .push(format!("download {}", f.name));
    }
    fn open_code(&mut self, g: Gallery, _: &mut App) {
        let it = g.current().unwrap();
        self.opened
            .borrow_mut()
            .push(format!("code {} from {}", it.file.name, it.sender));
    }
}

fn timeline_with(
    cx: &mut TestAppContext,
    f: FileRef,
) -> (Rc<RefCell<Vec<String>>>, &mut gpui::VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    state.self_user = Some(uid("u1"));
    state.members.insert(uid("u2"), member("u2", "Bob"));
    let mut m: Message = msg("C1", "1000.000001", "have a look");
    m.author = Some(uid("u2"));
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
    let opened = Rc::new(RefCell::new(Vec::new()));
    let o = opened.clone();
    let src = cx.new(move |_| Src { state, opened: o });
    let (_view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    (opened, vcx)
}

#[gpui::test]
fn the_snippet_cards_open_in_viewer_button_opens_the_code_viewer(cx: &mut TestAppContext) {
    let mut f = file("main.rs", "text/plain", FileKind::Snippet, 20);
    f.preview = Some(Arc::from("fn main() {}\n"));
    let (opened, vcx) = timeline_with(cx, f);
    let at = vcx
        .debug_bounds("snippet-open")
        .expect("open button")
        .center();
    vcx.simulate_click(at, gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(*opened.borrow(), ["code main.rs from Bob"]);
}

#[gpui::test]
fn clicking_a_code_file_card_opens_the_code_viewer_not_a_download(cx: &mut TestAppContext) {
    let f = file("theme_loader.rs", "text/plain", FileKind::Other, 6000);
    let (opened, vcx) = timeline_with(cx, f);
    let at = vcx
        .debug_bounds("file-card-F1")
        .expect("file card")
        .center();
    vcx.simulate_click(at, gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(*opened.borrow(), ["code theme_loader.rs from Bob"]);
}
