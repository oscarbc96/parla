//! The code viewer window: numbered lines, the whole file fetched when the
//! preview is only part of it, only the lines on screen drawn, Copy, Wrap,
//! find with next and previous, and Esc closing it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::code_viewer::MAX_CODE_BYTES;
use components::media::{MediaError, MediaFetcher};
use components::media_viewer::{Gallery, ViewerItem};
use domain::test_support::msg;
use domain::{FileId, FileKind, FileRef, ImageRef, MessageId, ProviderExt, RemoteUrl};
use futures::future::{BoxFuture, ready};
use gpui::{App, Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use parla::code_viewer::{CodeHost, CodeViewer};

fn code_file(preview: Option<&str>, size: u64) -> FileRef {
    FileRef {
        id: FileId(Arc::from("F1")),
        name: Arc::from("theme_loader.rs"),
        title: None,
        mime: Arc::from("text/plain"),
        size,
        kind: FileKind::Snippet,
        thumbs: Arc::from([]),
        download: Some(RemoteUrl(Arc::from("https://files.example/F1/dl"))),
        preview: preview.map(Arc::from),
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

fn gallery(f: FileRef) -> Gallery {
    Gallery::new(
        vec![ViewerItem {
            file: f,
            message: msg("C1", "1000.000001", "").id,
            sender: "Bob".into(),
        }],
        0,
    )
}

/// Answers every fetch with `body`, counting the calls.
struct Body {
    body: Result<Vec<u8>, MediaError>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl MediaFetcher for Body {
    fn fetch(&self, src: &ImageRef, max: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        assert!(matches!(src, ImageRef::Provider(u) if u.0.ends_with("/dl")));
        assert_eq!(max, MAX_CODE_BYTES);
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(ready(self.body.clone()))
    }
}

#[derive(Clone, Default)]
struct Host {
    calls: Rc<RefCell<Vec<String>>>,
}

impl CodeHost for Host {
    fn download(&self, f: &FileRef, _: &mut App) {
        self.calls.borrow_mut().push(format!("download {}", f.id.0));
    }
    fn open_in_channel(&self, m: &MessageId, _: &mut App) {
        self.calls.borrow_mut().push(format!("jump {}", m.ts.0));
    }
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
}

fn open_with(
    cx: &mut TestAppContext,
    f: FileRef,
    body: Result<Vec<u8>, MediaError>,
    host: Host,
) -> (
    WindowHandle<CodeViewer>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    init(cx);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fetcher: Arc<dyn MediaFetcher> = Arc::new(Body {
        body,
        calls: calls.clone(),
    });
    let w = cx.add_window(move |window, cx| {
        let host: Rc<dyn CodeHost> = Rc::new(host);
        let v = CodeViewer::new(gallery(f), Some(fetcher), host, window, cx);
        v.focus(window, cx);
        v
    });
    cx.run_until_parked();
    (w, calls)
}

fn draw(cx: &mut TestAppContext, w: WindowHandle<CodeViewer>) -> &mut VisualTestContext {
    let vcx = VisualTestContext::from_window(w.into(), cx).into_mut();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    vcx
}

fn click(vcx: &mut VisualTestContext, sel: &'static str) {
    let at = vcx.debug_bounds(sel).expect(sel).center();
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
}

#[gpui::test]
fn a_whole_preview_opens_at_once_with_numbered_lines_and_no_fetch(cx: &mut TestAppContext) {
    let text = "use a;\n\nfn b() {}\n";
    let (w, calls) = open_with(
        cx,
        code_file(Some(text), text.len() as u64),
        Ok(Vec::new()),
        Host::default(),
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(w.read_with(cx, |v, _| v.line_count()).unwrap(), 3);
    let vcx = draw(cx, w);
    assert!(vcx.debug_bounds("code-line-1").is_some());
    assert!(vcx.debug_bounds("code-line-3").is_some());
    assert!(
        vcx.debug_bounds("code-viewer-language").is_some(),
        "Rust label"
    );
}

#[gpui::test]
fn a_partial_preview_fetches_the_whole_file_and_draws_only_what_shows(cx: &mut TestAppContext) {
    let full: String = (1..=50_000).map(|i| format!("let x{i} = {i};\n")).collect();
    let (w, calls) = open_with(
        cx,
        code_file(Some("let x1 = 1;\n"), full.len() as u64),
        Ok(full.clone().into_bytes()),
        Host::default(),
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(w.read_with(cx, |v, _| v.line_count()).unwrap(), 50_000);
    let vcx = draw(cx, w);
    assert!(vcx.debug_bounds("code-line-1").is_some());
    assert!(
        vcx.debug_bounds("code-line-49999").is_none(),
        "lines off screen are not drawn"
    );
}

#[gpui::test]
fn a_failed_fetch_keeps_the_preview_and_says_so(cx: &mut TestAppContext) {
    let (w, _) = open_with(
        cx,
        code_file(Some("partial\n"), 10_000),
        Err(MediaError::Network("down".into())),
        Host::default(),
    );
    assert_eq!(w.read_with(cx, |v, _| v.line_count()).unwrap(), 1);
    let vcx = draw(cx, w);
    assert!(vcx.debug_bounds("code-viewer-failed").is_some());
    assert!(
        vcx.debug_bounds("code-line-1").is_some(),
        "the preview stays"
    );
}

#[gpui::test]
fn copy_puts_the_whole_file_on_the_clipboard(cx: &mut TestAppContext) {
    let text = "a\nb\nc\n";
    let (w, _) = open_with(
        cx,
        code_file(Some(text), text.len() as u64),
        Ok(Vec::new()),
        Host::default(),
    );
    let vcx = draw(cx, w);
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("stale".into()));
    click(vcx, "code-viewer-copy");
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()),
        Some(text.to_string())
    );
}

#[gpui::test]
fn wrap_and_line_numbers_toggle(cx: &mut TestAppContext) {
    let text = "a\nb\n";
    let (w, _) = open_with(
        cx,
        code_file(Some(text), text.len() as u64),
        Ok(Vec::new()),
        Host::default(),
    );
    let vcx = draw(cx, w);
    assert!(!w.read_with(vcx, |v, _| v.wrap()).unwrap());
    click(vcx, "code-viewer-wrap");
    assert!(w.read_with(vcx, |v, _| v.wrap()).unwrap(), "wraps");
    assert!(
        vcx.debug_bounds("code-line-2").is_some(),
        "still drawn wrapped"
    );
    click(vcx, "code-viewer-wrap");
    assert!(!w.read_with(vcx, |v, _| v.wrap()).unwrap());
    assert!(vcx.debug_bounds("code-number-1").is_some());
    click(vcx, "code-viewer-numbers");
    assert!(
        vcx.debug_bounds("code-number-1").is_none(),
        "numbers hidden"
    );
}

#[gpui::test]
fn download_and_jump_reach_the_workspace(cx: &mut TestAppContext) {
    let host = Host::default();
    let (w, _) = open_with(cx, code_file(Some("a\n"), 2), Ok(Vec::new()), host.clone());
    let vcx = draw(cx, w);
    click(vcx, "code-viewer-download");
    click(vcx, "code-viewer-jump");
    assert_eq!(*host.calls.borrow(), ["download F1", "jump 1000.000001"]);
}

#[gpui::test]
fn cmd_f_finds_and_enter_steps_through_the_matches(cx: &mut TestAppContext) {
    let text = "theme\nother\nTheme theme\n";
    let (w, _) = open_with(
        cx,
        code_file(Some(text), text.len() as u64),
        Ok(Vec::new()),
        Host::default(),
    );
    let vcx = draw(cx, w);
    vcx.simulate_keystrokes("cmd-f");
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(vcx.debug_bounds("code-viewer-find").is_some(), "find bar");
    vcx.simulate_input("theme");
    vcx.run_until_parked();
    let at = |vcx: &mut VisualTestContext| {
        w.read_with(vcx, |v, _| {
            (v.find().len(), v.find().current().map(|m| m.line))
        })
        .unwrap()
    };
    assert_eq!(at(vcx), (3, Some(0)));
    vcx.simulate_keystrokes("enter");
    assert_eq!(at(vcx), (3, Some(2)));
    vcx.simulate_keystrokes("shift-enter");
    assert_eq!(at(vcx), (3, Some(0)));
    vcx.simulate_keystrokes("shift-enter");
    assert_eq!(at(vcx), (3, Some(2)), "wraps to the last");
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    click(vcx, "code-viewer-find-next");
    assert_eq!(at(vcx), (3, Some(0)), "the button steps too");
    vcx.simulate_keystrokes("escape");
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert!(
        vcx.debug_bounds("code-viewer-find").is_none(),
        "find closed"
    );
    assert!(w.update(vcx, |_, _, _| ()).is_ok(), "the window stays");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(
        w.update(cx, |_, _, _| ()).is_err(),
        "a second Esc closes it"
    );
}

#[gpui::test]
fn open_reuses_one_window_and_close_shuts_it(cx: &mut TestAppContext) {
    use components::media::ImageCache;
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let fetcher: Arc<dyn MediaFetcher> = Arc::new(Body {
        body: Ok(b"x\n".to_vec()),
        calls: Arc::default(),
    });
    let d = dir.path().to_path_buf();
    cx.update(|cx| cx.set_global(ImageCache::new(fetcher, d, 1 << 20)));
    for _ in 0..2 {
        cx.update(|cx| {
            parla::code_viewer::open(
                gallery(code_file(Some("a\n"), 2)),
                Rc::new(Host::default()),
                cx,
            )
        });
        cx.run_until_parked();
    }
    assert_eq!(cx.windows().len(), 1, "reused");
    let w = cx
        .update(|cx| parla::code_viewer::open_window(cx))
        .expect("open");
    cx.update(parla::code_viewer::close);
    cx.run_until_parked();
    assert!(w.update(cx, |_, _, _| ()).is_err(), "closed");
    assert!(
        cx.update(|cx| parla::code_viewer::open_window(cx))
            .is_none()
    );
}

#[gpui::test]
fn enter_on_a_focused_button_presses_it_and_only_the_find_field_steps(cx: &mut TestAppContext) {
    let text = "theme\ntheme\n";
    let (w, _) = open_with(
        cx,
        code_file(Some(text), text.len() as u64),
        Ok(Vec::new()),
        Host::default(),
    );
    let vcx = draw(cx, w);
    vcx.simulate_keystrokes("cmd-f");
    vcx.simulate_input("theme");
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    let current = |vcx: &mut VisualTestContext| {
        w.read_with(vcx, |v, _| v.find().current().map(|m| m.line))
            .unwrap()
    };
    assert_eq!(current(vcx), Some(0));
    // Tab from the find field to the next control: find's Previous button.
    w.update(vcx, |v, window, cx| {
        v.focus_control_for_tests("code-viewer-wrap", window, cx)
    })
    .unwrap();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(
        w.read_with(vcx, |v, _| v.wrap()).unwrap(),
        "Enter pressed Wrap"
    );
    assert_eq!(current(vcx), Some(0), "and did not step find");
}

/// Counts the fetches in flight: each holds a guard until it is dropped.
struct Hanging {
    in_flight: Arc<std::sync::atomic::AtomicUsize>,
    started: Arc<std::sync::atomic::AtomicUsize>,
}

struct Guard(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl MediaFetcher for Hanging {
    fn fetch(&self, _: &ImageRef, _: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        self.in_flight
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.started
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let g = Guard(self.in_flight.clone());
        Box::pin(async move {
            let _g = g;
            futures::future::pending::<()>().await;
            Ok(Vec::new())
        })
    }
}

#[gpui::test]
fn another_file_or_closing_drops_the_fetch_in_flight(cx: &mut TestAppContext) {
    use std::sync::atomic::Ordering::SeqCst;
    init(cx);
    let (in_flight, started) = (Arc::new(Default::default()), Arc::new(Default::default()));
    let fetcher: Arc<dyn MediaFetcher> = Arc::new(Hanging {
        in_flight: Arc::clone(&in_flight),
        started: Arc::clone(&started),
    });
    let partial = || code_file(Some("a\n"), 10_000);
    let f2 = fetcher.clone();
    let w = cx.add_window(move |window, cx| {
        CodeViewer::new(
            gallery(partial()),
            Some(f2),
            Rc::new(Host::default()),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    let in_flight_now = |_: &mut TestAppContext| in_flight.load(SeqCst);
    assert_eq!(in_flight_now(cx), 1);
    w.update(cx, |v, window, cx| {
        v.set_gallery(
            gallery(partial()),
            Some(fetcher.clone()),
            Rc::new(Host::default()),
            window,
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started.load(SeqCst), 2);
    assert_eq!(in_flight_now(cx), 1, "the first fetch was dropped");
    w.update(cx, |_, window, _| window.remove_window()).unwrap();
    cx.run_until_parked();
    assert_eq!(in_flight_now(cx), 0, "closing drops it");
}

#[gpui::test]
fn the_whole_file_loads_through_the_owning_workspace(cx: &mut TestAppContext) {
    use components::media::ImageCache;
    use std::sync::atomic::Ordering::SeqCst;
    init(cx);
    let (global_calls, own_calls) = (Arc::new(Default::default()), Arc::new(Default::default()));
    let global: Arc<dyn MediaFetcher> = Arc::new(Body {
        body: Ok(b"x\n".to_vec()),
        calls: Arc::clone(&global_calls),
    });
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_path_buf();
    cx.update(|cx| cx.set_global(ImageCache::new(global, d, 1 << 20)));
    #[derive(Clone)]
    struct Owner(Arc<dyn MediaFetcher>);
    impl CodeHost for Owner {
        fn download(&self, _: &FileRef, _: &mut App) {}
        fn fetcher(&self, _: &App) -> Option<Arc<dyn MediaFetcher>> {
            Some(self.0.clone())
        }
    }
    let own: Arc<dyn MediaFetcher> = Arc::new(Body {
        body: Ok(b"whole\nfile\n".to_vec()),
        calls: Arc::clone(&own_calls),
    });
    cx.update(|cx| {
        parla::code_viewer::open(
            gallery(code_file(Some("whole\n"), 11)),
            Rc::new(Owner(own)),
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(own_calls.load(SeqCst), 1);
    assert_eq!(global_calls.load(SeqCst), 0, "not the active workspace's");
    let w = cx.update(|cx| parla::code_viewer::open_window(cx)).unwrap();
    assert_eq!(w.read_with(cx, |v, _| v.line_count()).unwrap(), 2);
}

#[gpui::test]
fn a_redraw_or_scroll_builds_only_new_lines_and_the_cache_is_bounded(cx: &mut TestAppContext) {
    let full: String = (1..=5_000).map(|i| format!("let x{i} = {i};\n")).collect();
    let (w, _) = open_with(
        cx,
        code_file(Some(&full), full.len() as u64),
        Ok(Vec::new()),
        Host::default(),
    );
    let vcx = draw(cx, w);
    let builds = |vcx: &mut VisualTestContext| w.read_with(vcx, |v, _| v.line_builds()).unwrap();
    let first = builds(vcx);
    assert!(first > 0);
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    vcx.update(|w, _| w.refresh());
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    assert_eq!(builds(vcx), first, "a redraw builds no line");
    for top in (100..4_000).step_by(300) {
        w.update(vcx, |v, _, cx| v.scroll_to_line_for_tests(top, cx))
            .unwrap();
        vcx.update(|w, cx| w.draw(cx).clear(cx));
    }
    let cached = w.read_with(vcx, |v, _| v.cached_lines()).unwrap();
    assert!(
        cached <= parla::code_viewer::LINE_CACHE_MAX,
        "{cached} lines cached"
    );
    assert!(builds(vcx) > first, "new lines were built");
}

#[gpui::test]
fn typing_fast_keeps_only_the_last_query(cx: &mut TestAppContext) {
    let text = "alpha\nbeta\nalphabet\n";
    let (w, _) = open_with(
        cx,
        code_file(Some(text), text.len() as u64),
        Ok(Vec::new()),
        Host::default(),
    );
    let vcx = draw(cx, w);
    vcx.simulate_keystrokes("cmd-f");
    vcx.simulate_input("b");
    vcx.simulate_input("et");
    vcx.run_until_parked();
    let r = w
        .read_with(vcx, |v, _| (v.find().query().to_string(), v.find().len()))
        .unwrap();
    assert_eq!(r, ("bet".to_string(), 2));
}
