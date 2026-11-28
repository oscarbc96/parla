//! The GIF picker popover: provider tabs, a search box, a three-column grid of
//! results and the set-up card of a provider that has no key yet.

use std::time::Duration;

use domain::ImageRef;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyBinding, Render,
    ScrollHandle, SharedString, Subscription, Window, actions, div, prelude::*,
};
use settings::GifAutoplay;
use themes::{ActiveTheme, metrics};

use crate::media::MediaImage;
use crate::text_input::{TextInput, TextInputEvent};

use super::{GifError, GifItem, GifPage, GifSources, GifTab, GifTabState};

actions!(
    gif_picker,
    [
        Confirm,
        Insert,
        SelectNext,
        SelectPrev,
        SelectLeft,
        SelectRight,
        NextTab,
        PrevTab,
        Dismiss
    ]
);

/// Key context of the picker view.
pub const KEY_CONTEXT: &str = "GifPicker";

/// Tiles per row.
pub const COLUMNS: usize = 3;

/// Wait after the last keystroke before the search runs.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);

/// The most results the picker holds; older pages stop loading past this.
pub const MAX_ITEMS: usize = 200;

/// Registers the default picker key bindings.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Confirm, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-enter", Insert, Some(KEY_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(KEY_CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(KEY_CONTEXT)),
        KeyBinding::new("left", SelectLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(KEY_CONTEXT)),
        KeyBinding::new("tab", NextTab, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", PrevTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
    ]);
}

/// Whether a source with at least one tab is installed.
pub fn enabled(cx: &App) -> bool {
    cx.try_global::<GifSources>()
        .is_some_and(|s| !s.0.tabs(cx).is_empty())
}

/// Tells the source the picker closed, so idle provider instances can go.
pub fn notify_closed(cx: &mut App) {
    let source = cx.try_global::<GifSources>().map(|s| s.0.clone());
    if let Some(source) = source {
        source.closed(cx);
    }
}

/// How a picked GIF leaves the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GifPickMode {
    /// A message of its own.
    Send,
    /// Into the draft, at the caret.
    Insert,
}

/// Emitted when the user picks a GIF.
#[derive(Clone, Debug, PartialEq)]
pub struct GifPicked {
    pub item: GifItem,
    pub mode: GifPickMode,
}

/// Emitted when the picker is dismissed without a choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GifPickerDismissed;

/// What the body shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Stage {
    Loading,
    Grid,
    Empty,
    Failed(GifError),
    Setup,
}

pub struct GifPicker {
    source: Option<std::sync::Arc<dyn super::GifSource>>,
    tabs: Vec<GifTab>,
    tab: usize,
    search: Entity<TextInput>,
    /// The text the last search ran for.
    query: String,
    /// The text the search box had when it was last looked at.
    typed: String,
    stage: Stage,
    items: Vec<GifItem>,
    next: Option<SharedString>,
    attribution: Option<SharedString>,
    selected: usize,
    scroll: ScrollHandle,
    focus: FocusHandle,
    /// The debounce of a typed query; dropped when another key arrives.
    pending: Option<gpui::Task<()>>,
    /// The page in flight; dropped when the query changes or the tab does.
    request: Option<gpui::Task<()>>,
    setup_key: Entity<TextInput>,
    setup_busy: bool,
    setup_error: Option<GifError>,
    setup_task: Option<gpui::Task<()>>,
    _search: Subscription,
}

impl EventEmitter<GifPicked> for GifPicker {}
impl EventEmitter<GifPickerDismissed> for GifPicker {}

impl Focusable for GifPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
}

