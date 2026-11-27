//! The workspace badge on the rail: the team icon once loaded, initials until
//! then and when the icon cannot be fetched.

use std::io::Cursor;
use std::sync::Arc;

use components::media::{ImageCache, MediaError, MediaFetcher};
use components::workspace_badge;
use domain::ImageRef;
use futures::future::{BoxFuture, ready};
use gpui::{Context, IntoElement, Render, TestAppContext, Window};
use image::{ImageFormat, Rgba, RgbaImage};

/// `None` never answers: the icon stays loading.
struct Fixed(Option<Result<Vec<u8>, MediaError>>);

impl MediaFetcher for Fixed {
    fn fetch(&self, _: &ImageRef, _: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        match self.0.clone() {
            Some(r) => Box::pin(ready(r)),
            None => Box::pin(std::future::pending()),
        }
    }
}

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(64, 64, Rgba([20, 120, 200, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

struct Host(Option<ImageRef>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        workspace_badge("Example Workspace", self.0.clone())
    }
}

fn icon() -> ImageRef {
    ImageRef::Web("https://example.invalid/icon.png".into())
}

fn setup(cx: &mut TestAppContext, fetch: Option<Result<Vec<u8>, MediaError>>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        cx.set_global(ImageCache::new(Arc::new(Fixed(fetch)), path, 8 << 20));
    });
    dir
}

fn draw(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn shows_initials_without_an_icon(cx: &mut TestAppContext) {
    let _d = setup(cx, Some(Ok(png())));
    let (_v, cx) = cx.add_window_view(|_, _| Host(None));
    draw(cx);
    assert!(cx.debug_bounds("workspace-badge").is_some());
    assert!(cx.debug_bounds("workspace-badge-initials").is_some());
    assert!(cx.debug_bounds("workspace-badge-icon").is_none());
}

#[gpui::test]
fn shows_initials_while_the_icon_loads(cx: &mut TestAppContext) {
    let _d = setup(cx, None);
    let (_v, cx) = cx.add_window_view(|_, _| Host(Some(icon())));
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("workspace-badge-initials").is_some());
    assert!(cx.debug_bounds("workspace-badge-icon").is_none());
}

#[gpui::test]
fn shows_the_icon_once_loaded(cx: &mut TestAppContext) {
    let _d = setup(cx, Some(Ok(png())));
    let (_v, cx) = cx.add_window_view(|_, _| Host(Some(icon())));
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("workspace-badge-icon").is_some(), "icon");
    assert!(cx.debug_bounds("workspace-badge-initials").is_none());
}

#[gpui::test]
fn falls_back_to_initials_when_the_icon_fails(cx: &mut TestAppContext) {
    let _d = setup(cx, Some(Err(MediaError::Refused)));
    let (_v, cx) = cx.add_window_view(|_, _| Host(Some(icon())));
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("workspace-badge-initials").is_some());
    assert!(cx.debug_bounds("workspace-badge-icon").is_none());
}
