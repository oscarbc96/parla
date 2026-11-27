//! The conversation's Files tab: the files shared in it, newest first, with
//! the loading, error and empty states, keyboard moves, and rows that open in
//! the media viewer or download.

mod common;

use std::collections::VecDeque;
use std::sync::Arc;

use components::media_viewer::Gallery;
use components::{FilesDone, FilesLoad, FilesSource, FilesTab};
use domain::test_support::{cid, member, ts, uid};
use domain::{
    ConversationId, FileHit, FileId, FileKind, FileRef, FixedNow, Page, ProviderExt, RemoteUrl,
    Rendition, WorkspaceState,
};
use gpui::{App, AppContext as _, Entity, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use settings::{InlineImages, WorkspaceKey};

/// 2026-11-15 10:00:00 UTC.
const NOW: i64 = 1_794_736_800;

const ROWS: [&str; 4] = ["files-row-0", "files-row-1", "files-row-2", "files-row-3"];

struct Src {
    state: WorkspaceState,
    /// Answers given to the next loads, in order; none left holds the load.
    pages: VecDeque<Result<Page<FileHit>, String>>,
    /// The cursor and limit of each load asked for.
    calls: Vec<(Option<Arc<str>>, usize)>,
    /// Loads not answered yet.
    held: Vec<FilesDone>,
    /// File ids of each gallery opened, and the one shown.
    viewed: Vec<(Vec<String>, String)>,
    downloaded: Vec<String>,
    inline: InlineImages,
}

impl FilesSource for Src {
    fn files_state(&self) -> &WorkspaceState {
        &self.state
    }
    fn load_files(
        &mut self,
        _c: ConversationId,
        cursor: Option<Arc<str>>,
        limit: usize,
        done: FilesDone,
        cx: &mut App,
    ) {
        self.calls.push((cursor, limit));
        match self.pages.pop_front() {
            Some(r) => cx.defer(move |cx| done(r, cx)),
            None => self.held.push(done),
        }
    }
    fn view_file(&mut self, g: Gallery, _cx: &mut App) {
        let ids = g.items().iter().map(|i| i.file.id.0.to_string()).collect();
        let shown = g.current().map(|i| i.file.id.0.to_string()).unwrap();
        self.viewed.push((ids, shown));
    }
    fn download_file(&mut self, f: &FileRef, _cx: &mut App) {
        self.downloaded.push(f.id.0.to_string());
    }
    fn files_inline_images(&self, _cx: &App) -> InlineImages {
        self.inline
    }
}

fn file(id: &str, name: &str, mime: &str, kind: FileKind) -> FileRef {
    let thumbs: Arc<[Rendition]> = if kind == FileKind::Image {
        Arc::from([Rendition {
            url: RemoteUrl(Arc::from(format!("https://files.test/{id}_80.png"))),
            width: 80,
            height: 80,
        }])
    } else {
        Arc::from([])
    };
    FileRef {
        id: FileId(Arc::from(id)),
        name: Arc::from(name),
        title: None,
        mime: Arc::from(mime),
        size: 51_200,
        kind,
        thumbs,
        download: Some(RemoteUrl(Arc::from(format!("https://files.test/{id}")))),
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

fn hit(f: FileRef, author: &str, t: &str) -> FileHit {
    FileHit {
        file: f,
        conversation: Some(cid("C1")),
        author: Some(uid(author)),
        ts: Some(ts(t)),
    }
}

/// Newest first: an image, a PDF, a binary and an older image.
fn sample() -> Vec<FileHit> {
    vec![
        hit(
            file("F4", "mockup.png", "image/png", FileKind::Image),
            "u2",
            "1794650400.000100",
        ),
        hit(
            file("F3", "report-q3.pdf", "application/pdf", FileKind::Pdf),
            "u3",
            "1794564000.000100",
        ),
        hit(
            file(
                "F2",
                "export.bin",
                "application/octet-stream",
                FileKind::Other,
            ),
            "u2",
            "1794477600.000100",
        ),
        hit(
            file("F1", "screenshot.png", "image/png", FileKind::Image),
            "u3",
            "1794391200.000100",
        ),
    ]
}

fn page(items: Vec<FileHit>, next: Option<&str>) -> Result<Page<FileHit>, String> {
    Ok(Page {
        items,
        next: next.map(Arc::from),
    })
}

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    for m in [member("u2", "Aiko Tanaka"), member("u3", "Jonas Weber")] {
        s.members.insert(m.id.clone(), m);
    }
    s
}

fn setup(
    cx: &mut TestAppContext,
    pages: Vec<Result<Page<FileHit>, String>>,
) -> (Entity<Src>, Entity<FilesTab<Src>>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(cx, &[components::catalog()], i18n::LocalePref::System);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        components::set_now(cx, Arc::new(FixedNow(Timestamp::from_second(NOW).unwrap())));
        components::set_time_zone(cx, jiff::tz::TimeZone::UTC);
    });
    let src = cx.new(|_| Src {
        state: state(),
        pages: pages.into(),
        calls: Vec::new(),
        held: Vec::new(),
        viewed: Vec::new(),
        downloaded: Vec::new(),
        inline: InlineImages::OnClick,
    });
    let s = src.clone();
    let (view, vcx) = cx.add_window_view(move |_w, cx| FilesTab::new(s, cid("C1"), cx));
    draw(vcx);
    (src, view, vcx)
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    vcx.run_until_parked();
}

