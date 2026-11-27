//! The media viewer window: an image from
//! a conversation, fit to the window, with zoom, pan, previous and next across
//! the images of the conversation's loaded window, Copy, Download, Open in the
//! default app and Open in channel. A PDF shows a file card (Parla draws no PDF
//! pages). One viewer window at a time; clicking another image reuses it with
//! that workspace's fetcher and host. Esc closes it, and so do switching
//! workspace and signing out.
//!
//! The window is always dark (the `viewer.*` tokens, the MediaViewer board);
//! only the PDF and failed cards follow the theme. Only the current image and
//! its next neighbour stay decoded ([`components::media::ViewerCache`]).

use std::rc::Rc;

use components::controls::{ButtonKind, button_frame, icon_button, on_activate_app};
use components::media::{ViewerCache, ViewerState, clipboard_image};
use components::media_viewer::{Gallery, ViewState, ViewerItem, source};
use components::{Icon, IconName};
use domain::{FileKind, FileRef, MessageId};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, BoxShadow, Context, FocusHandle, Global, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Render, ScrollEvent,
    SharedString, Subscription, Window, WindowHandle, div, img, point, prelude::*, px, size,
};
use themes::metrics as m;
use themes::{ActiveTheme as _, ParlaTokens, ThemeColors};

/// What the viewer asks of the workspace it was opened from.
pub trait ViewerHost {
    /// Saves the file into the downloads folder.
    fn download(&self, file: &FileRef, cx: &mut App);
    /// Saves the file, then opens it in the system's default app (when allowed).
    fn open_external(&self, file: &FileRef, cx: &mut App);
    /// Shows `message` in the main window.
    fn open_in_channel(&self, _message: &MessageId, _cx: &mut App) {}
    /// Watches the workspace so the viewer drops files whose message was
    /// deleted ([`MediaViewer::prune`]).
    fn watch(&self, _window: &mut Window, _cx: &mut Context<MediaViewer>) -> Option<Subscription> {
        None
    }
}

/// The file is still in the workspace: its message is loaded with it, or the
/// conversation is not loaded at all (nothing says it went away).
fn still_there(state: &domain::WorkspaceState, it: &ViewerItem) -> bool {
    state.windows.get(&it.message.conversation).is_none_or(|w| {
        w.messages().iter().any(|m| {
            m.id == it.message && m.files.iter().any(|f| f.id == it.file.id && !f.tombstone)
        })
    })
}

impl ViewerHost for gpui::WeakEntity<crate::session::WorkspaceSession> {
    fn download(&self, file: &FileRef, cx: &mut App) {
        let _ = self.update(cx, |s, cx| s.download(file, cx));
    }

    fn open_external(&self, file: &FileRef, cx: &mut App) {
        let _ = self.update(cx, |s, cx| s.download_and_open(file, cx));
    }

    fn open_in_channel(&self, message: &MessageId, cx: &mut App) {
        let id = message.clone();
        if self.update(cx, |s, cx| s.jump_to(id, cx)).is_ok() {
            cx.activate(true);
        }
    }

    fn watch(&self, window: &mut Window, cx: &mut Context<MediaViewer>) -> Option<Subscription> {
        let session = self.upgrade()?;
        Some(cx.observe_in(&session, window, |v, s, window, cx| {
            let state = &s.read(cx).state;
            let kept: Vec<bool> = v
                .gallery
                .items()
                .iter()
                .map(|it| still_there(state, it))
                .collect();
            let mut i = 0;
            v.prune(
                move |_| {
                    i += 1;
                    kept[i - 1]
                },
                window,
                cx,
            );
        }))
    }
}

/// Thumbnails shown in the footer strip at once (centred on the current one).
const STRIP_MAX: usize = components::media::VIEWER_THUMBS;

/// Opacity of the strip's other thumbnails (board `.thumb { opacity: .6 }`).
const THUMB_DIM_OPACITY: f32 = 0.6;

pub struct MediaViewer {
    gallery: Gallery,
    view: ViewState,
    images: ViewerCache,
    host: Rc<dyn ViewerHost>,
    focus: FocusHandle,
    /// Last mouse position of a drag that pans the image.
    drag: Option<Point<Pixels>>,
    copied: bool,
    watch: Option<Subscription>,
}

