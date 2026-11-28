//! What the GIF picker needs from the app. `components` knows nothing about
//! plugins: the app implements [`GifSource`] over its media-provider plugins
//! and installs it as the [`GifSources`] global. Without the global the
//! composer has no GIF button.

use std::sync::Arc;

use gpui::{App, Global, SharedString, Task};

/// Whether a tab can search yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GifTabState {
    Ready,
    /// The provider is bundled but the user has not accepted it or it has no
    /// API key: the tab shows the set-up card.
    NeedsSetup,
}

/// One provider tab, in `gifs.providers` order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GifTab {
    /// The media provider id, for example the plugin id.
    pub id: SharedString,
    /// The service's name as it wants to be shown; also used in the search
    /// placeholder ("Search {title}") and the attribution line.
    pub title: SharedString,
    pub state: GifTabState,
    /// Hosts the provider talks to, shown on the set-up card.
    pub hosts: Vec<SharedString>,
    /// Where the user gets an API key (https), shown on the set-up card.
    pub key_url: Option<SharedString>,
}

/// One search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GifItem {
    pub id: SharedString,
    /// Small rendition shown in the grid (https).
    pub preview_url: SharedString,
    /// What is sent (https).
    pub full_url: SharedString,
    pub width: u32,
    pub height: u32,
    /// Size of the full rendition; 0 when unknown.
    pub bytes: u64,
    pub title: SharedString,
    /// For example "Powered by ..."; shown under the grid.
    pub attribution: Option<SharedString>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GifPage {
    pub items: Vec<GifItem>,
    /// Opaque cursor of the next page; `None` on the last one.
    pub next: Option<SharedString>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GifError {
    /// No key, or the service refused it: show the set-up card.
    NeedsSetup,
    Offline,
    RateLimited,
    /// Anything else; the text is for the log, the picker shows a generic
    /// message with Retry.
    Failed(String),
}

/// The app's GIF providers. Methods are called on the main thread and must
/// not block; the work runs in the returned task.
pub trait GifSource: Send + Sync + 'static {
    /// The provider tabs, in the user's order. Cheap: called on render.
    fn tabs(&self, cx: &App) -> Vec<GifTab>;
    /// One page: trending when `query` is `None` or blank, else a search.
    fn load(
        &self,
        tab: &str,
        query: Option<String>,
        cursor: Option<SharedString>,
        cx: &mut App,
    ) -> Task<Result<GifPage, GifError>>;
    /// The user accepted the set-up card: store the key and enable the
    /// provider. An empty key keeps the stored one.
    fn set_up(&self, tab: &str, api_key: String, cx: &mut App) -> Task<Result<(), GifError>>;
    /// The picker closed: idle provider instances may be dropped.
    fn closed(&self, cx: &mut App);
}

/// Set by the app when GIF providers exist.
#[derive(Clone)]
pub struct GifSources(pub Arc<dyn GifSource>);

impl Global for GifSources {}
