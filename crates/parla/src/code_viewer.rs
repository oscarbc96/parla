//! The code viewer window: a snippet or a code file from a conversation with line numbers,
//! syntax colours, its language, Wrap, Line numbers, Copy, Download, Jump to
//! message and find (Cmd+F; Enter in the find field or Cmd+G for the next
//! match, Shift with either for the previous). One window at a time, reused for
//! the next file. Esc closes find, then the window; switching workspace,
//! signing out and the idle lock close it too.
//!
//! The text comes from the provider's preview when that is the whole file, else
//! the whole file is fetched once through the fetcher of the workspace that
//! owns it (at most [`MAX_CODE_BYTES`]), decoded in place and kept only while
//! the window is open; nothing is written to disk. Opening another file or
//! closing the window drops the fetch, which aborts its download. Only the
//! lines on screen are laid out; each drawn line is built once and kept in a
//! cache of [`LINE_CACHE_MAX`] lines, and labels are built when the state
//! changes, never in `render`. Find scans off the UI thread and drops stale
//! results.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use components::code_viewer::{CodeDoc, Find, MAX_CODE_BYTES, merge_styles, whole_preview};
use components::controls::{on_activate_app, ring};
use components::media::{MediaError, MediaFetcher};
use components::media_viewer::{Gallery, ViewerItem};
use components::snippet_highlight::Class;
use components::text_input::{TextInput, TextInputEvent};
use components::{Icon, IconName};
use domain::{FileRef, ImageRef, MessageId};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Div, Entity, FocusHandle, Focusable as _,
    Global, HighlightStyle, Hsla, IntoElement, KeyDownEvent, ListAlignment,
    ListHorizontalSizingBehavior, ListState, MouseButton, Render, ScrollStrategy, SharedString,
    Stateful, StyledText, Subscription, Task, UniformListScrollHandle, Window, WindowHandle, div,
    list, prelude::*, size, uniform_list,
};
use themes::metrics as m;
use themes::{ActiveTheme as _, ParlaTokens, ThemeColors};

/// What the viewer asks of the workspace it was opened from.
pub trait CodeHost {
    /// Saves the file into the downloads folder.
    fn download(&self, file: &FileRef, cx: &mut App);
    /// Shows `message` in the main window.
    fn open_in_channel(&self, _message: &MessageId, _cx: &mut App) {}
    /// Fetches through the workspace that owns the file (its provider and
    /// credentials). `None`: the file cannot be fetched.
    fn fetcher(&self, _cx: &App) -> Option<Arc<dyn MediaFetcher>> {
        None
    }
}

impl CodeHost for gpui::WeakEntity<crate::session::WorkspaceSession> {
    fn download(&self, file: &FileRef, cx: &mut App) {
        let _ = self.update(cx, |s, cx| s.download(file, cx));
    }

    fn open_in_channel(&self, message: &MessageId, cx: &mut App) {
        let id = message.clone();
        if self.update(cx, |s, cx| s.jump_to(id, cx)).is_ok() {
            cx.activate(true);
        }
    }

    fn fetcher(&self, cx: &App) -> Option<Arc<dyn MediaFetcher>> {
        let provider = self.upgrade()?.read(cx).provider.clone();
        Some(Arc::new(crate::media::Fetcher::new(
            provider,
            crate::runtime::handle(cx),
        )))
    }
}

/// Where the text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Load {
    /// The whole file is being fetched; the preview shows meanwhile.
    Loading,
    Ready,
    /// The fetch failed; the preview, if any, shows.
    Failed,
    /// Over [`MAX_CODE_BYTES`]: download it instead.
    TooLarge,
    /// The bytes are not text.
    NotText,
}

/// Lines laid out past the window's edge in the wrapped view.
const WRAP_OVERDRAW_LINES: f32 = 20.0;

/// Drawn lines kept built; the least recently drawn goes first.
pub const LINE_CACHE_MAX: usize = 256;

/// One drawn line, built once: its text, styles and number.
struct Line {
    text: SharedString,
    styles: Vec<(Range<usize>, HighlightStyle)>,
    number: SharedString,
}

/// The colours a built line depends on; a theme or accent change rebuilds.
type Palette = [Hsla; 9];

fn palette(colors: &ThemeColors, t: &ParlaTokens) -> Palette {
    [
        colors.text,
        t.code_keyword,
        t.code_string,
        t.code_function,
        t.code_comment,
        t.code_number,
        t.code_type,
        t.signal_soft,
        t.signal,
    ]
}

/// Drawn lines by index, bounded to [`LINE_CACHE_MAX`].
#[derive(Default)]
struct LineCache {
    lines: HashMap<usize, (u64, Rc<Line>)>,
    tick: u64,
    palette: Option<Palette>,
    builds: usize,
}

impl LineCache {
    fn clear(&mut self) {
        self.lines.clear();
    }