impl GifPicker {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::new_with_query(cx, String::new())
    }

    /// Opens with `query` already searched, so the first request is the search
    /// and not trending (used by `/gif <query>`).
    pub fn new_with_query(cx: &mut Context<Self>, query: String) -> Self {
        let source = cx.try_global::<GifSources>().map(|s| s.0.clone());
        let mut tabs = source.as_ref().map_or_else(Vec::new, |s| s.tabs(cx));
        // Only the tabs the user listed in `gifs.providers` are offered.
        tabs.retain(|t| !t.id.is_empty());
        let search = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_single_line(true);
            // Down belongs to the grid while it is open.
            input.set_intercept_down(true);
            if !query.is_empty() {
                input.set_text(&query, cx);
            }
            input
        });
        let setup_key = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_single_line(true);
            input.set_masked(true);
            input
        });
        let sub = cx.subscribe(&search, |this, input, _: &TextInputEvent, cx| {
            this.query_changed(input.read(cx).text().to_string(), cx)
        });
        let mut picker = Self {
            source,
            tabs,
            tab: 0,
            search,
            query: query.clone(),
            typed: query,
            stage: Stage::Loading,
            items: Vec::new(),
            next: None,
            attribution: None,
            selected: 0,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            pending: None,
            request: None,
            setup_key,
            setup_busy: false,
            setup_error: None,
            setup_task: None,
            _search: sub,
        };
        picker.reload(cx);
        picker
    }

    /// The provider tabs, in `gifs.providers` order.
    pub fn tabs(&self) -> &[GifTab] {
        &self.tabs
    }

    /// The active tab's index.
    pub fn tab(&self) -> usize {
        self.tab
    }

    /// The text in the search box.
    pub fn query(&self, cx: &App) -> String {
        self.search.read(cx).text().to_string()
    }

    /// The loaded results, newest page last.
    pub fn items(&self) -> &[GifItem] {
        &self.items
    }

    /// The item the keyboard is on.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The selected item, when the grid has one.
    pub fn selected_item(&self) -> Option<&GifItem> {
        self.items.get(self.selected)
    }

    /// The error the grid shows, if it failed.
    pub fn error(&self) -> Option<&GifError> {
        match &self.stage {
            Stage::Failed(e) => Some(e),
            _ => None,
        }
    }

    /// The set-up card is what the body shows.
    pub fn setup_shown(&self) -> bool {
        self.stage == Stage::Setup
    }

    /// The search box's focus handle (the picker's own).
    pub fn search_focus(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }

    /// Tabs own `enter` in the search box: it searches now; in the grid it
    /// sends the focused GIF.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stage == Stage::Setup && self.setup_key_focused(window, cx) {
            self.enable(cx);
            return;
        }
        if self.search.read(cx).focus_handle(cx).is_focused(window) {
            let text = self.search.read(cx).text().to_string();
            self.typed = text.clone();
            self.pending = None;
            self.query = text;
            self.reload(cx);
            return;
        }
        self.pick(GifPickMode::Send, cx);
    }

    fn pick(&mut self, mode: GifPickMode, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(self.selected).cloned() else {
            return;
        };
        cx.emit(GifPicked { item, mode });
    }

    fn setup_key_focused(&self, window: &Window, cx: &App) -> bool {
        self.setup_key.read(cx).focus_handle(cx).is_focused(window)
    }

    fn query_changed(&mut self, text: String, cx: &mut Context<Self>) {
        if text == self.typed {
            return;
        }
        self.typed = text.clone();
        // A newer query cancels the request and the timer of the older one.
        self.request = None;
        self.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                this.pending = None;
                this.query = text;
                this.reload(cx);
            })
            .ok();
        }));
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.request = None;
        self.items.clear();
        self.next = None;
        self.attribution = None;
        self.selected = 0;
        self.scroll = ScrollHandle::new();
        self.setup_error = None;
        let tab = self.tabs.get(self.tab).cloned();
        match tab {
            None => {
                self.stage = Stage::Empty;
                cx.notify();
            }
            Some(tab) if tab.state == GifTabState::NeedsSetup => {
                self.stage = Stage::Setup;
                cx.notify();
            }
            Some(_) => {
                self.stage = Stage::Loading;
                self.load_page(None, cx);
            }
        }
    }

    fn load_page(&mut self, cursor: Option<SharedString>, cx: &mut Context<Self>) {
        let (Some(source), Some(tab)) = (self.source.clone(), self.tabs.get(self.tab).cloned())
        else {
            return;
        };
        let query = (!self.query.trim().is_empty()).then(|| self.query.clone());
        let more = cursor.is_some();
        let task = source.load(tab.id.as_ref(), query, cursor, cx);
        self.request = Some(cx.spawn(async move |this, cx| {
            let answer = task.await;
            this.update(cx, |this, cx| this.page_loaded(answer, more, cx))
                .ok();
        }));
        cx.notify();
    }

    fn page_loaded(
        &mut self,
        answer: Result<GifPage, GifError>,
        more: bool,
        cx: &mut Context<Self>,
    ) {
        self.request = None;
        match answer {
            Ok(page) => {
                if more {
                    self.items.extend(page.items);
                } else {
                    self.items = page.items;
                }
                if self.attribution.is_none() {
                    self.attribution = self.items.iter().find_map(|i| i.attribution.clone());
                }
                self.next = page.next;
                if self.items.len() > MAX_ITEMS {
                    // The oldest pages stay; loading stops at the cap.
                    self.items.truncate(MAX_ITEMS);
                    self.next = None;
                }
                self.selected = self.selected.min(self.items.len().saturating_sub(1));
                self.stage = if self.items.is_empty() {
                    Stage::Empty
                } else {
                    Stage::Grid
                };
            }
            Err(GifError::NeedsSetup) => {
                if let Some(tab) = self.tabs.get_mut(self.tab) {
                    tab.state = GifTabState::NeedsSetup;
                }
                self.stage = Stage::Setup;
            }
            Err(error) => self.stage = Stage::Failed(error),
        }
        cx.notify();
    }

    /// Loads the next page when the provider has one and nothing is in flight.
    fn maybe_load_more(&mut self, cx: &mut Context<Self>) {
        if self.request.is_some() || self.stage != Stage::Grid {
            return;
        }
        let Some(next) = self.next.clone() else {
            return;
        };
        let n = self.items.len();
        if n == 0 || self.selected + COLUMNS < n {
            return;
        }
        self.load_page(Some(next), cx);
    }

    /// Loads the next page for a scroll that reached the end of the grid.
    fn scrolled_to_end(&mut self, cx: &mut Context<Self>) {
        if self.request.is_some() || self.stage != Stage::Grid {
            return;
        }
        let Some(next) = self.next.clone() else {
            return;
        };
        let offset = self.scroll.offset().y;
        let max = self.scroll.max_offset().y;
        if max > gpui::px(0.0) && offset >= max - metrics::SPACE_8 {
            self.load_page(Some(next), cx);
        }
    }

    fn select_tab(&mut self, tab: usize, window: &mut Window, cx: &mut Context<Self>) {
        if tab == self.tab || tab >= self.tabs.len() {
            return;
        }
        self.tab = tab;
        self.pending = None;
        self.query = self.search.read(cx).text().to_string();
        self.reload(cx);
        let focus = self.search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        // Up from the first row returns to the search box.
        if delta == -(COLUMNS as isize) && self.selected < COLUMNS {
            let focus = self.search.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            cx.notify();
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, n as isize - 1) as usize;
        self.scroll.scroll_to_item(self.selected);
        self.maybe_load_more(cx);
        cx.notify();
    }

    fn enter_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.items.is_empty() || self.stage != Stage::Grid {
            return;
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Retries the first page of the current query.
    fn retry(&mut self, cx: &mut Context<Self>) {
        self.reload(cx);
    }

    /// The autoplay override a tile is built with: the focused tile plays, the
    /// others play only while hovered.
    pub fn tile_autoplay(&self, index: usize) -> GifAutoplay {
        if index == self.selected {
            GifAutoplay::Visible
        } else {
            GifAutoplay::Hover
        }
    }

    /// The image of result `index`, boxed as the grid draws it.
    pub fn tile_image(&self, index: usize) -> Option<MediaImage> {
        let item = self.items.get(index)?;
        let (w, h) = tile_box(item);
        Some(
            MediaImage::new(
                ImageRef::Web(std::sync::Arc::from(item.preview_url.as_ref())),
                item.title.to_string(),
            )
            .max_size(w.as_f32(), h.as_f32())
            .cover()
            .autoplay(self.tile_autoplay(index)),
        )
    }

    fn open_key_url(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.tabs.get(self.tab).and_then(|t| t.key_url.clone()) {
            cx.open_url(&url);
        }
    }

    /// Stores the typed key, enables the provider and reloads its tab.
    fn enable(&mut self, cx: &mut Context<Self>) {
        if self.setup_busy {
            return;
        }
        let key = self.setup_key.read(cx).text().trim().to_string();
        if key.is_empty() {
            return;
        }
        let (Some(source), Some(tab)) = (self.source.clone(), self.tabs.get(self.tab).cloned())
        else {
            return;
        };
        self.setup_busy = true;
        self.setup_error = None;
        let task = source.set_up(tab.id.as_ref(), key, cx);
        let id = tab.id.clone();
        self.setup_task = Some(cx.spawn(async move |this, cx| {
            let answer = task.await;
            this.update(cx, |this, cx| this.set_up_done(id, answer, cx))
                .ok();
        }));
        cx.notify();
    }

    fn set_up_done(
        &mut self,
        id: SharedString,
        answer: Result<(), GifError>,
        cx: &mut Context<Self>,
    ) {
        self.setup_task = None;
        self.setup_busy = false;
        match answer {
            Ok(()) => {
                self.setup_key.update(cx, |i, cx| i.set_text("", cx));
                self.setup_error = None;
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
                    tab.state = GifTabState::Ready;
                }
                self.reload(cx);
            }
            Err(error) => {
                self.setup_error = Some(error);
                cx.notify();
            }
        }
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let colors = *cx.theme().colors();
        let mut row = div().flex().flex_none().gap(metrics::SPACE_1);
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = i == self.tab;
            row = row.child(
                div()
                    .id(("gif-tab", i))
                    .debug_selector(move || format!("gif-tab-{i}"))
                    .px(metrics::SPACE_5)
                    .py(metrics::SPACE_2)
                    .rounded(metrics::RADIUS_MD)
                    .text_size(metrics::FONT_SIZE_XS)
                    .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                    .text_color(if active {
                        colors.text
                    } else {
                        colors.text_muted
                    })
                    .when(active, |d| d.bg(colors.accent_soft))
                    .cursor_pointer()
                    .child(tab.title.clone())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.select_tab(i, window, cx)),
                    ),
            );
        }
        row
    }

    fn render_loading(&self, cx: &App) -> impl IntoElement + use<> {
        let fill = cx.theme().colors().element_selected;
        let mut grid = div()
            .debug_selector(|| "gif-loading".into())
            .flex()
            .flex_wrap()
            .content_start()
            .gap(metrics::SPACE_2)
            .flex_1()
            .min_h_0();
        for i in 0..6 {
            let (w, h) = tile_box(&placeholder_item(i));
            grid = grid.child(div().w(w).h(h).rounded(metrics::RADIUS_SM).bg(fill));
        }
        grid
    }

    fn render_error(&self, error: &GifError, cx: &mut Context<Self>) -> gpui::AnyElement {
        let l = i18n::gpui::localizer(cx);
        let (title, hint) = match error {
            GifError::NeedsSetup => (t!(l, "gif-error-failed"), t!(l, "gif-error-hint")),
            GifError::Offline => (t!(l, "gif-error-offline"), t!(l, "gif-error-hint")),
            GifError::RateLimited => (t!(l, "gif-error-rate-limited"), t!(l, "gif-error-hint")),
            GifError::Failed(_) => (t!(l, "gif-error-failed"), t!(l, "gif-error-hint")),
        };
        crate::load_state::load_error(
            "gif-error",
            "gif-retry",
            title.into_owned(),
            hint.into_owned(),
            t!(l, "gif-retry").into_owned(),
            cx,
            |this, _, cx| this.retry(cx),
        )
    }

    fn render_setup(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let Some(tab) = self.tabs.get(self.tab) else {
            return div().into_any_element();
        };
        let title = tab.title.clone();
        let hosts = tab
            .hosts
            .iter()
            .map(|h| h.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let key_empty = self.setup_key.read(cx).text().trim().is_empty();
        let enabled = !key_empty && !self.setup_busy;
        let l = i18n::gpui::localizer(cx);
        let mut card = div()
            .debug_selector(|| "gif-setup".into())
            .flex()
            .flex_col()
            .gap(metrics::SPACE_2)
            .p(metrics::SPACE_4)
            .rounded(metrics::RADIUS_MD)
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface_background)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_SM)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(t!(l, "gif-setup-title", title = title.to_string()).into_owned()),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(colors.text_muted)
                    .child(
                        t!(
                            l,
                            "gif-setup-body",
                            title = title.to_string(),
                            hosts = hosts
                        )
                        .into_owned(),
                    ),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(tokens.text_subtle)
                    .child(t!(l, "gif-setup-no-key").into_owned()),
            );
        if tab.key_url.is_some() {
            card = card.child(
                div()
                    .id("gif-setup-key-url")
                    .debug_selector(|| "gif-setup-key-url".into())
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(colors.text_accent)
                    .cursor_pointer()
                    .child(t!(l, "gif-setup-get-key").into_owned())
                    .on_click(cx.listener(|this, _, _, cx| this.open_key_url(cx))),
            );
        }
        card = card
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_XS)
                    .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                    .child(t!(l, "gif-setup-key").into_owned()),
            )
            .child(
                div().debug_selector(|| "gif-setup-key-field".into()).child(
                    crate::controls::field_frame(false, colors, tokens.border_control)
                        .child(self.setup_key.clone()),
                ),
            );
        if let Some(error) = &self.setup_error {
            let text = match error {
                GifError::Offline => t!(l, "gif-error-offline").into_owned(),
                _ => t!(l, "gif-setup-failed", title = title.to_string()).into_owned(),
            };
            card = card.child(
                div()
                    .debug_selector(|| "gif-setup-error".into())
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(theme.status().error)
                    .child(text),
            );
        }
        let label = if self.setup_busy {
            t!(l, "gif-setup-enabling").into_owned()
        } else {
            t!(l, "gif-setup-enable").into_owned()
        };
        card = card.child(
            div()
                .flex()
                .justify_end()
                .child(crate::controls::on_activate(
                    crate::controls::button(
                        "gif-setup-enable",
                        label,
                        crate::controls::ButtonKind::Primary,
                        enabled,
                        colors,
                    )
                    .debug_selector(|| "gif-setup-enable".into()),
                    cx,
                    |this, _, cx| this.enable(cx),
                )),
        );
        // Scrolls when the window is too short for the whole card.
        div()
            .id("gif-setup-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(card)
            .into_any_element()
    }
}