#[gpui::test]
fn the_first_page_is_asked_for_once_and_listed_newest_first(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, vec![page(sample(), None)]);
    assert_eq!(
        src.read_with(vcx, |s, _| s.calls.clone()),
        [(None, components::files_tab::FILES_PAGE)]
    );
    view.read_with(vcx, |v, _| {
        assert_eq!(v.load(), FilesLoad::Ready);
        assert_eq!(
            v.file_names(),
            [
                "mockup.png",
                "report-q3.pdf",
                "export.bin",
                "screenshot.png"
            ]
        );
    });
    for row in ROWS {
        assert!(vcx.debug_bounds(row).is_some(), "{row}");
    }
    assert!(vcx.debug_bounds("files-empty").is_none());
}

#[gpui::test]
fn a_row_shows_size_sharer_and_date(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, vec![page(sample(), None)]);
    let meta = view.read_with(vcx, |v, _| v.row_meta(0));
    assert_eq!(
        meta.as_deref(),
        Some("50.0 KB \u{b7} Aiko Tanaka \u{b7} Nov 14, 2026")
    );
}

#[gpui::test]
fn no_files_shows_the_empty_state(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, vec![page(Vec::new(), None)]);
    assert_eq!(view.read_with(vcx, |v, _| v.load()), FilesLoad::Ready);
    assert!(vcx.debug_bounds("files-empty").is_some());
    assert!(vcx.debug_bounds("files-row-0").is_none());
}

#[gpui::test]
fn loading_shows_the_skeleton_and_a_failure_offers_retry(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, Vec::new());
    assert_eq!(view.read_with(vcx, |v, _| v.load()), FilesLoad::Loading);
    assert!(vcx.debug_bounds("files-loading").is_some());
    let done = src.update(vcx, |s, _| s.held.pop().expect("a load in flight"));
    vcx.update(|_, cx| done(Err("offline".into()), cx));
    draw(vcx);
    assert_eq!(view.read_with(vcx, |v, _| v.load()), FilesLoad::Failed);
    let retry = vcx.debug_bounds("files-retry").expect("retry button");
    src.update(vcx, |s, _| s.pages.push_back(page(sample(), None)));
    vcx.simulate_click(retry.center(), gpui::Modifiers::none());
    draw(vcx);
    assert_eq!(src.read_with(vcx, |s, _| s.calls.len()), 2);
    assert_eq!(view.read_with(vcx, |v, _| v.load()), FilesLoad::Ready);
}

#[gpui::test]
fn clicking_an_image_opens_the_viewer_on_the_images_oldest_first(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, vec![page(sample(), None)]);
    let row = vcx.debug_bounds("files-row-0").expect("row 0");
    vcx.simulate_click(row.center(), gpui::Modifiers::none());
    draw(vcx);
    assert_eq!(
        src.read_with(vcx, |s, _| s.viewed.clone()),
        [(vec!["F1".to_string(), "F4".to_string()], "F4".to_string())]
    );
    assert!(src.read_with(vcx, |s, _| s.downloaded.is_empty()));
}

