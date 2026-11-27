mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use components::files::{Card, card};
use components::upload::{MAX_UPLOAD_BYTES, cleanup_temp, plan, write_pasted_image};
use components::{Composer, ComposerSource, Timeline, TimelineSource};
use domain::test_support::{cid, msg, uid};
use domain::{
    Capabilities, ConversationId, FileId, FileKind, FileRef, ImageRef, MessageWindow, ProviderExt,
    RemoteUrl, Rendition, UserId, WorkspaceState,
};
use gpui::{AppContext as _, Entity, TestAppContext};
use settings::{Clock, InlineImages, WorkspaceKey};

fn url(s: &str) -> RemoteUrl {
    RemoteUrl(Arc::from(s))
}

fn file(kind: FileKind) -> FileRef {
    FileRef {
        id: FileId(Arc::from("F1")),
        name: Arc::from("cat.png"),
        title: None,
        mime: Arc::from("image/png"),
        size: 2048,
        kind,
        thumbs: Arc::from([
            Rendition {
                url: url("https://files.example/t360"),
                width: 360,
                height: 200,
            },
            Rendition {
                url: url("https://files.example/t720"),
                width: 720,
                height: 400,
            },
        ]),
        download: Some(url("https://files.example/dl")),
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

#[test]
fn inline_images_setting_controls_preview() {
    let f = file(FileKind::Image);
    assert!(matches!(
        card(&f, InlineImages::Always, false),
        Card::Image { .. }
    ));
    assert!(matches!(
        card(&f, InlineImages::OnClick, false),
        Card::ImagePlaceholder { .. }
    ));
    assert!(matches!(
        card(&f, InlineImages::OnClick, true),
        Card::Image { .. }
    ));
    match card(&f, InlineImages::Never, true) {
        Card::Plain { name, .. } => assert_eq!(&*name, "cat.png"),
        other => panic!("{other:?}"),
    }
    match card(&f, InlineImages::Always, false) {
        Card::Image { src, .. } => {
            assert_eq!(src, ImageRef::Provider(url("https://files.example/t360")))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn pdf_uses_provider_thumbnail() {
    let mut f = file(FileKind::Pdf);
    f.name = Arc::from("a.pdf");
    match card(&f, InlineImages::Never, false) {
        Card::Pdf { thumb, .. } => assert!(thumb.is_some()),
        other => panic!("{other:?}"),
    }
    f.thumbs = Arc::from([]);
    assert!(matches!(
        card(&f, InlineImages::Always, false),
        Card::Pdf { thumb: None, .. }
    ));
}

#[test]
fn snippet_shows_its_first_lines() {
    let mut f = file(FileKind::Snippet);
    f.name = Arc::from("a.txt");
    f.preview = Some(Arc::from("1\n2\n3\n4\n5\n6\n7\n8\n9\n10"));
    match card(&f, InlineImages::Always, false) {
        Card::Snippet(s) => {
            assert_eq!(s.shown(), ["1", "2", "3", "4", "5", "6", "7", "8"]);
            assert_eq!(s.hidden(), 2);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn tombstone_card() {
    let mut f = file(FileKind::Image);
    f.tombstone = true;
    assert!(matches!(
        card(&f, InlineImages::Always, true),
        Card::Tombstone
    ));
}

#[test]
fn oversize_upload_refused() {
    let dir = tempfile::tempdir().unwrap();
    let ok = dir.path().join("ok.txt");
    std::fs::write(&ok, b"hi").unwrap();
    let big = dir.path().join("big.bin");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(MAX_UPLOAD_BYTES + 1).unwrap();
    let missing = dir.path().join("nope");
    let p = plan(&[ok.clone(), big, missing]);
    assert_eq!(p.accepted.len(), 1);
    assert_eq!(p.accepted[0].path, ok);
    assert_eq!(&*p.accepted[0].name, "ok.txt");
    assert_eq!(p.accepted[0].size, 2);
    assert_eq!(p.refused.len(), 2);
    let text = p.refused[0].text(&common::en());
    assert!(text.contains("big.bin") && text.contains("1 GB"), "{text}");
}

#[test]
fn paste_image_writes_temp_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let uploads = dir.path().join("uploads");
    let img = gpui::Image::from_bytes(gpui::ImageFormat::Png, tiny_png());
    let p = write_pasted_image(&uploads, &img).unwrap();
    assert!(p.starts_with(&uploads) && p.extension().unwrap() == "png");
    assert!(p.exists());
    let other = dir.path().join("keep.png");
    std::fs::write(&other, b"x").unwrap();
    cleanup_temp(&[p.clone(), other.clone()], &uploads);
    assert!(!p.exists());
    assert!(
        other.exists(),
        "files outside the uploads dir are never removed"
    );
}

fn tiny_png() -> Vec<u8> {
    let mut out = Vec::new();
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

type Calls = Rc<RefCell<Vec<(ConversationId, Option<String>, Vec<PathBuf>)>>>;

struct Src {
    state: WorkspaceState,
    me: UserId,
    caps: Capabilities,
    uploads: Calls,
}

impl Src {
    fn new(uploads: Calls) -> Self {
        Self {
            state: WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 100),
            me: uid("u1"),
            caps: Capabilities::default(),
            uploads,
        }
    }
}

impl TimelineSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn member(&self, u: &UserId) -> Option<&domain::Member> {
        self.state.members.get(u)
    }
    fn self_user(&self) -> Option<&UserId> {
        Some(&self.me)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn load_older(&mut self, _: ConversationId, _: &mut gpui::App) {}
    fn edit(&mut self, _: domain::MessageId, _: String, _: &mut gpui::App) {}
    fn delete(&mut self, _: domain::MessageId, _: &mut gpui::App) {}
    fn react(&mut self, _: domain::MessageId, _: domain::EmojiRef, _: bool, _: &mut gpui::App) {}
    fn custom_emoji(&self) -> Vec<domain::CustomEmoji> {
        Vec::new()
    }
    fn stage(
        &mut self,
        c: ConversationId,
        _thread: Option<domain::MessageTs>,
        paths: Vec<PathBuf>,
        _cx: &mut gpui::App,
    ) {
        self.uploads.borrow_mut().push((c, None, paths));
    }
}

impl ComposerSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn self_user(&self) -> Option<&UserId> {
        Some(&self.me)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn stage(
        &mut self,
        c: ConversationId,
        _thread: Option<domain::MessageTs>,
        paths: Vec<PathBuf>,
        _cx: &mut gpui::App,
    ) {
        self.uploads.borrow_mut().push((c, None, paths));
    }
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
}

#[gpui::test]
fn drop_files_are_staged_not_uploaded(cx: &mut TestAppContext) {
    init(cx);
    let calls: Calls = Rc::default();
    let src: Entity<Src> = cx.new(|_| Src::new(calls.clone()));
    let s2 = src;
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(s2, cid("C1"), Clock::H24, cx));
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    std::fs::write(&a, b"hi").unwrap();
    let big = dir.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_UPLOAD_BYTES + 1)
        .unwrap();
    view.update(vcx, |t, cx| t.drop_paths(vec![a.clone(), big], cx));
    let got = calls.borrow().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, cid("C1"));
    assert_eq!(got[0].2, vec![a]);
    view.read_with(vcx, |t, _| {
        assert!(t.upload_notice(&common::en()).unwrap().contains("big.bin"))
    });
    let _ = msg("C1", "1.0", "x");
}

/// The notice under the timeline takes its own space; it never covers the list.
#[gpui::test]
fn upload_notice_does_not_overlap_the_list(cx: &mut TestAppContext) {
    init(cx);
    let calls: Calls = Rc::default();
    let src: Entity<Src> = cx.new(|_| Src::new(calls.clone()));
    let s2 = src;
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(s2, cid("C1"), Clock::H24, cx));
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_UPLOAD_BYTES + 1)
        .unwrap();
    view.update(vcx, |t, cx| t.drop_paths(vec![big], cx));
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    let list = vcx.debug_bounds("timeline-list").expect("list painted");
    let footer = vcx.debug_bounds("timeline-footer").expect("footer painted");
    assert!(footer.size.height > gpui::px(0.0), "notice is shown");
    assert!(
        list.bottom() <= footer.origin.y,
        "list {list:?} overlaps notice {footer:?}"
    );
}

#[gpui::test]
fn pasted_image_is_written_and_staged(cx: &mut TestAppContext) {
    init(cx);
    let calls: Calls = Rc::default();
    let src: Entity<Src> = cx.new(|_| Src::new(calls.clone()));
    let s2 = src;
    let dir = tempfile::tempdir().unwrap();
    let uploads = dir.path().join("uploads");
    let up = uploads.clone();
    let (comp, vcx) = cx.add_window_view(move |_w, cx| {
        let mut c = Composer::new(s2, cid("C1"), cx);
        c.set_upload_dir(up);
        c
    });
    let img = gpui::Image::from_bytes(gpui::ImageFormat::Png, tiny_png());
    comp.update(vcx, |c, cx| c.paste_image(&img, cx));
    let got = calls.borrow().clone();
    assert_eq!(got.len(), 1);
    assert!(got[0].2[0].starts_with(&uploads) && got[0].2[0].exists());
}

#[gpui::test]
fn edit_menu_paste_attaches_a_clipboard_image(cx: &mut TestAppContext) {
    use gpui::Focusable as _;
    init(cx);
    let calls: Calls = Rc::default();
    let src: Entity<Src> = cx.new(|_| Src::new(calls.clone()));
    let dir = tempfile::tempdir().unwrap();
    let uploads = dir.path().join("uploads");
    let up = uploads.clone();
    let (comp, vcx) = cx.add_window_view(move |_w, cx| {
        let mut c = Composer::new(src, cid("C1"), cx);
        c.set_upload_dir(up);
        c
    });
    let focus = comp.read_with(vcx, |c, cx| c.input().read(cx).focus_handle(cx));
    vcx.update(|w, cx| w.focus(&focus, cx));
    let img = gpui::Image::from_bytes(gpui::ImageFormat::Png, tiny_png());
    vcx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_image(&img)));
    // The Edit menu's Paste is the text input's action, not the composer's.
    vcx.dispatch_action(components::text_input::Paste);
    assert_eq!(calls.borrow().len(), 1, "the image is staged, not dropped");
}