fn cache(v: &mut MediaViewer) -> &mut ViewerCache {
    &mut v.images
}

fn pair(s: gpui::Size<Pixels>) -> (f32, f32) {
    (s.width.as_f32(), s.height.as_f32())
}

/// The theme colours the shared controls read, set to the viewer's dark
/// palette, so icon buttons, hover grounds and focus rings match the window.
fn viewer_colors(colors: &ThemeColors, t: &ParlaTokens) -> ThemeColors {
    let mut c = *colors;
    c.text = t.viewer_text;
    c.text_muted = t.viewer_muted;
    c.element_hover = t.viewer_control_hover;
    c.border = t.viewer_border;
    c.border_focused = t.viewer_focus;
    c.surface_background = t.viewer_bg;
    c
}

/// The focus ring on the dark surface.
fn ring(t: &ParlaTokens) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: t.viewer_focus,
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(0.0),
        spread_radius: m::SPACE_1,
        inset: true,
    }]
}

impl MediaViewer {
    /// A viewer on `gallery`; starts loading the image on screen.
    pub fn new(
        gallery: Gallery,
        images: ViewerCache,
        host: Rc<dyn ViewerHost>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut v = Self {
            gallery,
            view: ViewState::default(),
            images,
            host,
            focus: cx.focus_handle(),
            drag: None,
            copied: false,
            watch: None,
        };
        v.load(cx);
        v
    }