#[gpui::test]
fn a_pdf_opens_alone_and_other_files_download(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, vec![page(sample(), None)]);
    for i in [1, 2] {
        let row = vcx.debug_bounds(ROWS[i]).expect("row");
        vcx.simulate_click(row.center(), gpui::Modifiers::none());
        draw(vcx);
    }
    assert_eq!(
        src.read_with(vcx, |s, _| s.viewed.clone()),
        [(vec!["F3".to_string()], "F3".to_string())]
    );
    assert_eq!(src.read_with(vcx, |s, _| s.downloaded.clone()), ["F2"]);
}

#[gpui::test]
fn arrow_keys_move_and_enter_opens(cx: &mut TestAppContext) {
    let (src, view, vcx) = setup(cx, vec![page(sample(), None)]);
    let focus = view.read_with(vcx, |v, _| v.focus_handle());
    vcx.update(|w, cx| w.focus(&focus, cx));
    vcx.simulate_keystrokes("down down");
    assert_eq!(view.read_with(vcx, |v, _| v.cursor()), 2);
    vcx.simulate_keystrokes("end");
    assert_eq!(view.read_with(vcx, |v, _| v.cursor()), 3);
    vcx.simulate_keystrokes("up home");
    assert_eq!(view.read_with(vcx, |v, _| v.cursor()), 0);
    vcx.simulate_keystrokes("down down enter");
    draw(vcx);
    assert_eq!(src.read_with(vcx, |s, _| s.downloaded.clone()), ["F2"]);
}

#[gpui::test]
fn scrolling_near_the_end_asks_for_the_next_page_once(cx: &mut TestAppContext) {
    // The first page's last rows are on screen, so drawing asks for page two;
    // its answer is held, and asks meanwhile are dropped.
    let (src, view, vcx) = setup(cx, vec![page(sample()[..2].to_vec(), Some("p2"))]);
    view.update(vcx, |v, cx| v.load_more(cx));
    draw(vcx);
    let cursors = |src: &Entity<Src>, vcx: &mut VisualTestContext| {
        src.read_with(vcx, |s, _| {
            s.calls.iter().map(|c| c.0.clone()).collect::<Vec<_>>()
        })
    };
    assert_eq!(cursors(&src, vcx), [None, Some(Arc::from("p2"))]);
    let done = src.update(vcx, |s, _| s.held.pop().expect("page two in flight"));
    vcx.update(|_, cx| done(page(sample()[2..].to_vec(), None), cx));
    draw(vcx);
    view.read_with(vcx, |v, _| {
        assert_eq!(v.file_names().len(), 4);
        assert!(!v.has_more());
    });
    view.update(vcx, |v, cx| v.load_more(cx));
    assert_eq!(
        cursors(&src, vcx).len(),
        2,
        "the last page asks for nothing"
    );
}

#[gpui::test]
fn past_the_cap_no_further_page_is_asked_for(cx: &mut TestAppContext) {
    let many: Vec<FileHit> = (0..components::files_tab::FILES_LOADED_MAX)
        .map(|i| {
            hit(
                file(
                    &format!("M{i}"),
                    "m.bin",
                    "application/octet-stream",
                    FileKind::Other,
                ),
                "u2",
                "1794391200.000100",
            )
        })
        .collect();
    let (src, view, vcx) = setup(cx, vec![page(many, Some("more"))]);
    view.update(vcx, |v, cx| v.load_more(cx));
    draw(vcx);
    assert_eq!(src.read_with(vcx, |s, _| s.calls.len()), 1);
    assert!(!view.read_with(vcx, |v, _| v.has_more()));
    view.update(vcx, |_, cx| cx.notify());
    draw(vcx);
    view.update(vcx, |v, _| v.scroll_to_end());
    draw(vcx);
    assert!(
        vcx.debug_bounds("files-end").is_some(),
        "the cap hint at the end"
    );
}