#[gpui::test]
fn switching_conversation_clears_the_upload_notice(cx: &mut TestAppContext) {
    init(cx);
    let calls: Calls = Rc::default();
    let src: Entity<Src> = cx.new(|_| Src::new(calls));
    let (view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_UPLOAD_BYTES + 1)
        .unwrap();
    view.update(vcx, |t, cx| t.drop_paths(vec![big], cx));
    view.read_with(vcx, |t, _| {
        assert!(t.upload_notice(&common::en()).is_some())
    });
    view.update(vcx, |t, cx| t.set_conversation(cid("C2"), cx));
    view.read_with(vcx, |t, _| {
        assert!(t.upload_notice(&common::en()).is_none())
    });
}

fn named(id: &str, name: &str, kind: FileKind) -> FileRef {
    let mut f = file(kind);
    f.id = FileId(Arc::from(id));
    f.name = Arc::from(name);
    f.thumbs = Arc::from([]);
    f
}

#[test]
fn type_badge_is_the_caps_extension_or_absent() {
    use components::files::type_badge_text;
    assert_eq!(type_badge_text("report.pdf").as_deref(), Some("PDF"));
    assert_eq!(type_badge_text("backup.zip").as_deref(), Some("ZIP"));
    assert_eq!(type_badge_text("README"), None);
    assert_eq!(type_badge_text("notes.markdown"), None);
    assert_eq!(type_badge_text("trailing."), None);
}

