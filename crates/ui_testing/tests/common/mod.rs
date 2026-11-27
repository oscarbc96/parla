//! Shared spike code: the exact construction sequence for offscreen capture.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Result, bail};
use components::{Sidebar, SidebarRow, SidebarSource, build_rows};
use domain::{Conversation, ConversationId, ConversationKind, ProviderExt, WorkspaceState};
use gpui::{
    App, AppContext as _, AssetSource, HeadlessAppContext, SharedString, WindowHandle, px, size,
};
use image::RgbaImage;
use settings::{Settings, SidebarSettings, WorkspaceKey};

pub const LOGICAL_W: f32 = 960.0;
pub const LOGICAL_H: f32 = 600.0;

/// Synthetic SVG served from memory, to probe SVG rendering.
pub const PROBE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="10" fill="#000"/></svg>"##;

pub struct ProbeAssets;

impl AssetSource for ProbeAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok((path == "probe.svg").then(|| Cow::Borrowed(PROBE_SVG.as_bytes())))
    }
    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

pub struct Src {
    pub rows: Vec<SidebarRow>,
}

impl SidebarSource for Src {
    fn rows(&self, _cx: &App) -> Vec<SidebarRow> {
        self.rows.clone()
    }
    fn open(&mut self, _id: ConversationId, _cx: &mut App) {}
}

fn conv(id: &str, kind: ConversationKind, name: &str, unread: bool) -> Conversation {
    Conversation {
        id: ConversationId(Arc::from(id)),
        kind,
        name: Arc::from(name),
        peer: None,
        is_member: true,
        is_archived: false,
        is_open: true,
        is_muted: false,
        last_read: None,
        latest: None,
        unread,
        mentions: 0,
        participants: Arc::from([]),
        member_ids: Arc::from([]),
        topic: Default::default(),
        member_count: None,
        purpose: Default::default(),
        ext: ProviderExt::none(),
    }
}

/// Synthetic rows (neutral names only).
pub fn rows() -> Vec<SidebarRow> {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    for (i, n) in ["general", "design", "random", "build-status", "support"]
        .iter()
        .enumerate()
    {
        let c = conv(
            &format!("C{i:03}"),
            ConversationKind::Channel,
            n,
            i % 2 == 0,
        );
        s.conversations.insert(c.id.clone(), c);
    }
    let l = i18n::Localizer::new(
        i18n::chain(
            &i18n::LocalePref::Code("en".parse().expect("language tag")),
            &[],
            i18n::LocaleSet::embedded(),
        ),
        &[components::catalog()],
    )
    .expect("catalogs build");
    build_rows(&s, None, &SidebarSettings::default(), &HashSet::new(), &l)
}

pub use ui_testing::text_system as platform_text_system;

/// Real text system plus the platform's headless renderer (Metal on macOS,
/// wgpu on Linux; `None` on Windows at this rev).
pub fn new_cx() -> HeadlessAppContext {
    let cx =
        HeadlessAppContext::with_platform(platform_text_system(), Arc::new(ProbeAssets), || {
            gpui_platform::current_headless_renderer()
        });
    cx.text_system()
        .add_fonts(vec![Cow::Borrowed(ui_testing::INTER_REGULAR)])
        .expect("add Inter");
    cx
}

/// Hard failure instead of GPUI's silent fallback for a missing family.
pub fn require_family(cx: &HeadlessAppContext, family: &str) -> Result<()> {
    if cx
        .text_system()
        .all_font_names()
        .iter()
        .any(|n| n == family)
    {
        Ok(())
    } else {
        bail!("font family {family:?} is not available in the text system")
    }
}

/// Installs themes (explicit light) and key bindings.
pub fn init(cx: &mut HeadlessAppContext) {
    cx.update(|app| {
        let mut s = Settings::default();
        s.theme.mode = settings::ThemeMode::Light;
        themes::init(app, &s);
        components::init(app);
    });
}

pub fn open_sidebar(cx: &mut HeadlessAppContext) -> WindowHandle<Sidebar<Src>> {
    let rows = rows();
    cx.open_window(size(px(LOGICAL_W), px(LOGICAL_H)), move |_w, app| {
        let src = app.new(|_| Src { rows });
        app.new(|cx| Sidebar::new(src, cx))
    })
    .expect("open window")
}

/// Draws the window and captures it.
pub fn capture(cx: &mut HeadlessAppContext, w: gpui::AnyWindowHandle) -> Result<RgbaImage> {
    cx.update_window(w, |_, window, app| {
        let needed = window.draw(app);
        needed.clear(app);
    })?;
    cx.capture_screenshot(w)
}

/// Fraction of pixels that differ from the top-left pixel.
pub fn non_background_fraction(img: &RgbaImage) -> f64 {
    let bg = *img.get_pixel(0, 0);
    let n = img.pixels().filter(|p| **p != bg).count();
    n as f64 / (img.width() as f64 * img.height() as f64)
}

/// Scale experiment: `Window::set_scale_factor` only (the TestWindow keeps 2.0).
pub fn capture_at_scale(
    cx: &mut HeadlessAppContext,
    w: gpui::AnyWindowHandle,
    scale: f32,
) -> Result<RgbaImage> {
    cx.update_window(w, |_, window, _| window.set_scale_factor(scale))?;
    capture(cx, w)
}

/// A window holding a generated PNG-decoded image and an in-memory SVG.
pub struct Media;

impl gpui::Render for Media {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _, div, img, rgb, svg};
        let mut raster = RgbaImage::new(64, 64);
        for (x, y, p) in raster.enumerate_pixels_mut() {
            *p = image::Rgba([(x * 4) as u8, (y * 4) as u8, 128, 255]);
        }
        let frame = image::Frame::new(raster);
        let render = Arc::new(gpui::RenderImage::new(smallvec::smallvec![frame]));
        div()
            .size_full()
            .bg(rgb(0xffffff))
            .child(img(render).size(px(128.0)))
            .child(
                svg()
                    .path("probe.svg")
                    .size(px(64.0))
                    .text_color(rgb(0xcc0000)),
            )
    }
}

pub fn open_media(cx: &mut HeadlessAppContext) -> WindowHandle<Media> {
    cx.open_window(size(px(LOGICAL_W), px(LOGICAL_H)), |_w, app| {
        app.new(|_| Media)
    })
    .expect("open window")
}
