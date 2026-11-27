//! A conversation's Files tab: the files shared in it, newest first. Each row
//! has the file's type icon (or its thumbnail), name, size, sharer and date.
//! Images and PDFs open in the media viewer; other files download. Pages come
//! from the host ([`FilesSource`]); the list keeps at most
//! [`FILES_LOADED_MAX`] files and asks for the next page only when the user
//! scrolls near its end.

use std::ops::Range;
use std::sync::Arc;

use domain::{ConversationId, FileHit, FileKind, FileRef, ImageRef, Page, WorkspaceState};
use gpui::{
    App, Context, Entity, FocusHandle, IntoElement, Render, SharedString, UniformListScrollHandle,
    Window, div, prelude::*, uniform_list,
};
use settings::InlineImages;
use themes::ActiveTheme;
use themes::metrics::{
    FONT_SIZE_MD, FONT_SIZE_XS, FONT_WEIGHT_SEMIBOLD, ICON_SIZE_LG, LAYOUT_MSG_AVATAR, RADIUS_MD,
    SPACE_1, SPACE_2, SPACE_4, SPACE_5, SPACE_7,
};

use crate::icon::{Icon, IconName};
use crate::media::MediaImage;
use crate::media_viewer::{Gallery, ViewerItem, viewable};

/// Files asked for per page.
pub const FILES_PAGE: usize = 50;

/// Most files the tab holds; past it no further page is asked for.
pub const FILES_LOADED_MAX: usize = 500;

/// Rows from the end of the list at which the next page is asked for.
const PREFETCH_ROWS: usize = 8;

/// Answers a [`FilesSource::load_files`] call: a page of files, or why it
/// failed. Runs on the main thread.
pub type FilesDone = Box<dyn FnOnce(Result<Page<FileHit>, String>, &mut App)>;

/// What the Files tab needs from the session.
pub trait FilesSource: 'static {
    /// Members (sharer names) and the workspace's conversations.
    fn files_state(&self) -> &WorkspaceState;
    /// Loads up to `limit` files of `c` older than `cursor` (newest first;
    /// `None` is the newest page), then calls `done`. `done` must not run
    /// inside this call.
    fn load_files(
        &mut self,
        c: ConversationId,
        cursor: Option<Arc<str>>,
        limit: usize,
        done: FilesDone,
        cx: &mut App,
    );
    /// Opens the media viewer on `gallery`.
    fn view_file(&mut self, gallery: Gallery, cx: &mut App);
    /// Downloads `file` (or opens it, as the host does for attachments).
    fn download_file(&mut self, file: &FileRef, cx: &mut App);
    /// Opens the code viewer on the current file of `gallery`; downloads it
    /// where there is no viewer.
    fn view_code(&mut self, gallery: Gallery, cx: &mut App) {
        if let Some(it) = gallery.current() {
            let file = it.file.clone();
            self.download_file(&file, cx);
        }
    }
    /// The `messages.inline_images` setting: `always` shows every image's
    /// thumbnail, `on_click` only those already cached, `never` none.
    fn files_inline_images(&self, _cx: &App) -> InlineImages {
        InlineImages::OnClick
    }
}

/// Where the tab's first page is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesLoad {
    Loading,
    Failed,
    Ready,
}

/// The icon of a file type.
pub fn kind_icon(kind: FileKind) -> IconName {
    match kind {
        FileKind::Image => IconName::Image,
        FileKind::Video => IconName::Video,
        FileKind::Audio => IconName::Volume,
        FileKind::Snippet => IconName::Code,
        FileKind::Pdf | FileKind::Other => IconName::File,
    }
}

/// What a click on hit `i` of `hits` (newest first) does: the viewer on the
/// listed images, oldest first, showing the clicked one; a PDF on its own; a
/// snippet or code file in the code viewer; anything else downloads.
pub enum FileOpen {
    View(Gallery),
    Code(Gallery),
    Download(FileRef),
}

