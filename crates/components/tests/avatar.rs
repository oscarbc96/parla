//! Avatars: the picture once loaded, initials while it loads, when it fails
//! and when the person has none.

use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use components::media::{ImageCache, MediaError, MediaFetcher};
use components::message_row::avatar_sized;
use domain::ImageRef;
use futures::future::{BoxFuture, ready};
use gpui::{Context, IntoElement, Render, TestAppContext, Window};
use image::{ImageFormat, Rgba, RgbaImage};
use themes::ActiveTheme as _;

/// `None` never answers; counts the fetches.
struct Fixed(Option<Result<Vec<u8>, MediaError>>, Arc<AtomicUsize>);

impl MediaFetcher for Fixed {
    fn fetch(&self, _: &ImageRef, _: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        self.1.fetch_add(1, Ordering::SeqCst);
        match self.0.clone() {
            Some(r) => Box::pin(ready(r)),
            None => Box::pin(std::future::pending()),
        }
    }
}

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(72, 72, Rgba([20, 120, 200, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

struct Host(Option<ImageRef>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        avatar_sized(
            "Ada Lovelace",
            cx.theme(),
            themes::metrics::LAYOUT_MSG_AVATAR,
            self.0.as_ref(),
        )
    }
}

fn pic() -> ImageRef {
    ImageRef::Web("https://example.invalid/ada.png".into())
}

fn setup(
    cx: &mut TestAppContext,
    fetch: Option<Result<Vec<u8>, MediaError>>,
) -> (tempfile::TempDir, Arc<AtomicUsize>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let n = Arc::new(AtomicUsize::new(0));
    let fetcher = Fixed(fetch, n.clone());
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        cx.set_global(ImageCache::new(Arc::new(fetcher), path, 8 << 20));
    });
    (dir, n)
}

fn draw(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn initials_without_a_picture_and_no_fetch(cx: &mut TestAppContext) {
    let (_d, n) = setup(cx, Some(Ok(png())));
    let (_v, cx) = cx.add_window_view(|_, _| Host(None));
    draw(cx);
    assert!(cx.debug_bounds("avatar-initials").is_some());
    assert!(cx.debug_bounds("avatar-image").is_none());
    assert_eq!(n.load(Ordering::SeqCst), 0);
}

#[gpui::test]
fn initials_while_the_picture_loads(cx: &mut TestAppContext) {
    let (_d, _n) = setup(cx, None);
    let (_v, cx) = cx.add_window_view(|_, _| Host(Some(pic())));
    draw(cx);
    draw(cx);
    let b = cx.debug_bounds("avatar-initials").unwrap();
    assert_eq!(b.size.width, themes::metrics::LAYOUT_MSG_AVATAR);
    assert_eq!(b.size.height, themes::metrics::LAYOUT_MSG_AVATAR);
    assert!(cx.debug_bounds("avatar-image").is_none());
}

#[gpui::test]
fn the_picture_replaces_the_initials_once_loaded_at_the_same_size(cx: &mut TestAppContext) {
    let (_d, n) = setup(cx, Some(Ok(png())));
    let (_v, cx) = cx.add_window_view(|_, _| Host(Some(pic())));
    draw(cx);
    draw(cx);
    draw(cx);
    let after = cx.debug_bounds("avatar-image").expect("picture");
    assert!(cx.debug_bounds("avatar-initials").is_none());
    assert_eq!(after.size.height, themes::metrics::LAYOUT_MSG_AVATAR);
    assert_eq!(after.size.width, themes::metrics::LAYOUT_MSG_AVATAR);
    assert_eq!(n.load(Ordering::SeqCst), 1, "one fetch");
}

#[gpui::test]
fn initials_stay_when_the_picture_fails(cx: &mut TestAppContext) {
    let (_d, _n) = setup(cx, Some(Err(MediaError::Refused)));
    let (_v, cx) = cx.add_window_view(|_, _| Host(Some(pic())));
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("avatar-initials").is_some());
    assert!(cx.debug_bounds("avatar-image").is_none());
}

#[gpui::test]
fn initials_without_an_image_cache(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (_v, cx) = cx.add_window_view(|_, _| Host(Some(pic())));
    draw(cx);
    assert!(cx.debug_bounds("avatar-initials").is_some());
}