#[gpui::test]
fn image_thumbnails_follow_the_inline_images_setting(cx: &mut TestAppContext) {
    // `on_click` shows only thumbnails already cached; none are here.
    let (src, view, vcx) = setup(cx, vec![page(sample(), None)]);
    assert!(vcx.debug_bounds("files-thumb-0").is_none());
    // `always` shows every image's thumbnail; other files keep their icon.
    src.update(vcx, |s, _| s.inline = InlineImages::Always);
    view.update(vcx, |_, cx| cx.notify());
    draw(vcx);
    assert!(vcx.debug_bounds("files-thumb-0").is_some(), "the image row");
    assert!(vcx.debug_bounds("files-thumb-1").is_none(), "the PDF row");
    assert!(
        vcx.debug_bounds("files-thumb-3").is_some(),
        "the older image"
    );
    src.update(vcx, |s, _| s.inline = InlineImages::Never);
    view.update(vcx, |_, cx| cx.notify());
    draw(vcx);
    assert!(vcx.debug_bounds("files-thumb-0").is_none());
}

#[gpui::test]
fn a_failed_next_page_is_not_retried_until_the_user_asks(cx: &mut TestAppContext) {
    // Page two fails; the rows near the end stay on screen, yet redraws ask
    // for nothing more until Retry at the end of the list.
    let (src, _view, vcx) = setup(
        cx,
        vec![
            page(sample()[..2].to_vec(), Some("p2")),
            Err("offline".into()),
        ],
    );
    draw(vcx);
    draw(vcx);
    assert_eq!(
        src.read_with(vcx, |s, _| s.calls.len()),
        2,
        "one extra call"
    );
    let retry = vcx.debug_bounds("files-more-retry").expect("retry row");
    src.update(vcx, |s, _| {
        s.pages.push_back(page(sample()[2..].to_vec(), None))
    });
    vcx.simulate_click(retry.center(), gpui::Modifiers::none());
    draw(vcx);
    assert_eq!(src.read_with(vcx, |s, _| s.calls.len()), 3);
    assert!(vcx.debug_bounds("files-row-3").is_some());
    assert!(vcx.debug_bounds("files-more-retry").is_none());
}

#[gpui::test]
fn a_failed_follow_up_with_nothing_listed_shows_the_error(cx: &mut TestAppContext) {
    // The first page held only deleted files; looking further back failed.
    let (src, view, vcx) = setup(
        cx,
        vec![page(Vec::new(), Some("p2")), Err("offline".into())],
    );
    assert_eq!(src.read_with(vcx, |s, _| s.calls.len()), 2);
    assert_eq!(view.read_with(vcx, |v, _| v.load()), FilesLoad::Failed);
    assert!(vcx.debug_bounds("files-error").is_some());
    assert!(vcx.debug_bounds("files-empty").is_none());
}

#[gpui::test]
fn a_next_page_in_flight_shows_a_loading_row(cx: &mut TestAppContext) {
    let (src, _view, vcx) = setup(cx, vec![page(sample()[..2].to_vec(), Some("p2"))]);
    assert!(
        src.read_with(vcx, |s, _| !s.held.is_empty()),
        "page two asked for"
    );
    assert!(vcx.debug_bounds("files-more-loading").is_some());
}

#[gpui::test]
fn clicking_a_row_focuses_the_list(cx: &mut TestAppContext) {
    let (_src, view, vcx) = setup(cx, vec![page(sample(), None)]);
    let row = vcx.debug_bounds("files-row-2").expect("row 2");
    vcx.simulate_click(row.center(), gpui::Modifiers::none());
    let focus = view.read_with(vcx, |v, _| v.focus_handle());
    assert!(vcx.update(|w, _| focus.is_focused(w)));
    assert_eq!(view.read_with(vcx, |v, _| v.cursor()), 2);
}

#[gpui::test]
fn a_deleted_file_row_does_nothing(cx: &mut TestAppContext) {
    let mut hits = sample();
    hits[0].file.tombstone = true;
    let (src, _view, vcx) = setup(cx, vec![page(hits, None)]);
    let row = vcx.debug_bounds("files-row-0").expect("row 0");
    vcx.simulate_click(row.center(), gpui::Modifiers::none());
    draw(vcx);
    src.read_with(vcx, |s, _| {
        assert!(s.viewed.is_empty());
        assert!(s.downloaded.is_empty());
    });
}