pub fn open_for(
    hits: &[FileHit],
    i: usize,
    sender: impl Fn(&FileHit) -> SharedString,
) -> Option<FileOpen> {
    let hit = hits.get(i)?;
    let f = &hit.file;
    if f.tombstone {
        return None;
    }
    let item = |h: &FileHit| ViewerItem {
        file: h.file.clone(),
        message: domain::MessageId {
            conversation: h
                .conversation
                .clone()
                .unwrap_or_else(|| ConversationId(Arc::from(""))),
            ts: h
                .ts
                .clone()
                .unwrap_or_else(|| domain::MessageTs(Arc::from(""))),
        },
        sender: sender(h),
    };
    if !viewable(f) {
        if crate::code_viewer::is_code(f) {
            return Some(FileOpen::Code(Gallery::new(vec![item(hit)], 0)));
        }
        return Some(FileOpen::Download(f.clone()));
    }
    if f.kind == FileKind::Pdf {
        return Some(FileOpen::View(Gallery::new(vec![item(hit)], 0)));
    }
    let mut items = Vec::new();
    let mut at = 0;
    for (n, h) in hits.iter().enumerate().rev() {
        if h.file.kind != FileKind::Image || !viewable(&h.file) {
            continue;
        }
        if n == i {
            at = items.len();
        }
        items.push(item(h));
    }
    Some(FileOpen::View(Gallery::new(items, at)))
}

pub struct FilesTab<S: FilesSource> {
    session: Entity<S>,
    conversation: ConversationId,
    hits: Vec<FileHit>,
    next: Option<Arc<str>>,
    load: FilesLoad,
    /// A next page is in flight.
    loading_more: bool,
    /// The last next page failed: nothing more is asked for until the user
    /// picks Retry at the end of the list (or the tab reloads).
    more_failed: bool,
    /// Each row's line under the name (size, sharer, date), built once when
    /// its page arrives, parallel to `hits`.
    metas: Vec<SharedString>,
    /// Bumped on each reload so an answer for an older load is dropped.
    generation: u64,
    cursor: usize,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// Whether each listed image's thumbnail is in the disk cache, checked
    /// once per file (at most [`FILES_LOADED_MAX`] entries; cleared on reload).
    /// Viewing an image does not change it: the viewer fetches the original
    /// or the largest rendition, never this preview, and the tab is built
    /// anew on each visit, so a preview the timeline cached meanwhile shows
    /// next time.
    cached_thumbs: std::collections::HashMap<domain::FileId, bool>,
}