    /// Starts watching the host's workspace for deleted messages.
    pub fn watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let host = self.host.clone();
        self.watch = host.watch(window, cx);
    }

    fn strip_range(&self) -> std::ops::Range<usize> {
        let count = self.gallery.len();
        if count <= 1 {
            return 0..0;
        }
        let start = self
            .gallery
            .index()
            .saturating_sub(STRIP_MAX / 2)
            .min(count.saturating_sub(STRIP_MAX));
        start..(start + STRIP_MAX).min(count)
    }

    fn thumb_src(f: &FileRef) -> Option<domain::ImageRef> {
        f.best_thumb(m::LAYOUT_MEDIA_VIEWER_THUMB_WIDTH.as_f32() as u32 * 2)
            .map(|r| domain::ImageRef::Provider(r.url.clone()))
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let wanted = self.gallery.wanted();
        self.images.want(&wanted, cache, cx);
        let thumbs: Vec<_> = self.gallery.items()[self.strip_range()]
            .iter()
            .filter_map(|it| Self::thumb_src(&it.file))
            .collect();
        self.images.want_thumbs(&thumbs, cache, cx);
    }

    /// Shows another gallery (another image was clicked while open), loaded
    /// through `images` and acting through `host`: those of the workspace the
    /// click came from.
    pub fn set_gallery(
        &mut self,
        gallery: Gallery,
        images: ViewerCache,
        host: Rc<dyn ViewerHost>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.gallery = gallery;
        self.images = images;
        self.host = host;
        self.watch(window, cx);
        self.shown_changed(cx);
    }

    /// Drops the files `keep` refuses (their message was deleted); closes the
    /// window when none is left.
    pub fn prune(
        &mut self,
        keep: impl FnMut(&ViewerItem) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keep = std::cell::RefCell::new(keep);
        if !self.gallery.retain(|it| (keep.borrow_mut())(it)) {
            return;
        }
        if self.gallery.is_empty() {
            window.remove_window();
            return;
        }
        self.shown_changed(cx);
    }

    fn shown_changed(&mut self, cx: &mut Context<Self>) {
        self.view = ViewState::default();
        self.copied = false;
        self.drag = None;
        self.load(cx);
        cx.notify();
    }

    pub fn gallery(&self) -> &Gallery {
        &self.gallery
    }

    pub fn view_state(&self) -> ViewState {
        self.view
    }

    pub fn images(&self) -> &ViewerCache {
        &self.images
    }

    /// Puts a decoded image on screen (visual tests).
    #[doc(hidden)]
    pub fn images_mut(&mut self) -> &mut ViewerCache {
        &mut self.images
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        if self.gallery.step_next() {
            self.shown_changed(cx);
        }
    }

    pub fn prev(&mut self, cx: &mut Context<Self>) {
        if self.gallery.step_prev() {
            self.shown_changed(cx);
        }
    }

    fn select(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.gallery.select(i) {
            self.shown_changed(cx);
        }
    }

    /// Natural size of the image on screen, once decoded.
    fn natural(&self) -> Option<(f32, f32)> {
        let src = source(&self.gallery.current()?.file)?;
        match self.images.state(&src)? {
            ViewerState::Ready(i) => Some((i.natural.0 as f32, i.natural.1 as f32)),
            _ => None,
        }
    }

    /// The area the image is drawn in: the window less header and footer.
    fn stage(window: &Window) -> (f32, f32) {
        let s = pair(window.viewport_size());
        (
            s.0,
            (s.1 - m::LAYOUT_MEDIA_VIEWER_HEADER.as_f32() - m::LAYOUT_MEDIA_VIEWER_FOOTER.as_f32())
                .max(1.0),
        )
    }

    fn zoom(&mut self, f: impl FnOnce(&mut ViewState, (f32, f32), (f32, f32)), window: &Window) {
        if let Some(n) = self.natural() {
            f(&mut self.view, n, Self::stage(window));
        }
    }

    pub fn zoom_in(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.zoom(|v, n, s| v.zoom_in(n, s), window);
        cx.notify();
    }

    pub fn zoom_out(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.zoom(|v, n, s| v.zoom_out(n, s), window);
        cx.notify();
    }

    pub fn actual_size(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.zoom(|v, n, s| v.actual_size(n, s), window);
        cx.notify();
    }

    pub fn fit(&mut self, cx: &mut Context<Self>) {
        self.view.fit();
        cx.notify();
    }

    /// Pans like a drag by `d` (visual tests).
    #[doc(hidden)]
    pub fn pan_for_tests(&mut self, d: (f32, f32), window: &Window, cx: &mut Context<Self>) {
        self.pan_by(d, window, cx);
    }

    fn pan_by(&mut self, d: (f32, f32), window: &Window, cx: &mut Context<Self>) {
        if let Some(n) = self.natural() {
            self.view.pan_by(d, n, Self::stage(window));
            cx.notify();
        }
    }

    fn download(&mut self, cx: &mut Context<Self>) {
        if let Some(it) = self.gallery.current() {
            let f = it.file.clone();
            self.host.download(&f, cx);
        }
    }

    fn open_external(&mut self, cx: &mut Context<Self>) {
        if let Some(it) = self.gallery.current() {
            let f = it.file.clone();
            self.host.open_external(&f, cx);
        }
    }

    fn open_in_channel(&mut self, cx: &mut Context<Self>) {
        if let Some(it) = self.gallery.current() {
            let id = it.message.clone();
            self.host.open_in_channel(&id, cx);
        }
    }

    /// Copies the image on screen (its original bytes from the disk cache).
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(src) = self.gallery.current().and_then(|it| source(&it.file)) else {
            return;
        };
        if !matches!(self.images.state(&src), Some(ViewerState::Ready(_))) {
            return;
        }
        let path = self.images.cached_path(&src);
        cx.spawn(async move |this, cx| {
            let bytes = cx
                .background_executor()
                .spawn(async move { std::fs::read(path).ok() })
                .await;
            let Some(image) = bytes.and_then(clipboard_image) else {
                return;
            };
            let _ = this.update(cx, |v, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_image(&image));
                v.copied = true;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn copied(&self) -> bool {
        self.copied
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        match k.key.as_str() {
            "escape" => window.remove_window(),
            "c" if cmd => self.copy(cx),
            "w" if cmd => window.remove_window(),
            _ if cmd || k.modifiers.alt => return,
            "left" => self.prev(cx),
            "right" => self.next(cx),
            "=" | "+" => self.zoom_in(window, cx),
            "-" => self.zoom_out(window, cx),
            "0" => self.actual_size(window, cx),
            "f" => self.fit(cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// "Today 10:44", "Yesterday 10:44", else the date and time.
    fn when(&self, it: &ViewerItem, l: &i18n::Localizer, cx: &App) -> String {
        use components::timeline_dividers::local_day;
        let (clock, tz) = (self.gallery.clock, &self.gallery.tz);
        let ts = &it.message.ts;
        let time = components::message_row::format_time(ts, clock, tz, l);
        let day = components::message_row::secs(ts).and_then(|s| local_day(s, tz));
        let today = local_day(components::now_secs(cx), tz);
        match (day, today) {
            (Some(d), Some(t)) if d == t => t!(l, "media-viewer-today", time = time).into_owned(),
            (Some(d), Some(t)) if d.tomorrow().ok() == Some(t) => {
                t!(l, "media-viewer-yesterday", time = time).into_owned()
            }
            _ => components::message_row::format_date_time(ts, clock, tz, l),
        }
    }
}

/// An icon button that runs `f` on the viewer. A press never starts a window
/// drag from the header.
fn action(
    id: &'static str,
    icon: IconName,
    label: String,
    colors: &ThemeColors,
    this: &gpui::Entity<MediaViewer>,
    f: impl Fn(&mut MediaViewer, &mut Window, &mut Context<MediaViewer>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let this = this.clone();
    icon_button(id, icon, label, colors, move |window, cx| {
        this.update(cx, |v, cx| f(v, window, cx))
    })
    .debug_selector(move || id.into())
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

impl Render for MediaViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let colors = viewer_colors(&theme_colors, &tokens);
        let l = i18n::gpui::localizer(cx);
        let this = cx.entity();
        let stage = Self::stage(window);
        let item = self.gallery.current().cloned();
        let src = item.as_ref().and_then(|it| source(&it.file));
        let state = src.as_ref().and_then(|s| self.images.state(s));
        let natural = self.natural();
        if let Some(n) = natural {
            // The window may have been resized since the last pan.
            self.view.clamp(n, stage);
        }

        // Header: avatar, file name and meta line, zoom group, actions.
        let mut meta = String::new();
        if let Some(it) = &item {
            let time = self.when(it, &l, cx);
            let size = components::files::format_size(it.file.size, &l);
            meta = match &self.gallery.place {
                Some(p) => t!(
                    l,
                    "media-viewer-meta",
                    sender = it.sender.to_string(),
                    place = p.to_string(),
                    time = time,
                    size = size
                ),
                None => t!(
                    l,
                    "media-viewer-meta-direct",
                    sender = it.sender.to_string(),
                    time = time,
                    size = size
                ),
            }
            .into_owned();
            if let Some((w, h)) = natural {
                meta.push_str(" · ");
                meta.push_str(&t!(l, "media-viewer-dimensions", width = w, height = h));
            }
        }
        let name: SharedString = item
            .as_ref()
            .map(|it| {
                it.file
                    .title
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .unwrap_or(&it.file.name)
                    .to_string()
                    .into()
            })
            .unwrap_or_default();
        let sender = item
            .as_ref()
            .map(|it| it.sender.to_string())
            .unwrap_or_default();
        let is_image = src.is_some();
        let ready = matches!(state, Some(ViewerState::Ready(_)));
        let left = if cfg!(target_os = "macos") {
            m::LAYOUT_TITLEBAR_LIGHTS
        } else {
            m::SPACE_7
        };

        let percent = natural.map_or(100, |n| self.view.percent(n, stage));
        let actual_label = t!(l, "media-viewer-actual-size").into_owned();
        let actual = {
            let this = this.clone();
            let (hover, muted) = (colors.element_hover, colors.text_muted);
            let r = ring(&tokens);
            on_activate_app(
                div()
                    .id("media-viewer-actual-size")
                    .debug_selector(|| "media-viewer-actual-size".into())
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(m::LAYOUT_TITLEBAR_BUTTON)
                    .rounded(m::RADIUS_MD)
                    .cursor_pointer()
                    .tab_stop(true)
                    .focusable()
                    .text_size(m::FONT_SIZE_XS)
                    .font_weight(m::FONT_WEIGHT_SEMIBOLD)
                    .text_color(muted)
                    .hover(move |s| s.bg(hover))
                    .focus_visible(move |s| s.shadow(r.clone()))
                    .tooltip(components::reactions::tooltip_shared(actual_label.into()))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child("1:1"),
                move |window, cx| this.update(cx, |v, cx| v.actual_size(window, cx)),
            )
        };
        let zoom_group = div()
            .flex()
            .items_center()
            .gap(m::SPACE_1)
            .p(m::SPACE_1)
            .rounded(m::RADIUS_MD)
            .bg(tokens.viewer_control_hover)
            .child(action(
                "media-viewer-zoom-out",
                IconName::ChevDown,
                t!(l, "media-viewer-zoom-out").into_owned(),
                &colors,
                &this,
                |v, w, cx| v.zoom_out(w, cx),
            ))
            .child(
                div()
                    .debug_selector(|| "media-viewer-percent".into())
                    .w(m::LAYOUT_MEDIA_VIEWER_ZOOM_LABEL)
                    .text_center()
                    .text_size(m::FONT_SIZE_XS)
                    .font_weight(m::FONT_WEIGHT_SEMIBOLD)
                    .child(t!(l, "media-viewer-percent", value = percent).into_owned()),
            )
            .child(action(
                "media-viewer-zoom-in",
                IconName::Plus,
                t!(l, "media-viewer-zoom-in").into_owned(),
                &colors,
                &this,
                |v, w, cx| v.zoom_in(w, cx),
            ))
            .child(actual)
            .child(action(
                "media-viewer-fit",
                IconName::Grid,
                t!(l, "media-viewer-fit").into_owned(),
                &colors,
                &this,
                |v, _, cx| v.fit(cx),
            ));

        // A labelled button: on the dark header (`dark`) or the themed card.
        let text_button =
            |id: &'static str,
             icon: IconName,
             label: String,
             dark: bool,
             f: fn(&mut MediaViewer, &mut Context<MediaViewer>)| {
                let this = this.clone();
                let c = if dark { colors } else { theme_colors };
                let hover = c.element_hover;
                let r = if dark {
                    ring(&tokens)
                } else {
                    components::controls::ring(&theme_colors)
                };
                on_activate_app(
                    button_frame(id, ButtonKind::Secondary, true, &c)
                        .min_h(m::LAYOUT_TITLEBAR_BUTTON)
                        .px(m::SPACE_5)
                        .py(m::SPACE_0)
                        .gap(m::SPACE_2)
                        .rounded(m::RADIUS_SM)
                        .text_size(m::FONT_SIZE_XS)
                        .when(dark, |d| {
                            d.bg(tokens.viewer_control_hover)
                                .border_color(tokens.viewer_border)
                                .text_color(tokens.viewer_text)
                        })
                        .hover(move |s| s.bg(hover))
                        .focus_visible(move |s| s.shadow(r.clone()))
                        .debug_selector(move || id.into())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(Icon::new(icon).size(m::ICON_SIZE_SM).color(c.text_muted))
                        .child(label),
                    move |_, cx| this.update(cx, f),
                )
            };

        let header = crate::titlebar::draggable(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(m::SPACE_5)
                .h(m::LAYOUT_MEDIA_VIEWER_HEADER)
                .pl(left)
                .pr(m::SPACE_5)
                .bg(tokens.viewer_bg)
                .text_color(tokens.viewer_text)
                .border_b_1()
                .border_color(tokens.viewer_border),
        )
        .child(components::message_row::avatar(&sender, cx.theme(), None))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(
                    div()
                        .debug_selector(|| "media-viewer-name".into())
                        .truncate()
                        .text_size(m::FONT_SIZE_SM)
                        .font_weight(m::FONT_WEIGHT_SEMIBOLD)
                        .child(name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(m::FONT_SIZE_XS)
                        .text_color(tokens.viewer_muted)
                        .child(meta),
                ),
        )
        .when(is_image, |d| d.child(zoom_group))
        .when(is_image && ready, |d| {
            let label = if self.copied {
                t!(l, "media-viewer-copied")
            } else {
                t!(l, "media-viewer-copy")
            };
            d.child(action(
                "media-viewer-copy",
                IconName::Copy,
                label.into_owned(),
                &colors,
                &this,
                |v, _, cx| v.copy(cx),
            ))
        })
        .child(action(
            "media-viewer-download",
            IconName::Download,
            t!(l, "media-viewer-download").into_owned(),
            &colors,
            &this,
            |v, _, cx| v.download(cx),
        ))
        .child(action(
            "media-viewer-open-external",
            IconName::External,
            t!(l, "media-viewer-open-external").into_owned(),
            &colors,
            &this,
            |v, _, cx| v.open_external(cx),
        ))
        .child(text_button(
            "media-viewer-open-in-channel",
            IconName::Message,
            t!(l, "media-viewer-open-in-channel").into_owned(),
            true,
            |v, cx| v.open_in_channel(cx),
        ))
        .child(action(
            "media-viewer-close",
            IconName::X,
            t!(l, "media-viewer-close").into_owned(),
            &colors,
            &this,
            |_, w, _| w.remove_window(),
        ));

        // Stage: the image (or a loading, failed or PDF card), with previous and next.
        let card = |title: SharedString, line: String, note: String| -> AnyElement {
            div()
                .debug_selector(|| "media-viewer-card".into())
                .flex()
                .flex_col()
                .gap(m::SPACE_3)
                .w(m::LAYOUT_MEDIA_VIEWER_CARD)
                .p(m::SPACE_6)
                .rounded(m::RADIUS_LG)
                .border_1()
                .border_color(theme_colors.border)
                .bg(theme_colors.elevated_surface_background)
                .text_color(theme_colors.text)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(m::SPACE_3)
                        .child(
                            Icon::new(IconName::File)
                                .size(m::ICON_SIZE_LG)
                                .color(theme_colors.text_muted),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(m::FONT_SIZE_SM)
                                        .font_weight(m::FONT_WEIGHT_SEMIBOLD)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_size(m::FONT_SIZE_XS)
                                        .text_color(tokens.text_subtle)
                                        .child(line),
                                ),
                        ),
                )
                .child(
                    div()
                        .text_size(m::FONT_SIZE_XS)
                        .text_color(theme_colors.text_muted)
                        .child(note),
                )
                .child(div().flex().child(text_button(
                    "media-viewer-card-open",
                    IconName::External,
                    t!(l, "media-viewer-open-external").into_owned(),
                    false,
                    |v, cx| v.open_external(cx),
                )))
                .into_any_element()
        };
        let size_text = item
            .as_ref()
            .map(|it| components::files::format_size(it.file.size, &l))
            .unwrap_or_default();
        let body: AnyElement = match (&item, &state) {
            (Some(it), _) if it.file.kind == FileKind::Pdf || src.is_none() => card(
                name.clone(),
                t!(l, "media-viewer-pdf", size = size_text.clone()).into_owned(),
                t!(l, "media-viewer-pdf-note").into_owned(),
            ),
            (_, Some(ViewerState::Ready(i))) => {
                let n = (i.natural.0 as f32, i.natural.1 as f32);
                let ((w, h), (x, y)) = self.view.layout(n, stage);
                img(i.image.clone())
                    .debug_selector(|| "media-viewer-image".into())
                    .absolute()
                    // design: allow intrinsic image size and pan offset
                    .left(px(x))
                    .top(px(y))
                    .w(px(w))
                    .h(px(h))
                    .rounded(m::RADIUS_SM)
                    .into_any_element()
            }
            (_, Some(ViewerState::Failed)) => card(
                name.clone(),
                size_text.clone(),
                t!(l, "media-viewer-failed").into_owned(),
            ),
            _ => div()
                .debug_selector(|| "media-viewer-loading".into())
                .text_size(m::FONT_SIZE_SM)
                .text_color(tokens.viewer_muted)
                .child(t!(l, "media-viewer-loading").into_owned())
                .into_any_element(),
        };
        let can_pan = natural.is_some_and(|n| self.view.can_pan(n, stage));
        let nav = |id: &'static str, icon, label: String, prev: bool| {
            action(id, icon, label, &colors, &this, move |v, _, cx| {
                if prev { v.prev(cx) } else { v.next(cx) }
            })
            .absolute()
            .top(px((stage.1 - m::LAYOUT_MEDIA_VIEWER_NAV.as_f32()) / 2.0)) // design: allow centred on the stage
            .size(m::LAYOUT_MEDIA_VIEWER_NAV)
            .rounded_full()
            .border_1()
            .border_color(tokens.viewer_border)
            .bg(tokens.viewer_nav_bg)
        };
        let stage_el = div()
            .id("media-viewer-stage")
            .debug_selector(|| "media-viewer-stage".into())
            .relative()
            .flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .bg(tokens.viewer_stage)
            .when(can_pan, |d| d.cursor_grab())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|v, ev: &MouseDownEvent, _, _| v.drag = Some(ev.position)),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(|v, _, _, _| v.drag = None))
            .on_mouse_move(cx.listener(|v, ev: &MouseMoveEvent, window, cx| {
                let Some(last) = v.drag else { return };
                if ev.pressed_button != Some(MouseButton::Left) {
                    v.drag = None;
                    return;
                }
                v.drag = Some(ev.position);
                let d = ev.position - last;
                v.pan_by((d.x.as_f32(), d.y.as_f32()), window, cx);
            }))
            .on_scroll(cx.listener(|v, ev: &ScrollEvent, window, cx| {
                let d = ev.delta.pixel_delta(m::FONT_SIZE_MD);
                v.pan_by((d.x.as_f32(), d.y.as_f32()), window, cx);
            }))
            .child(body)
            .when(self.gallery.has_prev(), |d| {
                d.child(
                    nav(
                        "media-viewer-prev",
                        IconName::ChevLeft,
                        t!(l, "media-viewer-prev").into_owned(),
                        true,
                    )
                    .left(m::SPACE_8),
                )
            })
            .when(self.gallery.has_next(), |d| {
                d.child(
                    nav(
                        "media-viewer-next",
                        IconName::ChevRight,
                        t!(l, "media-viewer-next").into_owned(),
                        false,
                    )
                    .right(m::SPACE_8),
                )
            });

        // Footer: position, the thumbnail strip, the key hint.
        let (index, count) = (self.gallery.index() + 1, self.gallery.len());
        let position = match &self.gallery.place {
            Some(p) => t!(
                l,
                "media-viewer-position-in",
                index = index,
                count = count,
                place = p.to_string()
            ),
            None => t!(l, "media-viewer-position", index = index, count = count),
        }
        .into_owned();
        let mut strip = div().flex().flex_1().justify_center().gap(m::SPACE_4);
        for i in self.strip_range() {
            let it = &self.gallery.items()[i];
            let on = i == self.gallery.index();
            let thumb = Self::thumb_src(&it.file).and_then(|s| match self.images.thumb(&s) {
                Some(ViewerState::Ready(t)) => Some(t.image),
                _ => None,
            });
            let this = this.clone();
            let r = ring(&tokens);
            strip = strip.child(on_activate_app(
                div()
                    .id(("media-viewer-thumb", i))
                    .debug_selector(move || format!("media-viewer-thumb-{i}"))
                    .flex_none()
                    .w(m::LAYOUT_MEDIA_VIEWER_THUMB_WIDTH)
                    .h(m::LAYOUT_MEDIA_VIEWER_THUMB_HEIGHT)
                    .overflow_hidden()
                    .rounded(m::RADIUS_SM)
                    .border_2()
                    .border_color(if on {
                        tokens.signal
                    } else {
                        gpui::transparent_black()
                    })
                    .bg(tokens.viewer_stage)
                    .when(!on, |d| d.opacity(THUMB_DIM_OPACITY))
                    .cursor_pointer()
                    .tab_stop(true)
                    .focusable()
                    .focus_visible(move |s| s.shadow(r.clone()))
                    .tooltip(components::reactions::tooltip_shared(
                        it.file.name.to_string().into(),
                    ))
                    // overflow_hidden clips to a rectangle: round the picture too.
                    .children(thumb.map(|t| img(t).size_full().rounded(m::RADIUS_SM))),
                move |_, cx| this.update(cx, |v, cx| v.select(i, cx)),
            ));
        }
        let kbd = |k: &'static str| {
            div()
                .px(m::SPACE_2)
                .rounded(m::RADIUS_SM)
                .border_1()
                .border_color(tokens.viewer_border)
                .text_color(tokens.viewer_muted)
                .child(components::keycap::glyph_row(
                    &k.into(),
                    m::ICON_SIZE_SM,
                    tokens.viewer_muted,
                    cx,
                ))
        };
        let footer = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(m::SPACE_4)
            .h(m::LAYOUT_MEDIA_VIEWER_FOOTER)
            .px(m::SPACE_7)
            .bg(tokens.viewer_bg)
            .border_t_1()
            .border_color(tokens.viewer_border)
            .text_size(m::FONT_SIZE_XS)
            .child(
                div()
                    .debug_selector(|| "media-viewer-position".into())
                    .w(m::LAYOUT_MEDIA_VIEWER_META)
                    .truncate()
                    .text_color(tokens.viewer_muted)
                    .child(position),
            )
            .child(strip)
            .child(
                div()
                    .w(m::LAYOUT_MEDIA_VIEWER_META)
                    .flex()
                    .justify_end()
                    .gap(m::SPACE_2)
                    .when(count > 1, |d| {
                        d.child(kbd(components::keycap::LEFT))
                            .child(kbd(components::keycap::RIGHT))
                    }),
            );

        div()
            .id("media-viewer")
            .track_focus(&self.focus)
            .key_context("MediaViewer")
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .size_full()
            .bg(tokens.viewer_bg)
            .text_color(tokens.viewer_text)
            .child(header)
            .child(stage_el)
            .child(footer)
    }
}

