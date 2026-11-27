//! The media viewer's model: which files it steps through and in what order,
//! zoom and pan, the bound on decoded images, and opening it from the timeline.

use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::Arc;

use components::media::{MediaError, MediaFetcher, VIEWER_KEEP, ViewerCache, ViewerState};
use components::media_viewer::{Gallery, ViewState, Zoom, source};
use components::{Timeline, TimelineSource};
use domain::test_support::{cid, member, msg, uid};
use domain::{
    Capabilities, ConversationId, FileId, FileKind, FileRef, HistoryPage, ImageRef, Input, Member,
    Message, MessageId, MessageWindow, ProviderExt, RemoteUrl, Rendition, UserId, WorkspaceState,
    reduce,
};
use futures::future::{BoxFuture, ready};
use gpui::{App, AppContext as _, Entity, TestAppContext};
use image::{ImageFormat, Rgba, RgbaImage};
use settings::{Clock, WorkspaceKey};

fn url(s: &str) -> RemoteUrl {
    RemoteUrl(Arc::from(s))
}

fn file(id: &str, kind: FileKind) -> FileRef {
    let (name, mime) = match kind {
        FileKind::Pdf => (format!("{id}.pdf"), "application/pdf"),
        FileKind::Image => (format!("{id}.png"), "image/png"),
        _ => (format!("{id}.txt"), "text/plain"),
    };
    FileRef {
        id: FileId(Arc::from(id)),
        name: Arc::from(name.as_str()),
        title: None,
        mime: Arc::from(mime),
        size: 2048,
        kind,
        thumbs: Arc::from([Rendition {
            url: url(&format!("https://files.example/{id}/t360")),
            width: 360,
            height: 200,
        }]),
        download: Some(url(&format!("https://files.example/{id}/dl"))),
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

/// Messages 1..: m1 [A image, T text], m2 [B image, P pdf], m3 [C image].
fn messages() -> Vec<Message> {
    let mut out = Vec::new();
    for (ts, files) in [
        (
            "1000.000001",
            vec![file("A", FileKind::Image), file("T", FileKind::Other)],
        ),
        (
            "1000.000002",
            vec![file("B", FileKind::Image), file("P", FileKind::Pdf)],
        ),
        ("1000.000003", vec![file("C", FileKind::Image)]),
    ] {
        let mut m = msg("C1", ts, "files");
        m.author = Some(uid("u2"));
        m.files = Arc::from(files);
        out.push(m);
    }
    out
}

fn window() -> MessageWindow {
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    reduce(
        &mut state,
        Input::HistoryLoaded {
            conversation: cid("C1"),
            page: HistoryPage {
                messages: messages(),
                has_more: false,
            },
        },
    );
    state.windows.get(&cid("C1")).unwrap().clone()
}

fn names(g: &Gallery) -> Vec<String> {
    g.items().iter().map(|i| i.file.id.0.to_string()).collect()
}

fn mid(ts: &str) -> MessageId {
    msg("C1", ts, "").id
}

#[test]
fn gallery_steps_through_the_images_of_the_window_in_order() {
    let w = window();
    let mut g = Gallery::from_window(&w, &mid("1000.000002"), &FileId(Arc::from("B")), |_| {
        "Bob".into()
    })
    .expect("an image opens a gallery");
    assert_eq!(names(&g), ["A", "B", "C"], "images only, oldest first");
    assert_eq!(g.index(), 1, "opens on the clicked image");
    assert_eq!(&*g.current().unwrap().sender, "Bob");
    assert!(g.step_next());
    assert_eq!(g.current().unwrap().file.id.0.as_ref(), "C");
    assert!(!g.step_next(), "does not wrap at the end");
    assert!(g.step_prev() && g.step_prev());
    assert_eq!(g.current().unwrap().file.id.0.as_ref(), "A");
    assert!(!g.step_prev(), "does not wrap at the start");
}

#[test]
fn a_pdf_opens_on_its_own_and_text_files_do_not_open() {
    let w = window();
    let g = Gallery::from_window(&w, &mid("1000.000002"), &FileId(Arc::from("P")), |_| {
        "Bob".into()
    })
    .expect("a PDF opens the viewer");
    assert_eq!(names(&g), ["P"]);
    assert!(
        source(&g.current().unwrap().file).is_none(),
        "no image to decode"
    );
    assert!(
        Gallery::from_window(&w, &mid("1000.000001"), &FileId(Arc::from("T")), |_| {
            "Bob".into()
        })
        .is_none()
    );
}

#[test]
fn the_viewer_shows_the_original_unless_it_is_too_large() {
    let f = file("A", FileKind::Image);
    assert_eq!(
        source(&f),
        Some(ImageRef::Provider(url("https://files.example/A/dl")))
    );
    let mut big = f.clone();
    big.size = 50 * 1024 * 1024;
    assert_eq!(
        source(&big),
        Some(ImageRef::Provider(url("https://files.example/A/t360"))),
        "too large to fetch: the largest rendition"
    );
}

const IMG: (f32, f32) = (2000.0, 1000.0);
const STAGE: (f32, f32) = (1000.0, 600.0);

#[test]
fn zoom_steps_actual_size_fit_and_pan() {
    let mut v = ViewState::default();
    assert_eq!(v.zoom, Zoom::Fit);
    assert_eq!(v.percent(IMG, STAGE), 50, "fits the stage");
    assert!(!v.can_pan(IMG, STAGE), "nothing to pan when it fits");
    v.zoom_in(IMG, STAGE);
    assert_eq!(v.percent(IMG, STAGE), 75);
    v.zoom_in(IMG, STAGE);
    assert_eq!(v.percent(IMG, STAGE), 100);
    v.zoom_out(IMG, STAGE);
    v.zoom_out(IMG, STAGE);
    v.zoom_out(IMG, STAGE);
    assert_eq!(v.percent(IMG, STAGE), 25, "out goes below fit");
    v.actual_size(IMG, STAGE);
    assert_eq!(v.zoom, Zoom::Scale(1.0));
    assert!(v.can_pan(IMG, STAGE));
    v.pan_by((-5000.0, 30.0), IMG, STAGE);
    assert_eq!(v.pan, (-500.0, 30.0), "pan stops at the image edge");
    v.fit();
    assert_eq!(v, ViewState::default(), "fit recentres");
    small_images_are_never_enlarged_to_fit();
}

fn small_images_are_never_enlarged_to_fit() {
    let v = ViewState::default();
    assert_eq!(v.percent((100.0, 50.0), STAGE), 100);
}

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(8, 4, Rgba([30, 120, 200, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

struct Fixed(Vec<u8>);

impl MediaFetcher for Fixed {
    fn fetch(&self, _: &ImageRef, _: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        Box::pin(ready(Ok(self.0.clone())))
    }
}

struct Holder {
    cache: ViewerCache,
}

#[gpui::test]
fn the_viewer_keeps_only_the_current_and_next_image_decoded(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut items = Vec::new();
    let w = window();
    for m in w.messages() {
        for f in m.files.iter().filter(|f| f.kind == FileKind::Image) {
            items.push(components::media_viewer::ViewerItem {
                file: f.clone(),
                message: m.id.clone(),
                sender: "Bob".into(),
            });
        }
    }
    let mut g = Gallery::new(items, 0);
    let h: Entity<Holder> = cx.new(|_| Holder {
        cache: ViewerCache::new(Arc::new(Fixed(png())), dir.path().to_path_buf()),
    });
    let first = source(&g.current().unwrap().file).unwrap();
    loop {
        let wanted = g.wanted();
        h.update(cx, |v, cx| {
            v.cache.want(&wanted, |v: &mut Holder| &mut v.cache, cx)
        });
        cx.run_until_parked();
        h.read_with(cx, |v, _| {
            assert!(v.cache.held() <= VIEWER_KEEP, "held {}", v.cache.held());
            assert_eq!(v.cache.decoded(), wanted.len());
            let cur = source(&g.current().unwrap().file).unwrap();
            assert!(matches!(v.cache.state(&cur), Some(ViewerState::Ready(_))));
        });
        if !g.step_next() {
            break;
        }
    }
    h.read_with(cx, |v, _| {
        assert!(
            v.cache.state(&first).is_none(),
            "the first image was dropped"
        );
        assert_eq!(v.cache.held(), 1, "the last image has no next neighbour");
    });
}

type Opened = Rc<RefCell<Vec<Gallery>>>;

struct Src {
    state: WorkspaceState,
    opened: Opened,
    downloads: Rc<RefCell<Vec<FileId>>>,
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
        self.downloads.borrow_mut().push(f.id.clone());
    }
    fn open_media(&mut self, g: Gallery, _: &mut App) {
        self.opened.borrow_mut().push(g);
    }
}

#[gpui::test]
fn clicking_an_image_in_the_timeline_opens_the_viewer_on_it(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    state.self_user = Some(uid("u1"));
    state.members.insert(uid("u2"), member("u2", "Bob"));
    reduce(
        &mut state,
        Input::HistoryLoaded {
            conversation: cid("C1"),
            page: HistoryPage {
                messages: messages(),
                has_more: false,
            },
        },
    );
    let opened: Opened = Rc::default();
    let downloads = Rc::new(RefCell::new(Vec::new()));
    let (o, d) = (opened.clone(), downloads.clone());
    let src = cx.new(move |_| Src {
        state,
        opened: o,
        downloads: d,
    });
    let (_view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    // Images are held back by default: the first click shows it, the next opens it.
    for _ in 0..2 {
        let at = vcx
            .debug_bounds("file-card-B")
            .expect("image card B")
            .center();
        vcx.simulate_click(at, gpui::Modifiers::none());
        vcx.run_until_parked();
        vcx.update(|w, cx| w.draw(cx).clear(cx));
    }
    let got = opened.borrow();
    assert_eq!(got.len(), 1, "one viewer opened");
    assert_eq!(names(&got[0]), ["A", "B", "C"]);
    assert_eq!(got[0].index(), 1);
    assert_eq!(&*got[0].current().unwrap().sender, "Bob");
    assert!(downloads.borrow().is_empty(), "nothing is downloaded");
}

#[test]
fn zoom_never_jumps_the_wrong_way_past_its_limits() {
    let huge = (100_000.0, 1000.0);
    let mut v = ViewState::default();
    let fit = v.scale(huge, STAGE);
    assert!(fit < components::media_viewer::ZOOM_MIN);
    v.zoom_out(huge, STAGE);
    assert!(v.scale(huge, STAGE) <= fit, "zoom out never enlarges");
    let mut v = ViewState {
        zoom: Zoom::Scale(9.0),
        pan: (0.0, 0.0),
    };
    v.zoom_in(IMG, STAGE);
    assert!(v.scale(IMG, STAGE) >= 9.0, "zoom in never shrinks");
}

#[test]
fn layout_keeps_a_stale_pan_inside_a_resized_stage() {
    let mut v = ViewState::default();
    v.actual_size(IMG, STAGE);
    v.pan_by((-5000.0, 0.0), IMG, STAGE);
    // The window grew to fit the whole image: nothing is left to pan.
    let (_, origin) = v.layout(IMG, (2000.0, 1000.0));
    assert_eq!(origin, (0.0, 0.0));
}

fn encoded(w: u32, h: u32, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    match format {
        ImageFormat::Jpeg => image::RgbImage::from_pixel(w, h, image::Rgb([200, 90, 40]))
            .write_to(&mut out, format)
            .unwrap(),
        _ => RgbaImage::from_pixel(w, h, Rgba([30, 120, 200, 255]))
            .write_to(&mut out, format)
            .unwrap(),
    }
    out.into_inner()
}

/// A PNG of only its signature and header chunk: decoding its pixels fails.
fn png_header(w: u32, h: u32) -> Vec<u8> {
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = !0u32;
        for b in bytes {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    (c >> 1) ^ 0xEDB8_8320
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    let mut chunk = b"IHDR".to_vec();
    chunk.extend(w.to_be_bytes());
    chunk.extend(h.to_be_bytes());
    chunk.extend([8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(13u32.to_be_bytes());
    out.extend(&chunk);
    out.extend(crc(&chunk).to_be_bytes());
    out
}

#[test]
fn camera_photos_display_downscaled_to_the_viewer_cap() {
    use components::media::{VIEWER_MAX_PX, decode_viewer};
    for (w, h, format) in [
        (6000, 4000, ImageFormat::Jpeg),
        (5000, 5000, ImageFormat::Png),
    ] {
        let img = decode_viewer(&encoded(w, h, format)).expect("shown");
        assert_eq!(img.natural, (w, h), "{format:?}");
        let s = img.image.size(0);
        let (dw, dh) = (s.width.0 as u32, s.height.0 as u32);
        assert!(
            dw <= VIEWER_MAX_PX && dh <= VIEWER_MAX_PX,
            "{format:?} {dw}x{dh}"
        );
        assert_eq!(dw.max(dh), VIEWER_MAX_PX, "{format:?} fills the cap");
    }
    let small = decode_viewer(&png()).expect("shown");
    assert_eq!(
        small.image.size(0).width.0,
        8,
        "a small image keeps its size"
    );
}

#[test]
fn images_above_the_decode_cap_are_refused_from_the_header() {
    use components::media::decode_viewer;
    assert_eq!(
        decode_viewer(&png_header(20_000, 20_000)).err(),
        Some(MediaError::TooLarge)
    );
}

#[gpui::test]
fn a_large_next_image_is_not_preloaded(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let h: Entity<Holder> = cx.new(|_| Holder {
        cache: ViewerCache::new(
            Arc::new(Fixed(encoded(6000, 4000, ImageFormat::Jpeg))),
            dir.path().to_path_buf(),
        ),
    });
    let (cur, next) = (
        ImageRef::Provider(url("https://files.example/cur")),
        ImageRef::Provider(url("https://files.example/next")),
    );
    let wanted = [cur.clone(), next.clone()];
    h.update(cx, |v, cx| {
        v.cache.want(&wanted, |v: &mut Holder| &mut v.cache, cx)
    });
    cx.run_until_parked();
    h.read_with(cx, |v, _| {
        assert!(matches!(v.cache.state(&cur), Some(ViewerState::Ready(_))));
        assert!(
            v.cache.state(&next).is_none(),
            "left for when it is current"
        );
    });
    // Stepping to it decodes it.
    let wanted = [next.clone()];
    h.update(cx, |v, cx| {
        v.cache.want(&wanted, |v: &mut Holder| &mut v.cache, cx)
    });
    cx.run_until_parked();
    h.read_with(cx, |v, _| {
        assert!(matches!(v.cache.state(&next), Some(ViewerState::Ready(_))));
    });
}

#[test]
fn the_disk_cache_is_pruned_when_a_write_passes_its_budget() {
    use components::media::DiskBudget;
    let dir = tempfile::tempdir().unwrap();
    let budget = DiskBudget::new(dir.path().to_path_buf(), 250);
    for (i, name) in ["a", "b", "c"].iter().enumerate() {
        let p = dir.path().join(name);
        std::fs::write(&p, [0u8; 100]).unwrap();
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i as u64);
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(t)
            .unwrap();
        budget.record(100);
    }
    assert!(!dir.path().join("a").exists(), "the oldest file goes");
    assert!(dir.path().join("b").exists() && dir.path().join("c").exists());
}

#[test]
fn deleted_messages_leave_the_gallery() {
    let w = window();
    let mut g = Gallery::from_window(&w, &mid("1000.000002"), &FileId(Arc::from("B")), |_| {
        "Bob".into()
    })
    .unwrap();
    assert!(g.retain(|it| it.file.id.0.as_ref() != "B"));
    assert_eq!(names(&g), ["A", "C"]);
    assert_eq!(
        g.current().unwrap().file.id.0.as_ref(),
        "C",
        "the next one shows"
    );
    assert!(!g.retain(|_| true), "nothing changed");
    g.retain(|_| false);
    assert!(g.is_empty());
}

#[gpui::test]
fn thumbnails_load_through_the_viewers_own_fetcher(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let h: Entity<Holder> = cx.new(|_| Holder {
        cache: ViewerCache::new(Arc::new(Fixed(png())), dir.path().to_path_buf()),
    });
    let thumb = ImageRef::Provider(url("https://files.example/A/t360"));
    let want = vec![thumb.clone()];
    h.update(cx, |v, cx| {
        v.cache
            .want_thumbs(&want, |v: &mut Holder| &mut v.cache, cx)
    });
    cx.run_until_parked();
    // No global image cache exists in this test: the viewer's fetcher served it.
    h.read_with(cx, |v, _| {
        assert!(matches!(v.cache.thumb(&thumb), Some(ViewerState::Ready(_))))
    });
}