fn placeholder_item(i: usize) -> GifItem {
    GifItem {
        id: SharedString::from(format!("placeholder-{i}")),
        preview_url: SharedString::default(),
        full_url: SharedString::default(),
        width: 4,
        height: 3,
        bytes: 0,
        title: SharedString::default(),
        attribution: None,
    }
}

/// Width of one tile: three fit in the popover's inner width, minus its two
/// 1px borders and a hair of rounding room so the third never wraps.
fn tile_width() -> gpui::Pixels {
    (metrics::LAYOUT_POPOVER_LG
        - metrics::SPACE_5 * 2.0
        - metrics::SPACE_2 * 2.0
        - metrics::SPACE_1 * 2.0)
        * (1.0 / COLUMNS as f32)
}

/// The box of one tile: the item's aspect ratio, with the height capped.
fn tile_box(item: &GifItem) -> (gpui::Pixels, gpui::Pixels) {
    let w = tile_width();
    let ratio = if item.width == 0 || item.height == 0 {
        3.0 / 4.0
    } else {
        item.height as f32 / item.width as f32
    };
    let h = (w * ratio).clamp(metrics::SPACE_9 * 2.0, metrics::SPACE_11 * 2.0);
    (w, h)
}

impl Render for GifPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let l = i18n::gpui::localizer(cx);
        let Some(tab) = self.tabs.get(self.tab).cloned() else {
            return div();
        };
        let placeholder = t!(l, "gif-search", title = tab.title.to_string()).into_owned();
        self.search
            .update(cx, |i, _| i.set_placeholder(placeholder));

        let mut tabs = div().flex().flex_none().gap(metrics::SPACE_1);
        if self.tabs.len() > 1 {
            tabs = tabs.child(self.render_tabs(cx));
        }

        let body: gpui::AnyElement = match self.stage.clone() {
            Stage::Loading => self.render_loading(cx).into_any_element(),
            Stage::Grid => {
                let mut grid = div()
                    .id("gif-grid")
                    .debug_selector(|| "gif-grid".into())
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .on_scroll(cx.listener(|this, _, _, cx| this.scrolled_to_end(cx)))
                    .flex()
                    .flex_wrap()
                    .content_start()
                    .gap(metrics::SPACE_2)
                    .flex_1()
                    .min_h_0();
                for (i, item) in self.items.iter().enumerate() {
                    let selected = i == self.selected;
                    let (w, h) = tile_box(item);
                    let title = item.title.clone();
                    let image = self.tile_image(i);
                    grid = grid.child(
                        div()
                            .id(("gif-tile", i))
                            .debug_selector(move || format!("gif-tile-{i}"))
                            .role(gpui::Role::Button)
                            .aria_label(title.to_string())
                            .w(w)
                            .h(h)
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .rounded(metrics::RADIUS_SM)
                            .border_1()
                            .border_color(if selected {
                                colors.border_focused
                            } else {
                                gpui::transparent_black()
                            })
                            .cursor_pointer()
                            .children(image)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected = i;
                                this.pick(GifPickMode::Send, cx);
                            })),
                    );
                }
                grid.into_any_element()
            }
            Stage::Empty => crate::controls::empty_state(
                t!(l, "gif-empty").into_owned(),
                t!(l, "gif-empty-hint").into_owned(),
                &colors,
            )
            .debug_selector(|| "gif-empty".into())
            .flex_1()
            .into_any_element(),
            Stage::Failed(error) => self.render_error(&error, cx),
            Stage::Setup => self.render_setup(cx),
        };

        let selected = self.selected_item().map(|i| i.title.clone());
        let attribution = self.attribution.clone();
        // The footer describes a result; an empty, failed or set-up tab has none.
        let show_footer = self.stage == Stage::Grid;
        let mut footer = div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(metrics::SPACE_1)
            .pt(metrics::SPACE_1);
        footer = footer.child(
            div()
                .flex()
                .items_center()
                .gap(metrics::SPACE_4)
                .child(
                    div()
                        .debug_selector(|| "gif-selected-title".into())
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(colors.text_muted)
                        .children(selected),
                )
                .child(
                    div()
                        .debug_selector(|| "gif-hint".into())
                        .flex_none()
                        .text_size(metrics::FONT_SIZE_2XS)
                        .text_color(tokens.text_subtle)
                        .child(t!(l, "gif-hint").into_owned()),
                ),
        );
        if let Some(attribution) = attribution {
            footer = footer.child(
                div()
                    .text_size(metrics::FONT_SIZE_2XS)
                    .text_color(tokens.text_subtle)
                    .truncate()
                    .child(attribution),
            );
        }

        let search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.confirm(window, cx)))
            .on_action(cx.listener(|this, _: &Insert, _, cx| this.pick(GifPickMode::Insert, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, window, cx| {
                this.step(-(COLUMNS as isize), window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectNext, window, cx| {
                this.step(COLUMNS as isize, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectLeft, window, cx| this.step(-1, window, cx)))
            .on_action(cx.listener(|this, _: &SelectRight, window, cx| this.step(1, window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                let n = this.tabs.len();
                if n > 1 {
                    this.select_tab((this.tab + 1) % n, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                let n = this.tabs.len();
                if n > 1 {
                    this.select_tab((this.tab + n - 1) % n, window, cx);
                }
            }))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(GifPickerDismissed)))
            // The search box leaves Down to the grid.
            .on_action(
                cx.listener(|this, _: &crate::text_input::Down, window, cx| {
                    this.enter_grid(window, cx)
                }),
            )
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .p(metrics::SPACE_5)
            .w(metrics::LAYOUT_POPOVER_LG)
            .max_h(metrics::LAYOUT_POPOVER_MAX_HEIGHT)
            .overflow_hidden()
            .rounded(metrics::RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .child(tabs)
            .child(
                crate::controls::field_frame(search_focused, &colors, tokens.border_control)
                    .child(self.search.clone()),
            )
            .child(body)
            .when(show_footer, |d| d.child(footer))
    }
}
