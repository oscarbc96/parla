//! A scripted [`GifSource`] for tests and visual scenarios: answers load and
//! set-up calls from queues and records what it was asked. Enabled by the
//! `test-support` feature.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
};

use gpui::{App, SharedString, Task};

use super::{GifError, GifItem, GifPage, GifSource, GifTab, GifTabState};

/// One call the fake served.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeRequest {
    pub tab: SharedString,
    pub query: Option<String>,
    pub cursor: Option<SharedString>,
}

/// What the fake has been asked and will answer. Tests may edit it through
/// [`FakeGifSource::state`].
#[derive(Default)]
pub struct FakeState {
    pub tabs: Vec<GifTab>,
    pub requests: Vec<FakeRequest>,
    /// Answers to `load`, in order; an empty queue answers an empty page.
    pub answers: VecDeque<Result<GifPage, GifError>>,
    /// Calls to `set_up`: `(tab, key)`.
    pub setups: Vec<(String, String)>,
    /// The answer to the next `set_up`; `Ok` when unset.
    pub setup_result: Option<Result<(), GifError>>,
    /// Calls to `closed`.
    pub closed: usize,
}

/// A [`GifSource`] the test drives. Cheap to clone: clones share the state.
#[derive(Clone, Default)]
pub struct FakeGifSource(Arc<Mutex<FakeState>>);

impl FakeGifSource {
    pub fn new(tabs: Vec<GifTab>) -> Self {
        Self(Arc::new(Mutex::new(FakeState {
            tabs,
            ..Default::default()
        })))
    }

    /// Queues the answer to the next `load`.
    pub fn with_answer(self, answer: Result<GifPage, GifError>) -> Self {
        self.state().answers.push_back(answer);
        self
    }

    /// Queues the answer to the next `set_up`.
    pub fn with_setup_result(self, result: Result<(), GifError>) -> Self {
        self.state().setup_result = Some(result);
        self
    }

    /// The recorded state.
    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.0.lock().expect("fake gif source state")
    }
}

impl GifSource for FakeGifSource {
    fn tabs(&self, _: &App) -> Vec<GifTab> {
        self.state().tabs.clone()
    }

    fn load(
        &self,
        tab: &str,
        query: Option<String>,
        cursor: Option<SharedString>,
        _: &mut App,
    ) -> Task<Result<GifPage, GifError>> {
        let mut state = self.state();
        state.requests.push(FakeRequest {
            tab: tab.into(),
            query,
            cursor,
        });
        let answer = state.answers.pop_front().unwrap_or(Ok(GifPage::default()));
        Task::ready(answer)
    }

    fn set_up(&self, tab: &str, api_key: String, _: &mut App) -> Task<Result<(), GifError>> {
        let mut state = self.state();
        state.setups.push((tab.to_string(), api_key));
        let answer = state.setup_result.take().unwrap_or(Ok(()));
        if answer.is_ok()
            && let Some(t) = state.tabs.iter_mut().find(|t| t.id.as_ref() == tab)
        {
            t.state = GifTabState::Ready;
        }
        Task::ready(answer)
    }

    fn closed(&self, _: &mut App) {
        self.state().closed += 1;
    }
}

/// A tab that can search, with a host and a key page.
pub fn ready_tab(id: &str, title: &str) -> GifTab {
    GifTab {
        id: id.into(),
        title: title.into(),
        state: GifTabState::Ready,
        hosts: vec![format!("{id}.example.test").into()],
        key_url: Some(format!("https://{id}.example.test/keys").into()),
    }
}

/// A tab whose provider is not set up yet.
pub fn setup_tab(id: &str, title: &str) -> GifTab {
    GifTab {
        state: GifTabState::NeedsSetup,
        ..ready_tab(id, title)
    }
}

/// A result item; its pictures live on the example host the stage serves.
pub fn item(id: &str, title: &str, width: u32, height: u32) -> GifItem {
    GifItem {
        id: id.into(),
        preview_url: format!("https://gifs.example.test/{id}.png").into(),
        full_url: format!("https://gifs.example.test/{id}.gif").into(),
        width,
        height,
        bytes: 0,
        title: title.into(),
        attribution: None,
    }
}

/// `items` in a page whose next cursor is `next`.
pub fn page(items: Vec<GifItem>, next: Option<&str>) -> GifPage {
    GifPage {
        items,
        next: next.map(SharedString::from),
    }
}