#[test]
fn download_button_label_is_localised() {
    for (code, want) in [("en", "Download"), ("es", "Descargar"), ("ca", "Baixa")] {
        assert_eq!(
            common::localizer(code).format("components", "file-download", None),
            want
        );
    }
}

type Downloads = Rc<RefCell<Vec<FileId>>>;

struct DlSrc {
    state: WorkspaceState,
    downloads: Downloads,
}

impl TimelineSource for DlSrc {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn member(&self, u: &UserId) -> Option<&domain::Member> {
        self.state.members.get(u)
    }
    fn self_user(&self) -> Option<&UserId> {
        None
    }
    fn capabilities(&self) -> &Capabilities {
        static CAPS: std::sync::OnceLock<Capabilities> = std::sync::OnceLock::new();
        CAPS.get_or_init(Capabilities::default)
    }
    fn load_older(&mut self, _: ConversationId, _: &mut gpui::App) {}
    fn edit(&mut self, _: domain::MessageId, _: String, _: &mut gpui::App) {}
    fn delete(&mut self, _: domain::MessageId, _: &mut gpui::App) {}
    fn react(&mut self, _: domain::MessageId, _: domain::EmojiRef, _: bool, _: &mut gpui::App) {}
    fn custom_emoji(&self) -> Vec<domain::CustomEmoji> {
        Vec::new()
    }
    fn download(&mut self, f: &FileRef, _: &mut gpui::App) {
        self.downloads.borrow_mut().push(f.id.clone());
    }
}