impl<S: FilesSource> FilesTab<S> {
    pub fn new(session: Entity<S>, conversation: ConversationId, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            session,
            conversation,
            hits: Vec::new(),
            next: None,
            load: FilesLoad::Loading,
            loading_more: false,
            more_failed: false,
            metas: Vec::new(),
            generation: 0,
            cursor: 0,
            focus: cx.focus_handle().tab_stop(true),
            scroll: UniformListScrollHandle::new(),
            cached_thumbs: std::collections::HashMap::new(),
        };
        this.reload(cx);
        this
    }

    pub fn conversation(&self) -> &ConversationId {
        &self.conversation
    }

    pub fn load(&self) -> FilesLoad {
        self.load
    }

    /// The row the keyboard is on.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The list's focus (arrow keys move, Enter opens).
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Names of the listed files, in order (a test hook).
    pub fn file_names(&self) -> Vec<String> {
        self.hits.iter().map(|h| display_name(&h.file)).collect()
    }

    /// An older page may still be asked for.
    pub fn has_more(&self) -> bool {
        self.next.is_some() && self.hits.len() < FILES_LOADED_MAX
    }

    /// Starts over from the newest page (on open, Retry, or a new visit).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.cached_thumbs.clear();
        self.load = FilesLoad::Loading;
        self.loading_more = false;
        self.more_failed = false;
        self.request(None, cx);
        cx.notify();
    }

    /// Asks for the next older page, once at a time, while under the cap and
    /// unless the last one failed (see [`Self::retry_more`]).
    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.load != FilesLoad::Ready
            || self.loading_more
            || self.more_failed
            || !self.has_more()
        {
            return;
        }
        self.loading_more = true;
        let cursor = self.next.clone();
        self.request(cursor, cx);
        // The footer shows the loading row (a retained view repaints only when notified).
        cx.notify();
    }

    /// The user asked again for the next page after it failed.
    pub fn retry_more(&mut self, cx: &mut Context<Self>) {
        self.more_failed = false;
        self.load_more(cx);
        cx.notify();
    }

    /// Scrolls to the end of the list (a test hook).
    pub fn scroll_to_end(&mut self) {
        let n = self.row_count();
        self.scroll
            .scroll_to_item(n.saturating_sub(1), gpui::ScrollStrategy::Bottom);
    }

    /// What the row after the files shows, if anything.
    fn footer(&self) -> Option<Footer> {
        if self.loading_more {
            Some(Footer::Loading)
        } else if self.more_failed {
            Some(Footer::Failed)
        } else if self.next.is_some() && self.hits.len() >= FILES_LOADED_MAX {
            Some(Footer::Cap)
        } else {
            None
        }
    }

    fn row_count(&self) -> usize {
        self.hits.len() + usize::from(self.footer().is_some())
    }

    fn request(&mut self, cursor: Option<Arc<str>>, cx: &mut Context<Self>) {
        let generation = self.generation;
        let more = cursor.is_some();
        let this = cx.weak_entity();
        // The host may answer while it is itself being updated; the tab reads
        // it when the page lands, so take the answer on the next cycle.
        let done: FilesDone = Box::new(move |r, cx| {
            cx.defer(move |cx| {
                this.update(cx, |t, cx| t.answered(generation, more, r, cx))
                    .ok();
            });
        });
        let c = self.conversation.clone();
        let limit = if more {
            FILES_PAGE.min(FILES_LOADED_MAX.saturating_sub(self.hits.len()))
        } else {
            FILES_PAGE
        };
        self.session
            .update(cx, |s, cx| s.load_files(c, cursor, limit, done, cx));
    }

    fn answered(
        &mut self,
        generation: u64,
        more: bool,
        r: Result<Page<FileHit>, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        match (r, more) {
            (Ok(page), false) => {
                self.hits = page.items;
                self.hits.truncate(FILES_LOADED_MAX);
                self.next = page.next;
                self.load = FilesLoad::Ready;
                self.cursor = self.cursor.min(self.hits.len().saturating_sub(1));
                self.metas.clear();
            }
            (Ok(page), true) => {
                self.loading_more = false;
                let room = FILES_LOADED_MAX.saturating_sub(self.hits.len());
                self.hits.extend(page.items.into_iter().take(room));
                self.next = page.next;
            }
            (Err(_), false) => self.load = FilesLoad::Failed,
            (Err(_), true) => {
                self.loading_more = false;
                self.more_failed = true;
                if self.hits.is_empty() {
                    // Nothing listed to keep: show the error and its Retry.
                    self.load = FilesLoad::Failed;
                }
                cx.notify();
                return;
            }
        }
        self.build_metas(cx);
        if self.load == FilesLoad::Ready && self.hits.is_empty() && self.has_more() {
            // A page of deleted files only: look further back.
            self.load_more(cx);
        }
        cx.notify();
    }

    /// Builds the line under each row that has none yet (the new page's).
    fn build_metas(&mut self, cx: &App) {
        let l = i18n::Translate::localizer(cx);
        let tz = crate::time_zone(cx);
        let state = self.session.read(cx).files_state();
        for h in &self.hits[self.metas.len()..] {
            let sharer = crate::message_row::author_name(
                h.author.as_ref().and_then(|u| state.members.get(u)),
                l,
            );
            let date =
                h.ts.as_ref()
                    .and_then(|t| t.0.split('.').next()?.parse::<i64>().ok())
                    .and_then(|s| jiff::Timestamp::from_second(s).ok())
                    .map(|t| l.date(t, &tz))
                    .unwrap_or_default();
            self.metas.push(
                t!(
                    l,
                    "files-row-meta",
                    size = crate::files::format_size(h.file.size, l),
                    sharer = sharer.to_string(),
                    date = date
                )
                .into_owned()
                .into(),
            );
        }
    }

    /// Opens row `i`: the viewer for images and PDFs, the code viewer for code,
    /// else a download.
    pub fn open(&mut self, i: usize, cx: &mut Context<Self>) {
        let app: &App = cx;
        let l = i18n::Translate::localizer(app);
        let state = self.session.read(app).files_state();
        let open = open_for(&self.hits, i, |h| {
            crate::message_row::author_name(h.author.as_ref().and_then(|u| state.members.get(u)), l)
        });
        self.cursor = i.min(self.hits.len().saturating_sub(1));
        match open {
            Some(FileOpen::View(g)) => self.session.update(cx, |s, cx| s.view_file(g, cx)),
            Some(FileOpen::Code(g)) => self.session.update(cx, |s, cx| s.view_code(g, cx)),
            Some(FileOpen::Download(f)) => self.session.update(cx, |s, cx| s.download_file(&f, cx)),
            None => {}
        }
        cx.notify();
    }

    /// The line under row `i`'s name: size, sharer and date.
    pub fn row_meta(&self, i: usize) -> Option<String> {
        self.metas.get(i).map(|m| m.to_string())
    }

    /// The thumbnail row `f` shows: the preview the timeline shows (so a
    /// cached one is reused), when the setting allows it.
    fn thumbnail(&mut self, f: &FileRef, inline: InlineImages, cx: &App) -> Option<ImageRef> {
        if f.kind != FileKind::Image || f.tombstone {
            return None;
        }
        let src = ImageRef::Provider(f.best_thumb(crate::files::PREVIEW_W)?.url.clone());
        let show = match inline {
            InlineImages::Always => true,
            InlineImages::Never => false,
            InlineImages::OnClick => *self.cached_thumbs.entry(f.id.clone()).or_insert_with(|| {
                cx.try_global::<crate::media::ImageCache>()
                    .is_some_and(|c| c.on_disk(&src))
            }),
        };
        show.then_some(src)
    }

    fn on_key(&mut self, ev: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        if k.modifiers.platform || k.modifiers.control || k.modifiers.alt || self.hits.is_empty() {
            return;
        }
        let last = self.hits.len() - 1;
        let at = self.cursor.min(last);
        let next = match k.key.as_str() {
            "up" => at.saturating_sub(1),
            "down" => (at + 1).min(last),
            "home" => 0,
            "end" => last,
            "enter" | "space" => {
                cx.stop_propagation();
                if !k.modifiers.shift && !ev.is_held {
                    self.open(at, cx);
                }
                return;
            }
            _ => return,
        };
        cx.stop_propagation();
        self.cursor = next;
        self.scroll
            .scroll_to_item(next, gpui::ScrollStrategy::Nearest);
        if next + PREFETCH_ROWS >= self.hits.len() {
            self.load_more(cx);
        }
        cx.notify();
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        if range.end + PREFETCH_ROWS >= self.hits.len() {
            // A no-op while a page is in flight or after one failed.
            self.load_more(cx);
        }
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let ring_at = self.focus.is_focused(window).then_some(self.cursor);
        let inline = self.session.read(cx).files_inline_images(cx);
        let tile = LAYOUT_MSG_AVATAR;
        let mut out = Vec::with_capacity(range.len());
        for i in range {
            let Some(f) = self.hits.get(i).map(|h| h.file.clone()) else {
                if let Some(footer) = self.footer() {
                    out.push(self.render_footer(footer, &colors, cx));
                }
                break;
            };
            let f = &f;
            let gone = f.tombstone;
            // A deleted file's row is dimmed. Dim its colours, not the row:
            // GPUI Fast multiplies element opacity into every paint, so the
            // thumbnail tile's border (a one-pixel edge over its fill) drew as
            // a darker outline over the faded fill. `faded` composites each
            // colour over the list first, as main's group opacity drew them.
            let ground = colors.editor_background;
            let fade = |colour: gpui::Hsla| {
                if gone {
                    themes::color::faded(colour, ground, 0.5)
                } else {
                    colour
                }
            };
            let name = display_name(f);
            let meta = self.metas.get(i).cloned().unwrap_or_default();
            let thumb = self.thumbnail(f, inline, cx);
            let lead = match thumb {
                Some(src) => {
                    let key = format!("files-thumb-{i}");
                    div()
                        .debug_selector(move || key)
                        .when(gone, |d| d.opacity(0.5))
                        .child(
                            MediaImage::new(src, name.clone())
                                .max_size(tile.as_f32(), tile.as_f32())
                                .fixed(),
                        )
                        .into_any_element()
                }
                None => Icon::new(kind_icon(f.kind))
                    .size(ICON_SIZE_LG)
                    .color(fade(colors.text_muted))
                    .into_any_element(),
            };
            let id = format!("files-row-{i}");
            let sel = id.clone();
            let hover = colors.element_hover;
            let ring_colors = themes::ThemeColors {
                border_focused: fade(colors.border_focused),
                ..colors
            };
            out.push(
                div()
                    .id(SharedString::from(id))
                    .debug_selector(move || sel.clone())
                    .flex()
                    .items_center()
                    .gap(SPACE_5)
                    .w_full()
                    .px(SPACE_7)
                    .py(SPACE_4)
                    .when(!gone, |d| d.cursor_pointer().hover(move |d| d.bg(hover)))
                    .when(ring_at == Some(i), |d| {
                        d.bg(fade(colors.element_selected))
                            .shadow(crate::controls::ring(&ring_colors))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // The list takes the focus so the clicked row shows
                        // as the selected one and the arrows go on from it.
                        window.focus(&this.focus, cx);
                        this.open(i, cx)
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(tile)
                            .rounded(RADIUS_MD)
                            .overflow_hidden()
                            .bg(fade(tokens.bg_sunken))
                            .border_1()
                            .border_color(fade(tokens.border_subtle))
                            .child(lead),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(SPACE_1)
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(FONT_SIZE_MD)
                                    .font_weight(FONT_WEIGHT_SEMIBOLD)
                                    .text_color(fade(colors.text))
                                    .child(name),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(FONT_SIZE_XS)
                                    .text_color(fade(tokens.text_subtle))
                                    .child(meta),
                            ),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    /// The row after the files: the next page loading, its failure with
    /// Retry, or the hint that the list stops at [`FILES_LOADED_MAX`].
    fn render_footer(
        &self,
        footer: Footer,
        colors: &themes::ThemeColors,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let row = div()
            .flex()
            .items_center()
            .justify_center()
            .gap(SPACE_4)
            .w_full()
            .h(LAYOUT_MSG_AVATAR + SPACE_4 + SPACE_4)
            .px(SPACE_7)
            .text_size(FONT_SIZE_XS)
            .text_color(colors.text_muted);
        match footer {
            Footer::Loading => row
                .debug_selector(|| "files-more-loading".into())
                .child(
                    Icon::new(IconName::Refresh)
                        .size(ICON_SIZE_LG)
                        .color(colors.text_muted),
                )
                .child(t!(cx, "files-loading-more").into_owned())
                .into_any_element(),
            Footer::Failed => row
                .debug_selector(|| "files-more-failed".into())
                .child(t!(cx, "files-load-more-error").into_owned())
                .child(crate::controls::on_activate(
                    crate::controls::compact(crate::controls::button(
                        "files-more-retry",
                        t!(cx, "load-retry").into_owned(),
                        crate::controls::ButtonKind::Secondary,
                        true,
                        colors,
                    ))
                    .debug_selector(|| "files-more-retry".into()),
                    cx,
                    |this: &mut Self, _, cx| this.retry_more(cx),
                ))
                .into_any_element(),
            Footer::Cap => row
                .debug_selector(|| "files-end".into())
                .child(t!(cx, "files-end-cap", count = FILES_LOADED_MAX).into_owned())
                .into_any_element(),
        }
    }
}

/// What the row after the files shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Footer {
    Loading,
    Failed,
    Cap,
}

fn display_name(f: &FileRef) -> String {
    f.title
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or(&f.name)
        .to_string()
}

impl<S: FilesSource> gpui::Focusable for FilesTab<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: FilesSource> Render for FilesTab<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let root = div()
            .id("files-tab")
            .debug_selector(|| "files-tab".into())
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.editor_background);
        match self.load {
            FilesLoad::Loading => root.child(crate::load_state::skeleton(
                "files-loading",
                3,
                t!(cx, "files-loading").into_owned(),
                cx,
            )),
            FilesLoad::Failed => root.child(crate::load_state::load_error(
                "files-error",
                "files-retry",
                t!(cx, "files-load-error-title").into_owned(),
                t!(cx, "files-load-error-hint").into_owned(),
                t!(cx, "load-retry").into_owned(),
                cx,
                |this: &mut Self, _, cx| this.reload(cx),
            )),
            FilesLoad::Ready if self.hits.is_empty() => root.child(
                div()
                    .debug_selector(|| "files-empty".into())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(SPACE_2)
                    .child(
                        Icon::new(IconName::Layers)
                            .size(ICON_SIZE_LG)
                            .color(colors.text_muted),
                    )
                    .child(
                        crate::controls::empty_state(
                            t!(cx, "files-empty-title").into_owned(),
                            t!(cx, "files-empty-hint").into_owned(),
                            &colors,
                        )
                        .py_0(),
                    ),
            ),
            FilesLoad::Ready => root.child(
                div()
                    .id("files-list")
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::on_key))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .py(SPACE_2)
                    .child(
                        uniform_list(
                            "files-rows",
                            self.row_count(),
                            cx.processor(Self::render_rows),
                        )
                        .track_scroll(&self.scroll)
                        .w_full()
                        .flex_1()
                        .min_h_0(),
                    ),
            ),
        }
    }
}
