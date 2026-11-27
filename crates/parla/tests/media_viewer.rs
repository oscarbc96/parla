//! The media viewer window: keys step through the gallery and zoom, the
//! buttons reach the workspace, and Esc closes the window.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::media::{MediaError, MediaFetcher, ViewerCache, ViewerImage};
use components::media_viewer::{Gallery, ViewerItem, Zoom, source};
use domain::test_support::msg;
use domain::{FileId, FileKind, FileRef, ImageRef, ProviderExt, RemoteUrl, Rendition};
use futures::future::{BoxFuture, ready};
use gpui::{App, TestAppContext, WindowHandle};
use parla::media_viewer::{MediaViewer, ViewerHost};

fn url(s: &str) -> RemoteUrl {
    RemoteUrl(Arc::from(s))
}

fn image(id: &str) -> FileRef {
    FileRef {
        id: FileId(Arc::from(id)),
        name: Arc::from(format!("{id}.png").as_str()),
        title: None,
        mime: Arc::from("image/png"),
        size: 2048,
        kind: FileKind::Image,
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

/// Never answers: the tests put the decoded images in place themselves.
struct Pending;

impl MediaFetcher for Pending {
    fn fetch(&self, _: &ImageRef, _: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        Box::pin(ready(Err(MediaError::Refused)))
    }
}

#[derive(Clone, Default)]
struct Host {
    calls: Rc<RefCell<Vec<String>>>,
}

impl ViewerHost for Host {
    fn download(&self, f: &FileRef, _: &mut App) {
        self.calls.borrow_mut().push(format!("download {}", f.id.0));
    }
    fn open_external(&self, f: &FileRef, _: &mut App) {
        self.calls.borrow_mut().push(format!("open {}", f.id.0));
    }
}

fn open(cx: &mut TestAppContext, host: Host) -> WindowHandle<MediaViewer> {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let items: Vec<ViewerItem> = ["A", "B", "C"]
        .iter()
        .enumerate()
        .map(|(i, id)| ViewerItem {
            file: image(id),
            message: msg("C1", &format!("1000.00000{}", i + 1), "").id,
            sender: "Bob".into(),
        })
        .collect();
    let gallery = Gallery::new(items, 0);
    let dir = std::env::temp_dir().join(format!("parla-viewer-test-{}", std::process::id()));
    let w = cx.add_window(move |_, cx| {
        let host: Rc<dyn ViewerHost> = Rc::new(host);
        MediaViewer::new(gallery, ViewerCache::new(Arc::new(Pending), dir), host, cx)
    });
    cx.run_until_parked();
    w.update(cx, |v, window, cx| {
        for id in ["A", "B", "C"] {
            let src = source(&image(id)).unwrap();
            v.images_mut()
                .insert_ready(&src, ViewerImage::solid(4000, 2000, [40, 90, 160, 255]));
        }
        v.focus(window, cx);
    })
    .unwrap();
    w
}

fn current(cx: &mut TestAppContext, w: WindowHandle<MediaViewer>) -> String {
    w.read_with(cx, |v, _| {
        v.gallery().current().unwrap().file.id.0.to_string()
    })
    .unwrap()
}

#[gpui::test]
fn arrow_keys_step_through_the_gallery_in_order(cx: &mut TestAppContext) {
    let w = open(cx, Host::default());
    cx.simulate_keystrokes(w.into(), "right");
    assert_eq!(current(cx, w), "B");
    cx.simulate_keystrokes(w.into(), "right right");
    assert_eq!(current(cx, w), "C", "stops at the last image");
    cx.simulate_keystrokes(w.into(), "left");
    assert_eq!(current(cx, w), "B");
}

#[gpui::test]
fn zoom_keys_change_the_zoom_and_stepping_resets_it(cx: &mut TestAppContext) {
    let w = open(cx, Host::default());
    let zoom = |cx: &mut TestAppContext| w.read_with(cx, |v, _| v.view_state().zoom).unwrap();
    assert_eq!(zoom(cx), Zoom::Fit);
    cx.simulate_keystrokes(w.into(), "0");
    assert_eq!(zoom(cx), Zoom::Scale(1.0), "0 is actual size");
    cx.simulate_keystrokes(w.into(), "=");
    assert_eq!(zoom(cx), Zoom::Scale(1.5));
    cx.simulate_keystrokes(w.into(), "- -");
    assert_eq!(zoom(cx), Zoom::Scale(0.75));
    cx.simulate_keystrokes(w.into(), "f");
    assert_eq!(zoom(cx), Zoom::Fit);
    cx.simulate_keystrokes(w.into(), "0 right");
    assert_eq!(zoom(cx), Zoom::Fit, "the next image opens fitted");
}

#[gpui::test]
fn download_and_open_buttons_reach_the_workspace(cx: &mut TestAppContext) {
    let host = Host::default();
    let w = open(cx, host.clone());
    let mut vcx = gpui::VisualTestContext::from_window(w.into(), cx);
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    for sel in ["media-viewer-download", "media-viewer-open-external"] {
        let at = vcx.debug_bounds(sel).expect(sel).center();
        vcx.simulate_click(at, gpui::Modifiers::none());
    }
    assert_eq!(*host.calls.borrow(), ["download A", "open A"]);
}

#[gpui::test]
fn escape_closes_the_viewer(cx: &mut TestAppContext) {
    let w = open(cx, Host::default());
    cx.simulate_keystrokes(w.into(), "escape");
    cx.run_until_parked();
    assert!(
        w.update(cx, |_, _, _| ()).is_err(),
        "the window is gone after Esc"
    );
}

/// Files the test pretends were deleted from the workspace.
struct Deleted(Vec<String>);

#[derive(Clone)]
struct Watching {
    host: Host,
    deleted: gpui::Entity<Deleted>,
}

impl ViewerHost for Watching {
    fn download(&self, f: &FileRef, cx: &mut App) {
        self.host.download(f, cx)
    }
    fn open_external(&self, f: &FileRef, cx: &mut App) {
        self.host.open_external(f, cx)
    }
    fn watch(
        &self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<MediaViewer>,
    ) -> Option<gpui::Subscription> {
        Some(cx.observe_in(&self.deleted, window, |v, d, window, cx| {
            let gone = d.read(cx).0.clone();
            v.prune(|it| !gone.contains(&it.file.id.0.to_string()), window, cx);
        }))
    }
}

#[gpui::test]
fn a_deleted_message_leaves_the_viewer_and_the_last_one_closes_it(cx: &mut TestAppContext) {
    use gpui::AppContext as _;
    let w = open(cx, Host::default());
    let deleted = cx.new(|_| Deleted(Vec::new()));
    let host = Watching {
        host: Host::default(),
        deleted: deleted.clone(),
    };
    w.update(cx, |v, window, cx| {
        let g = v.gallery().clone();
        let images = ViewerCache::new(Arc::new(Pending), std::env::temp_dir());
        v.set_gallery(g, images, Rc::new(host), window, cx);
    })
    .unwrap();
    deleted.update(cx, |d, cx| {
        d.0.push("A".into());
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(current(cx, w), "B", "the deleted image is gone");
    deleted.update(cx, |d, cx| {
        d.0.extend(["B".into(), "C".into()]);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(w.update(cx, |_, _, _| ()).is_err(), "nothing left: closed");
}

#[gpui::test]
fn reopening_from_another_workspace_swaps_its_images_and_host_and_close_shuts_it(
    cx: &mut TestAppContext,
) {
    use components::media::ImageCache;
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let dir = tempfile::tempdir().unwrap();
    let gallery = || {
        Gallery::new(
            vec![ViewerItem {
                file: image("A"),
                message: msg("C1", "1000.000001", "").id,
                sender: "Bob".into(),
            }],
            0,
        )
    };
    let (first, second): (Arc<dyn MediaFetcher>, Arc<dyn MediaFetcher>) =
        (Arc::new(Pending), Arc::new(Pending));
    let (host_a, host_b) = (Host::default(), Host::default());
    for (fetcher, host) in [(&first, &host_a), (&second, &host_b)] {
        let (f, h, d) = (fetcher.clone(), host.clone(), dir.path().to_path_buf());
        cx.update(|cx| {
            cx.set_global(ImageCache::new(f, d, 1 << 20));
            parla::media_viewer::open(gallery(), Rc::new(h), cx);
        });
        cx.run_until_parked();
    }
    let w = cx
        .update(|cx| parla::media_viewer::open_window(cx))
        .expect("one viewer");
    assert_eq!(cx.windows().len(), 1, "the viewer is reused");
    w.read_with(cx, |v, _| {
        assert!(
            Arc::ptr_eq(v.images().fetcher(), &second),
            "second workspace's fetcher"
        );
    })
    .unwrap();
    let mut vcx = gpui::VisualTestContext::from_window(w.into(), cx);
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    let at = vcx.debug_bounds("media-viewer-download").unwrap().center();
    vcx.simulate_click(at, gpui::Modifiers::none());
    assert!(host_a.calls.borrow().is_empty());
    assert_eq!(*host_b.calls.borrow(), ["download A"]);
    cx.update(parla::media_viewer::close);
    cx.run_until_parked();
    assert!(
        w.update(cx, |_, _, _| ()).is_err(),
        "closed on switch or sign-out"
    );
}