fn timeline_with(
    cx: &mut TestAppContext,
    files: Vec<FileRef>,
) -> (Downloads, gpui::VisualTestContext) {
    init(cx);
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    let mut m = msg("C1", "1000.000001", "files");
    m.author = Some(uid("u2"));
    m.files = Arc::from(files);
    domain::reduce(
        &mut state,
        domain::Input::HistoryLoaded {
            conversation: cid("C1"),
            page: domain::HistoryPage {
                messages: vec![m],
                has_more: false,
            },
        },
    );
    let downloads: Downloads = Rc::default();
    let d = downloads.clone();
    let src = cx.new(move |_| DlSrc {
        state,
        downloads: d,
    });
    let (_view, vcx) =
        cx.add_window_view(move |_w, cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    (downloads, vcx.clone())
}

fn click(vcx: &mut gpui::VisualTestContext, sel: &'static str) {
    let at = vcx.debug_bounds(sel).expect(sel).center();
    vcx.simulate_click(at, gpui::Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
}

fn ids(d: &Downloads) -> Vec<String> {
    d.borrow().iter().map(|f| f.0.to_string()).collect()
}

#[gpui::test]
fn file_card_download_buttons_download_their_file(cx: &mut TestAppContext) {
    // Two files with the same name still get distinct buttons (selected by id).
    let (downloads, mut vcx) = timeline_with(
        cx,
        vec![
            named("Z", "backup.zip", FileKind::Other),
            named("P", "report.pdf", FileKind::Pdf),
            named("Q", "report.pdf", FileKind::Pdf),
        ],
    );
    for sel in ["file-download-Z", "file-download-P", "file-download-Q"] {
        assert!(vcx.debug_bounds(sel).is_some(), "{sel} is drawn");
    }
    click(&mut vcx, "file-download-Z");
    assert_eq!(ids(&downloads), ["Z"]);
    click(&mut vcx, "file-download-P");
    assert_eq!(ids(&downloads), ["Z", "P"]);
}

#[gpui::test]
fn image_placeholder_download_does_not_reveal_the_image(cx: &mut TestAppContext) {
    let mut img = file(FileKind::Image);
    img.id = FileId(Arc::from("I"));
    let (downloads, mut vcx) = timeline_with(cx, vec![img]);
    click(&mut vcx, "file-download-I");
    assert_eq!(ids(&downloads), ["I"]);
    assert!(
        vcx.debug_bounds("file-download-I").is_some(),
        "still the placeholder card"
    );
}

#[gpui::test]
fn download_button_is_reachable_and_activated_from_the_keyboard(cx: &mut TestAppContext) {
    let (downloads, mut vcx) = timeline_with(cx, vec![named("Z", "backup.zip", FileKind::Other)]);
    vcx.update(|w, cx| w.focus_next(cx));
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(ids(&downloads), ["Z"], "exactly one download");
}