/// The open viewer window, reused for the next image clicked.
#[derive(Default)]
struct OpenViewer(Option<WindowHandle<MediaViewer>>);

impl Global for OpenViewer {}

/// The viewer window, when one is open.
pub fn open_window(cx: &App) -> Option<WindowHandle<MediaViewer>> {
    cx.try_global::<OpenViewer>().and_then(|o| o.0)
}

/// Closes the viewer (workspace switch, sign-out, workspace removal).
pub fn close(cx: &mut App) {
    if let Some(h) = cx.try_global::<OpenViewer>().and_then(|o| o.0) {
        let _ = h.update(cx, |_, window, _| window.remove_window());
        cx.set_global(OpenViewer(None));
    }
}

/// Opens the viewer on `gallery`, or shows it in the viewer already open.
/// Images load through the active workspace's [`components::media::ImageCache`]
/// (its fetcher and disk cache); without one nothing opens.
pub fn open(gallery: Gallery, host: Rc<dyn ViewerHost>, cx: &mut App) {
    if gallery.is_empty() {
        return;
    }
    let images = |cx: &App| {
        cx.try_global::<components::media::ImageCache>().map(|c| {
            ViewerCache::with_budget(c.fetcher(), c.disk_dir().to_path_buf(), c.disk_budget())
        })
    };
    let Some(first) = images(cx) else {
        return;
    };
    if let Some(h) = open_window(cx) {
        let (g, host2) = (gallery.clone(), host.clone());
        let mut fresh = Some(first);
        let reused = h
            .update(cx, |v, window, cx| {
                if let Some(images) = fresh.take() {
                    v.set_gallery(g, images, host2, window, cx);
                }
                window.activate_window();
                v.focus(window, cx);
            })
            .is_ok();
        if reused {
            return;
        }
        let Some(again) = images(cx) else { return };
        return open_new(gallery, again, host, cx);
    }
    open_new(gallery, first, host, cx);
}

fn open_new(gallery: Gallery, images: ViewerCache, host: Rc<dyn ViewerHost>, cx: &mut App) {
    let bounds = Bounds::centered(
        None,
        size(m::LAYOUT_WINDOW_MEDIA_WIDTH, m::LAYOUT_WINDOW_MEDIA_HEIGHT),
        cx,
    );
    let opened = cx.open_window(
        crate::window_options::media_viewer(bounds),
        move |window, cx| {
            let v = cx.new(|cx| MediaViewer::new(gallery, images, host, cx));
            v.update(cx, |v, cx| {
                v.watch(window, cx);
                v.focus(window, cx);
            });
            v
        },
    );
    if let Ok(h) = opened {
        cx.set_global(OpenViewer(Some(h)));
    }
}