    fn get(&mut self, ix: usize) -> Option<Rc<Line>> {
        self.tick += 1;
        let tick = self.tick;
        self.lines.get_mut(&ix).map(|(t, l)| {
            *t = tick;
            l.clone()
        })
    }

    fn insert(&mut self, ix: usize, line: Rc<Line>) {
        if self.lines.len() >= LINE_CACHE_MAX
            && let Some(old) = self
                .lines
                .iter()
                .min_by_key(|(_, (t, _))| *t)
                .map(|(k, _)| *k)
        {
            self.lines.remove(&old);
        }
        self.tick += 1;
        self.builds += 1;
        self.lines.insert(ix, (self.tick, line));
    }
}

/// Every string the window shows, built when the state changes.
#[derive(Default)]
struct Labels {
    name: SharedString,
    meta: Option<SharedString>,
    jump: SharedString,
    wrap: SharedString,
    numbers: SharedString,
    copy: SharedString,
    download: SharedString,
    language: Option<SharedString>,
    find_count: SharedString,
    find_count_label: SharedString,
    find_prev: SharedString,
    find_next: SharedString,
    find_close: SharedString,
    find_region: SharedString,
    empty: SharedString,
    note: Option<(&'static str, SharedString)>,
    size: Option<SharedString>,
    lines: SharedString,
    code_region: SharedString,
}

/// The focusable controls, so Tab reaches them and a test can focus one.
struct Controls {
    jump: FocusHandle,
    wrap: FocusHandle,
    numbers: FocusHandle,
    copy: FocusHandle,
    download: FocusHandle,
}

pub struct CodeViewer {
    gallery: Gallery,
    load: Load,
    doc: Option<CodeDoc>,
    find: Find,
    find_open: bool,
    /// Bumped by each search; a result for an older one is dropped.
    find_gen: u64,
    find_task: Option<Task<()>>,
    input: Entity<TextInput>,
    wrap: bool,
    numbers: bool,
    copied: bool,
    scroll: UniformListScrollHandle,
    list: ListState,
    focus: FocusHandle,
    controls: Controls,
    host: Rc<dyn CodeHost>,
    fetcher: Option<Arc<dyn MediaFetcher>>,
    code_font: SharedString,
    task: Option<Task<()>>,
    cache: LineCache,
    labels: Labels,
    _subs: Vec<Subscription>,
}

impl CodeViewer {
    /// A viewer on the current file of `gallery`; fetches the whole file
    /// through `fetcher` when the preview is only part of it.
    pub fn new(
        gallery: Gallery,
        fetcher: Option<Arc<dyn MediaFetcher>>,
        host: Rc<dyn CodeHost>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut i = TextInput::new(cx);
            i.set_single_line(true);
            i
        });
        let on_input = cx.subscribe(&input, |v, input, _: &TextInputEvent, cx| {
            let q = input.read(cx).text().to_string();
            v.search(q, cx);
        });
        let on_settings = cx.observe_global::<settings::SettingsStore>(|v, cx| {
            let font = components::live_settings::code_font(cx);
            if font != v.code_font {
                v.code_font = font;
                // Line heights and widths follow the font.
                v.list.remeasure();
                cx.notify();
            }
        });
        let on_locale = cx.observe_global::<i18n::gpui::I18n>(|v, cx| {
            v.relabel(cx);
            cx.notify();
        });
        let control = |cx: &mut Context<Self>| cx.focus_handle().tab_stop(true);
        let controls = Controls {
            jump: control(cx),
            wrap: control(cx),
            numbers: control(cx),
            copy: control(cx),
            download: control(cx),
        };
        let mut v = Self {
            gallery,
            load: Load::Loading,
            doc: None,
            find: Find::default(),
            find_open: false,
            find_gen: 0,
            find_task: None,
            input,
            wrap: false,
            numbers: true,
            copied: false,
            scroll: UniformListScrollHandle::new(),
            list: ListState::new(
                0,
                ListAlignment::Top,
                m::LAYOUT_CODE_VIEWER_LINE * WRAP_OVERDRAW_LINES,
            ),
            focus: cx.focus_handle(),
            controls,
            host,
            fetcher,
            code_font: components::live_settings::code_font(cx),
            task: None,
            cache: LineCache::default(),
            labels: Labels::default(),
            _subs: vec![on_input, on_settings, on_locale],
        };
        v.load(cx);
        v
    }

    /// Shows another file (one was opened while this window is open), loaded
    /// through `fetcher` and acting through `host`. The fetch in flight, if
    /// any, is dropped.
    pub fn set_gallery(
        &mut self,
        gallery: Gallery,
        fetcher: Option<Arc<dyn MediaFetcher>>,
        host: Rc<dyn CodeHost>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.gallery = gallery;
        self.fetcher = fetcher;
        self.host = host;
        self.close_find(window, cx);
        self.load(cx);
    }

    fn item(&self) -> Option<&ViewerItem> {
        self.gallery.current()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        // Dropping the task drops the fetch, which aborts its download.
        self.task = None;
        self.copied = false;
        let Some(f) = self.item().map(|it| it.file.clone()) else {
            self.set_doc(None, Load::Failed, cx);
            return;
        };
        let name = f.name.clone();
        if whole_preview(&f).is_some()
            && let Some(p) = f.preview.clone()
        {
            // Shares the preview: no copy.
            self.set_doc(Some(CodeDoc::new(p, &name)), Load::Ready, cx);
            return;
        }
        let preview = f.preview.clone().map(|p| CodeDoc::new(p, &name));
        let (Some(url), Some(fetcher)) = (f.download.clone(), self.fetcher.clone()) else {
            self.set_doc(preview, Load::Failed, cx);
            return;
        };
        if f.size > MAX_CODE_BYTES {
            self.set_doc(preview, Load::TooLarge, cx);
            return;
        }
        self.set_doc(preview, Load::Loading, cx);
        self.task = Some(cx.spawn(async move |this, cx| {
            let fetched = fetcher
                .fetch(&ImageRef::Provider(url), MAX_CODE_BYTES)
                .await;
            let parsed = match fetched {
                Ok(bytes) => Ok(cx
                    .background_executor()
                    .spawn(async move { CodeDoc::from_bytes(bytes, &name) })
                    .await),
                Err(e) => Err(e),
            };
            let _ = this.update(cx, |v, cx| match parsed {
                Ok(Some(doc)) => v.set_doc(Some(doc), Load::Ready, cx),
                Ok(None) => v.set_load(Load::NotText, cx),
                Err(MediaError::TooLarge) => v.set_load(Load::TooLarge, cx),
                Err(_) => v.set_load(Load::Failed, cx),
            });
        }));
    }

    fn set_load(&mut self, load: Load, cx: &mut Context<Self>) {
        self.load = load;
        self.relabel(cx);
        cx.notify();
    }

    fn set_doc(&mut self, doc: Option<CodeDoc>, load: Load, cx: &mut Context<Self>) {
        self.list.reset(doc.as_ref().map_or(0, |d| d.line_count()));
        self.cache.clear();
        self.doc = doc;
        self.load = load;
        if self.find_open {
            let q = self.find.query().to_string();
            self.search(q, cx);
        }
        self.relabel(cx);
        cx.notify();
    }

    /// The text shown, when there is any.
    pub fn doc(&self) -> Option<&CodeDoc> {
        self.doc.as_ref()
    }

    pub fn load_state(&self) -> Load {
        self.load
    }

    /// Lines of the text shown (0 with none).
    pub fn line_count(&self) -> usize {
        self.doc.as_ref().map_or(0, |d| d.line_count())
    }

    pub fn wrap(&self) -> bool {
        self.wrap
    }

    pub fn find(&self) -> &Find {
        &self.find
    }

    /// Lines built so far (each is built once until the cache drops it).
    pub fn line_builds(&self) -> usize {
        self.cache.builds
    }

    /// Lines held built.
    pub fn cached_lines(&self) -> usize {
        self.cache.lines.len()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// Focuses the control with debug selector `id` (tests).
    #[doc(hidden)]
    pub fn focus_control_for_tests(&self, id: &str, window: &mut Window, cx: &mut App) {
        let c = &self.controls;
        let h = match id {
            "code-viewer-jump" => &c.jump,
            "code-viewer-wrap" => &c.wrap,
            "code-viewer-numbers" => &c.numbers,
            "code-viewer-copy" => &c.copy,
            "code-viewer-download" => &c.download,
            _ => panic!("no control {id}"),
        };
        window.focus(h, cx);
    }

    /// Scrolls line `ix` to the top (tests).
    #[doc(hidden)]
    pub fn scroll_to_line_for_tests(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.reveal(ix, ScrollStrategy::Top);
        cx.notify();
    }

    fn top_line(&self) -> usize {
        if self.wrap {
            self.list.logical_scroll_top().item_ix
        } else {
            // `logical_scroll_top_index` is test-support only in gpui; read
            // the same state so release builds compile.
            let state = self.scroll.0.borrow();
            state
                .deferred_scroll_to_item
                .as_ref()
                .map(|d| d.item_index)
                .unwrap_or_else(|| state.base_handle.logical_scroll_top().0)
        }
    }

    pub fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        let top = self.top_line();
        self.wrap = !self.wrap;
        self.list.reset(self.line_count());
        self.reveal(top, ScrollStrategy::Top);
        cx.notify();
    }

    pub fn toggle_numbers(&mut self, cx: &mut Context<Self>) {
        self.numbers = !self.numbers;
        self.list.remeasure();
        cx.notify();
    }

    /// Puts the whole text on the clipboard.
    pub fn copy_all(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = &self.doc else { return };
        let text = doc.shared_text();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_string()));
        self.copied = true;
        self.relabel(cx);
        cx.notify();
    }

    fn download(&mut self, cx: &mut Context<Self>) {
        if let Some(f) = self.item().map(|it| it.file.clone()) {
            self.host.download(&f, cx);
        }
    }

    fn jump(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.item().map(|it| it.message.clone()) {
            self.host.open_in_channel(&m, cx);
        }
    }

    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = true;
        let handle = self.input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        self.input
            .update(cx, |i, cx| i.select(0..i.text().len(), cx));
        self.relabel(cx);
        cx.notify();
    }

    /// Opens find with `query` typed in (tests and visual baselines).
    pub fn find_text(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(window, cx);
        self.input.update(cx, |i, cx| i.set_text(query, cx));
        self.search(query.to_string(), cx);
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.find_open {
            return;
        }
        self.find_open = false;
        self.find_gen += 1;
        self.find_task = None;
        self.find.clear();
        self.cache.clear();
        self.input.update(cx, |i, cx| i.set_text("", cx));
        window.focus(&self.focus, cx);
        self.relabel(cx);
        cx.notify();
    }

    /// Scans for `q` off the UI thread; the result of an older scan is dropped.
    fn search(&mut self, q: String, cx: &mut Context<Self>) {
        self.find_gen += 1;
        let generation = self.find_gen;
        let Some(doc) = self.doc.clone() else {
            self.find.clear();
            self.find_task = None;
            self.cache.clear();
            self.relabel(cx);
            cx.notify();
            return;
        };
        let top = self.top_line();
        let scan = cx.background_executor().spawn(async move {
            let mut f = Find::default();
            f.search_from(&doc, &q, top);
            f
        });
        self.find_task = Some(cx.spawn(async move |this, cx| {
            let found = scan.await;
            let _ = this.update(cx, |v, cx| {
                if v.find_gen != generation {
                    return;
                }
                v.find = found;
                v.cache.clear();
                v.reveal_current();
                v.relabel(cx);
                cx.notify();
            });
        }));
    }

    pub fn find_next(&mut self, cx: &mut Context<Self>) {
        self.find.step_next();
        self.find_moved(cx);
    }

    pub fn find_prev(&mut self, cx: &mut Context<Self>) {
        self.find.step_prev();
        self.find_moved(cx);
    }

    fn find_moved(&mut self, cx: &mut Context<Self>) {
        // The current match's style moves; rebuild the lines on screen.
        self.cache.clear();
        self.reveal_current();
        self.relabel(cx);
        cx.notify();
    }

    fn reveal_current(&mut self) {
        if let Some(line) = self.find.current().map(|m| m.line) {
            self.reveal(line, ScrollStrategy::Center);
        }
    }

    fn reveal(&mut self, line: usize, how: ScrollStrategy) {
        if self.wrap {
            self.list.scroll_to_reveal_item(line);
        } else {
            self.scroll.scroll_to_item(line, how);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        let in_field = self.input.read(cx).focus_handle(cx).is_focused(window);
        match k.key.as_str() {
            "f" if cmd => self.open_find(window, cx),
            "g" if cmd && k.modifiers.shift => self.find_prev(cx),
            "g" if cmd => self.find_next(cx),
            "w" if cmd => window.remove_window(),
            // Enter steps only from the find field; on a button it presses it.
            "enter" if in_field && !cmd && k.modifiers.shift => self.find_prev(cx),
            "enter" if in_field && !cmd => self.find_next(cx),
            "escape" if self.find_open => self.close_find(window, cx),
            "escape" => window.remove_window(),
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

    /// Builds every label from the state (on a state or locale change).
    fn relabel(&mut self, cx: &mut Context<Self>) {
        let l = i18n::gpui::localizer(cx);
        let s = |c: std::borrow::Cow<'_, str>| -> SharedString { c.into_owned().into() };
        let item = self.item().cloned();
        let count = self.line_count() as i64;
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
        let meta = item.as_ref().map(|it| {
            let time = self.when(it, &l, cx);
            let text = match &self.gallery.place {
                Some(p) => t!(
                    l,
                    "code-viewer-meta",
                    sender = it.sender.to_string(),
                    place = p.to_string(),
                    time = time
                ),
                None => t!(
                    l,
                    "code-viewer-meta-direct",
                    sender = it.sender.to_string(),
                    time = time
                ),
            };
            s(text)
        });
        let (find_count, find_count_label) = if self.find.query().is_empty() {
            (SharedString::default(), SharedString::default())
        } else if self.find.is_empty() {
            let none = s(t!(l, "code-viewer-find-none"));
            (none.clone(), none)
        } else {
            let index = self.find.current_index().map_or(0, |i| i + 1) as i64;
            let total = self.find.len() as i64;
            let (shown, spoken) = if self.find.capped() {
                (
                    t!(
                        l,
                        "code-viewer-find-count-capped",
                        index = index,
                        count = total
                    ),
                    t!(
                        l,
                        "code-viewer-find-count-label-capped",
                        index = index,
                        count = total
                    ),
                )
            } else {
                (
                    t!(l, "code-viewer-find-count", index = index, count = total),
                    t!(
                        l,
                        "code-viewer-find-count-label",
                        index = index,
                        count = total
                    ),
                )
            };
            (s(shown), s(spoken))
        };
        let has_doc = self.doc.is_some();
        let note = match (self.load, has_doc) {
            (Load::Loading, true) => Some(("code-viewer-loading", s(t!(l, "code-viewer-loading")))),
            (Load::Failed, true) => Some(("code-viewer-failed", s(t!(l, "code-viewer-failed")))),
            (Load::TooLarge, true) => {
                Some(("code-viewer-too-large", s(t!(l, "code-viewer-too-large"))))
            }
            (Load::NotText, true) => {
                Some(("code-viewer-not-text", s(t!(l, "code-viewer-not-text"))))
            }
            _ => None,
        };
        let empty = match self.load {
            Load::Loading => t!(l, "code-viewer-loading"),
            Load::TooLarge => t!(l, "code-viewer-too-large"),
            Load::NotText => t!(l, "code-viewer-not-text"),
            Load::Ready | Load::Failed => t!(l, "code-viewer-failed-empty"),
        };
        let placeholder = t!(l, "code-viewer-find-placeholder").into_owned();
        self.input.update(cx, |i, _| i.set_placeholder(placeholder));
        self.labels = Labels {
            name,
            meta,
            jump: s(t!(l, "code-viewer-jump")),
            wrap: s(t!(l, "code-viewer-wrap")),
            numbers: s(t!(l, "code-viewer-line-numbers")),
            copy: s(if self.copied {
                t!(l, "code-viewer-copied")
            } else {
                t!(l, "code-viewer-copy")
            }),
            download: s(t!(l, "code-viewer-download")),
            language: self
                .doc
                .as_ref()
                .and_then(|d| d.language())
                .map(|lang| lang.display(&l).into()),
            find_count,
            find_count_label,
            find_prev: s(t!(l, "code-viewer-find-prev")),
            find_next: s(t!(l, "code-viewer-find-next")),
            find_close: s(t!(l, "code-viewer-find-close")),
            find_region: s(t!(l, "code-viewer-find-placeholder")),
            empty: s(empty),
            note,
            size: item
                .as_ref()
                .map(|it| components::files::format_size(it.file.size, &l).into()),
            lines: s(t!(l, "code-viewer-lines", count = count)),
            code_region: s(t!(l, "code-viewer-region", count = count)),
        };
    }

    /// Line `ix`, from the cache or built once: its number, and its text with
    /// syntax colours and find marks (a match is drawn in `text` on the soft
    /// signal ground, which every built-in theme keeps at 4.5:1).
    fn line(&mut self, ix: usize, p: &Palette) -> Option<Rc<Line>> {
        if self.cache.palette.as_ref() != Some(p) {
            self.cache.clear();
            self.cache.palette = Some(*p);
        }
        if let Some(l) = self.cache.get(ix) {
            return Some(l);
        }
        let doc = self.doc.as_ref()?;
        let drawn = doc.drawn_line(ix);
        let color = |c: Class| match c {
            Class::Keyword => p[1],
            Class::String => p[2],
            Class::Function => p[3],
            Class::Comment => p[4],
            Class::Number => p[5],
            Class::Type => p[6],
        };
        let syntax: Vec<(Range<usize>, HighlightStyle)> = doc
            .spans(ix)
            .into_iter()
            .map(|(r, c)| {
                (
                    r,
                    HighlightStyle {
                        color: Some(color(c)),
                        font_style: (c == Class::Comment).then_some(gpui::FontStyle::Italic),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let current = self.find.current();
        let marks: Vec<(Range<usize>, HighlightStyle)> = self
            .find
            .on_line(ix)
            .iter()
            .filter(|mk| mk.range.start < drawn.len())
            .map(|mk| {
                let is_current = current == Some(mk);
                (
                    mk.range.start..mk.range.end.min(drawn.len()),
                    HighlightStyle {
                        color: Some(p[0]),
                        background_color: Some(p[7]),
                        underline: is_current.then_some(gpui::UnderlineStyle {
                            thickness: m::SPACE_1,
                            color: Some(p[8]),
                            wavy: false,
                        }),
                        font_weight: is_current.then_some(m::FONT_WEIGHT_SEMIBOLD),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let shown = if drawn.is_empty() { " " } else { drawn };
        let line = Rc::new(Line {
            text: shown.to_string().into(),
            styles: merge_styles(&syntax, &marks),
            number: (ix + 1).to_string().into(),
        });
        self.cache.insert(ix, line.clone());
        Some(line)
    }

    fn row(&mut self, ix: usize, colors: &ThemeColors, tokens: &ParlaTokens) -> Div {
        let p = palette(colors, tokens);
        let Some(line) = self.line(ix, &p) else {
            return div();
        };
        let n = ix + 1;
        let on_current = self.find.current().is_some_and(|c| c.line == ix);
        let text = StyledText::new(line.text.clone()).with_highlights(line.styles.iter().cloned());
        div()
            .debug_selector(move || format!("code-line-{n}"))
            .flex()
            .w_full()
            .when(!self.wrap, |d| d.h(m::LAYOUT_CODE_VIEWER_LINE))
            .line_height(m::LAYOUT_CODE_VIEWER_LINE)
            .when(on_current, |d| d.bg(colors.element_hover))
            .when(self.numbers, |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("code-number-{n}"))
                        .flex_none()
                        .w(m::LAYOUT_CODE_VIEWER_GUTTER)
                        .pr(m::SPACE_5)
                        .text_right()
                        .text_color(tokens.text_subtle)
                        .child(line.number.clone()),
                )
            })
            .when(!self.numbers, |d| d.pl(m::SPACE_6))
            .child(
                div()
                    .pr(m::SPACE_6)
                    .when(self.wrap, |d| d.flex_1().min_w_0())
                    .when(!self.wrap, |d| d.flex_none().whitespace_nowrap())
                    .child(text),
            )
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        range.map(|ix| self.row(ix, &colors, &tokens)).collect()
    }

    fn render_wrapped(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        self.row(ix, &colors, &tokens).into_any_element()
    }
}

/// What a toolbar button shows and does.
struct Tool<'a> {
    id: &'static str,
    focus: &'a FocusHandle,
    icon: Option<IconName>,
    label: SharedString,
    /// `Some` for a toggle: whether it is on.
    on: Option<bool>,
}

/// A toolbar button (board `.tbb`): label, optional icon, hover ground, focus
/// ring; a toggle that is on shows pressed.
fn tool_button(
    t: Tool<'_>,
    colors: &ThemeColors,
    tokens: &ParlaTokens,
    f: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let pressed = t.on == Some(true);
    let (fg, hover) = (colors.text_muted, colors.element_hover);
    let r = ring(colors);
    let id = t.id;
    let mut el = div()
        .id(id)
        .debug_selector(move || id.into())
        .track_focus(t.focus)
        .flex()
        .flex_none()
        .items_center()
        .gap(m::SPACE_3)
        .h(m::LAYOUT_TITLEBAR_BUTTON)
        .px(m::SPACE_4)
        .rounded(m::RADIUS_MD)
        .text_size(m::FONT_SIZE_XS)
        .font_weight(m::FONT_WEIGHT_MEDIUM)
        .text_color(if pressed { tokens.accent_soft_text } else { fg })
        .when(pressed, |d| d.bg(colors.accent_soft))
        .cursor_pointer()
        .focus_visible(move |s| s.shadow(r.clone()))
        .hover(move |s| s.bg(hover))
        .role(gpui::Role::Button)
        .aria_label(t.label.clone())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    if let Some(on) = t.on {
        el = el.aria_toggled(if on {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        });
    }
    if let Some(icon) = t.icon {
        el = el.child(
            Icon::new(icon)
                .size(m::ICON_SIZE_SM)
                .color(colors.text_muted),
        );
    }
    on_activate_app(el.child(t.label), f)
}

impl Render for CodeViewer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let this = cx.entity();
        let (ui_family, ui_size) = components::live_settings::ui_font(cx);
        let lb = &self.labels;
        let can_jump = self.item().is_some_and(|it| !it.message.ts.0.is_empty());
        let has_doc = self.doc.is_some();
        let left = if cfg!(target_os = "macos") {
            m::LAYOUT_TITLEBAR_LIGHTS
        } else {
            m::SPACE_6
        };

        // Header: icon, file name, sender, place and time, Jump to message.
        let header = crate::titlebar::draggable(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(m::SPACE_5)
                .h(m::LAYOUT_TITLEBAR)
                .pl(left)
                .pr(m::SPACE_5)
                .bg(colors.background)
                .border_b_1()
                .border_color(tokens.border_subtle),
        )
        .child(
            Icon::new(IconName::Code)
                .size(m::ICON_SIZE_SM)
                .color(colors.text_muted),
        )
        .child(
            div()
                .debug_selector(|| "code-viewer-name".into())
                .flex_none()
                .max_w_1_2()
                .truncate()
                .text_size(m::FONT_SIZE_SM)
                .font_weight(m::FONT_WEIGHT_SEMIBOLD)
                .child(lb.name.clone()),
        )
        .children(lb.meta.clone().map(|meta| {
            div()
                .min_w_0()
                .truncate()
                .text_size(m::FONT_SIZE_XS)
                .text_color(tokens.text_subtle)
                .child(meta)
        }))
        .child(div().flex_1())
        .when(can_jump, |d| {
            let this = this.clone();
            d.child(tool_button(
                Tool {
                    id: "code-viewer-jump",
                    focus: &self.controls.jump,
                    icon: Some(IconName::Message),
                    label: lb.jump.clone(),
                    on: None,
                },
                &colors,
                &tokens,
                move |_, cx| this.update(cx, |v, cx| v.jump(cx)),
            ))
        });

        // Toolbar: language (a static label), Wrap, Line numbers, Copy, Download.
        let toolbar = div()
            .id("code-viewer-toolbar")
            .flex()
            .flex_none()
            .items_center()
            .gap(m::SPACE_2)
            .h(m::LAYOUT_CODE_VIEWER_TOOLBAR)
            .px(m::SPACE_5)
            .border_b_1()
            .border_color(tokens.border_subtle)
            .role(gpui::Role::Toolbar)
            .children(lb.language.clone().map(|lang| {
                div()
                    .debug_selector(|| "code-viewer-language".into())
                    .flex_none()
                    .px(m::SPACE_3)
                    .mr(m::SPACE_3)
                    .rounded(m::RADIUS_SM)
                    .border_1()
                    .border_color(tokens.border_subtle)
                    .text_color(colors.text_muted)
                    .text_size(m::FONT_SIZE_2XS)
                    .font_weight(m::FONT_WEIGHT_SEMIBOLD)
                    .child(lang)
            }))
            .child({
                let this = this.clone();
                tool_button(
                    Tool {
                        id: "code-viewer-wrap",
                        focus: &self.controls.wrap,
                        icon: None,
                        label: lb.wrap.clone(),
                        on: Some(self.wrap),
                    },
                    &colors,
                    &tokens,
                    move |_, cx| this.update(cx, |v, cx| v.toggle_wrap(cx)),
                )
            })
            .child({
                let this = this.clone();
                tool_button(
                    Tool {
                        id: "code-viewer-numbers",
                        focus: &self.controls.numbers,
                        icon: None,
                        label: lb.numbers.clone(),
                        on: Some(self.numbers),
                    },
                    &colors,
                    &tokens,
                    move |_, cx| this.update(cx, |v, cx| v.toggle_numbers(cx)),
                )
            })
            .child(div().flex_1())
            .when(has_doc, |d| {
                let this = this.clone();
                d.child(tool_button(
                    Tool {
                        id: "code-viewer-copy",
                        focus: &self.controls.copy,
                        icon: Some(IconName::Copy),
                        label: lb.copy.clone(),
                        on: None,
                    },
                    &colors,
                    &tokens,
                    move |_, cx| this.update(cx, |v, cx| v.copy_all(cx)),
                ))
            })
            .child({
                let this = this.clone();
                tool_button(
                    Tool {
                        id: "code-viewer-download",
                        focus: &self.controls.download,
                        icon: Some(IconName::Download),
                        label: lb.download.clone(),
                        on: None,
                    },
                    &colors,
                    &tokens,
                    move |_, cx| this.update(cx, |v, cx| v.download(cx)),
                )
            });

        // Body: the lines, or why there are none.
        let count = self.line_count();
        let lines: AnyElement = if has_doc {
            if self.wrap {
                list(self.list.clone(), cx.processor(Self::render_wrapped))
                    .size_full()
                    .into_any_element()
            } else {
                let longest = self.doc.as_ref().map(|d| d.longest_line());
                uniform_list("code-lines", count, cx.processor(Self::render_rows))
                    .track_scroll(&self.scroll)
                    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                    .with_width_from_item(longest)
                    .size_full()
                    .into_any_element()
            }
        } else {
            div()
                .debug_selector(|| "code-viewer-empty".into())
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p(m::SPACE_8)
                .text_size(m::FONT_SIZE_SM)
                .text_color(colors.text_muted)
                .font_family(ui_family.clone())
                .child(lb.empty.clone())
                .into_any_element()
        };

        let find_bar = self.find_open.then(|| {
            let step = |id: &'static str, icon: IconName, label: SharedString, next: bool| {
                let this = this.clone();
                components::controls::icon_button(id, icon, label, &colors, move |_, cx| {
                    this.update(cx, |v, cx| {
                        if next {
                            v.find_next(cx)
                        } else {
                            v.find_prev(cx)
                        }
                    })
                })
                .debug_selector(move || id.into())
            };
            let close = {
                let this = this.clone();
                components::controls::icon_button(
                    "code-viewer-find-close",
                    IconName::X,
                    lb.find_close.clone(),
                    &colors,
                    move |window, cx| this.update(cx, |v, cx| v.close_find(window, cx)),
                )
                .debug_selector(|| "code-viewer-find-close".into())
            };
            div()
                .id("code-viewer-find")
                .debug_selector(|| "code-viewer-find".into())
                .role(gpui::Role::Search)
                .aria_label(lb.find_region.clone())
                .absolute()
                .top(m::SPACE_4)
                .right(m::SPACE_7)
                .flex()
                .items_center()
                .gap(m::SPACE_3)
                .py(m::SPACE_3)
                .pl(m::SPACE_5)
                .pr(m::SPACE_3)
                .rounded(m::RADIUS_LG)
                .border_1()
                .border_color(colors.border)
                .bg(colors.elevated_surface_background)
                .font_family(ui_family.clone())
                .text_size(m::FONT_SIZE_SM)
                .child(
                    Icon::new(IconName::Search)
                        .size(m::ICON_SIZE_SM)
                        .color(colors.text_muted),
                )
                .child(
                    div()
                        .w(m::LAYOUT_CODE_VIEWER_FIND_INPUT)
                        .px(m::SPACE_4)
                        .py(m::SPACE_2)
                        .rounded(m::RADIUS_MD)
                        .border_1()
                        .border_color(colors.border_focused)
                        .bg(colors.surface_background)
                        .font_family(self.code_font.clone())
                        .child(self.input.clone()),
                )
                .child(
                    div()
                        .id("code-viewer-find-count")
                        .debug_selector(|| "code-viewer-find-count".into())
                        .role(gpui::Role::Status)
                        .aria_label(lb.find_count_label.clone())
                        .min_w(m::LAYOUT_CODE_VIEWER_FIND_COUNT)
                        .text_size(m::FONT_SIZE_XS)
                        .text_color(colors.text_muted)
                        .child(lb.find_count.clone()),
                )
                .child(step(
                    "code-viewer-find-prev",
                    IconName::ChevUp,
                    lb.find_prev.clone(),
                    false,
                ))
                .child(step(
                    "code-viewer-find-next",
                    IconName::ChevDown,
                    lb.find_next.clone(),
                    true,
                ))
                .child(close)
        });

        let body = div()
            .id("code-viewer-code")
            .debug_selector(|| "code-viewer-code".into())
            .role(gpui::Role::Region)
            .aria_label(lb.code_region.clone())
            .relative()
            .flex_1()
            .min_h_0()
            .py(m::SPACE_4)
            .bg(colors.surface_background)
            .font_family(self.code_font.clone())
            .text_size(m::FONT_SIZE_SM)
            .text_color(colors.text)
            .child(lines)
            .children(find_bar);

        // Status bar: what is loading or failed, then size, lines, language.
        let status = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(m::SPACE_6)
            .h(m::LAYOUT_CODE_VIEWER_STATUS)
            .px(m::SPACE_6)
            .bg(colors.background)
            .border_t_1()
            .border_color(colors.border)
            .text_size(m::FONT_SIZE_2XS)
            .text_color(colors.text_muted)
            .children(lb.note.clone().map(|(sel, text)| {
                div()
                    .debug_selector(move || sel.into())
                    .min_w_0()
                    .truncate()
                    .child(text)
            }))
            .child(div().flex_1())
            .children(lb.size.clone())
            .when(has_doc, |d| {
                d.child(
                    div()
                        .debug_selector(|| "code-viewer-lines".into())
                        .child(lb.lines.clone()),
                )
            })
            .children(lb.language.clone());

        div()
            .id("code-viewer")
            .track_focus(&self.focus)
            .key_context("CodeViewer")
            .capture_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .font_family(ui_family)
            .text_size(ui_size)
            .child(header)
            .child(toolbar)
            .child(body)
            .child(status)
    }
}

/// The open viewer window, reused for the next file.
#[derive(Default)]
struct OpenCode(Option<WindowHandle<CodeViewer>>);

impl Global for OpenCode {}

/// The viewer window, when one is open.
pub fn open_window(cx: &App) -> Option<WindowHandle<CodeViewer>> {
    cx.try_global::<OpenCode>().and_then(|o| o.0)
}

/// Closes the viewer (workspace switch, sign-out, workspace removal, idle lock).
pub fn close(cx: &mut App) {
    if let Some(h) = open_window(cx) {
        let _ = h.update(cx, |_, window, _| window.remove_window());
        cx.set_global(OpenCode(None));
    }
}

/// Opens the viewer on the current file of `gallery`, or shows it in the
/// viewer already open. The whole file loads through `host`'s fetcher (the
/// workspace that owns it).
pub fn open(gallery: Gallery, host: Rc<dyn CodeHost>, cx: &mut App) {
    if gallery.is_empty() {
        return;
    }
    let fetcher = host.fetcher(cx);
    if let Some(h) = open_window(cx) {
        let (g, f, ho) = (gallery.clone(), fetcher.clone(), host.clone());
        let reused = h
            .update(cx, |v, window, cx| {
                v.set_gallery(g, f, ho, window, cx);
                window.activate_window();
                v.focus(window, cx);
            })
            .is_ok();
        if reused {
            return;
        }
    }
    let bounds = Bounds::centered(
        None,
        size(m::LAYOUT_WINDOW_MEDIA_WIDTH, m::LAYOUT_WINDOW_MEDIA_HEIGHT),
        cx,
    );
    let opened = cx.open_window(
        crate::window_options::code_viewer(bounds),
        move |window, cx| {
            let v = cx.new(|cx| CodeViewer::new(gallery, fetcher, host, window, cx));
            v.update(cx, |v, cx| v.focus(window, cx));
            v
        },
    );
    cx.set_global(OpenCode(opened.ok()));
}
