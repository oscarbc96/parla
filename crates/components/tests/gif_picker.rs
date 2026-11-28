//! The composer's GIF picker: a scripted source drives trending, search,
//! paging, the set-up card and the keyboard (GF-*).

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::gif_picker::fake::{FakeGifSource, item, page, ready_tab, setup_tab};
use components::gif_picker::{GifError, GifPicker, GifSources, GifTab, MAX_ITEMS, SEARCH_DEBOUNCE};
use components::{Composer, ComposerEvent, ComposerSource};
use domain::test_support::cid;
use domain::{Capabilities, ComposedText, ConversationId, MessageTs, MessageWindow, UserId};
use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};

/// `(gif id, send)` per `send_gif_upload` call.
type Uploads = Rc<RefCell<Vec<(String, bool)>>>;

struct Src {
    me: UserId,
    caps: Capabilities,
    slashes: Rc<RefCell<Vec<String>>>,
    /// Whether the session takes an uploaded GIF instead of sending the link.
    takes_gif_upload: bool,
    gif_uploads: Uploads,
}

impl ComposerSource for Src {
    fn window(&self, _: &ConversationId) -> Option<&MessageWindow> {
        None
    }
    fn self_user(&self) -> Option<&UserId> {
        Some(&self.me)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn run_slash(
        &mut self,
        _cmd: domain::SlashCommand,
        _cx: &mut gpui::App,
    ) -> Result<(), domain::SlashError> {
        self.slashes.borrow_mut().push("slash".into());
        Ok(())
    }
    fn send_gif_upload(
        &mut self,
        _c: ConversationId,
        _thread: Option<MessageTs>,
        item: &components::gif_picker::GifItem,
        send: bool,
        _cx: &mut gpui::App,
    ) -> bool {
        self.gif_uploads
            .borrow_mut()
            .push((item.id.to_string(), send));
        self.takes_gif_upload
    }
}

type Events = Rc<RefCell<Vec<ComposerEvent>>>;

#[allow(clippy::type_complexity)]
fn setup(
    cx: &mut TestAppContext,
    fake: Option<FakeGifSource>,
) -> (
    Entity<Composer<Src>>,
    Events,
    Rc<RefCell<Vec<String>>>,
    &mut VisualTestContext,
) {
    setup_with(cx, fake, Capabilities::default(), Rc::default())
}

/// Like [`setup`], with the session's capabilities (and recording GIF uploads).
#[allow(clippy::type_complexity)]
fn setup_with(
    cx: &mut TestAppContext,
    fake: Option<FakeGifSource>,
    caps: Capabilities,
    uploads: Uploads,
) -> (
    Entity<Composer<Src>>,
    Events,
    Rc<RefCell<Vec<String>>>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        if let Some(fake) = fake {
            cx.set_global(GifSources(Arc::new(fake)));
        }
    });
    let slashes: Rc<RefCell<Vec<String>>> = Rc::default();
    let s = slashes.clone();
    let src = cx.new(move |_| Src {
        me: domain::test_support::uid("u0"),
        caps,
        slashes: s,
        takes_gif_upload: true,
        gif_uploads: uploads,
    });
    let (view, vcx) = cx.add_window_view(move |_w, cx| Composer::new(src, cid("C1"), cx));
    let events: Events = Rc::default();
    let sink = events.clone();
    vcx.update(|w, cx| {
        cx.subscribe(&view, move |_, e: &ComposerEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach();
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx);
    });
    (view, events, slashes, vcx)
}

fn text(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> String {
    v.read_with(vcx, |c, cx| c.text(cx).to_string())
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.run_until_parked();
    let b = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    vcx.simulate_click(b.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
}

fn picker(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> Entity<GifPicker> {
    v.read_with(vcx, |c, _| c.gif_picker().cloned())
        .expect("the GIF picker is open")
}

fn open(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) {
    click(vcx, "composer-gif");
    assert!(v.read_with(vcx, |c, _| c.gif_picker().is_some()));
}

fn press(vcx: &mut VisualTestContext, keys: &str) {
    vcx.simulate_keystrokes(keys);
    vcx.run_until_parked();
}

fn ready_tabs() -> Vec<GifTab> {
    vec![
        ready_tab("one", "Example GIFs"),
        ready_tab("two", "Studio GIFs"),
    ]
}

fn three() -> Vec<components::gif_picker::GifItem> {
    vec![
        item("a", "A cat", 4, 3),
        item("b", "A dog", 4, 3),
        item("c", "A bird", 4, 3),
    ]
}

fn gif_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-shift-g"
    } else {
        "ctrl-shift-g"
    }
}

#[gpui::test]
fn opens_with_trending(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Ok(page(three(), None)));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    let p = picker(&v, vcx);
    assert_eq!(p.read_with(vcx, |p, _| p.items().len()), 3);
    assert_eq!(p.read_with(vcx, |p, _| p.selected()), 0);
    let state = handle.state();
    assert_eq!(state.requests.len(), 1);
    assert_eq!(state.requests[0].tab.as_ref(), "one");
    assert_eq!(state.requests[0].query, None, "opening loads trending");
    // The search box has the focus, ready for typing.
    let focused = vcx.update(|w, cx| p.read(cx).search_focus(cx).is_focused(w));
    assert!(focused);
}

#[gpui::test]
fn typing_searches_after_pause(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs())
        .with_answer(Ok(page(three(), None)))
        .with_answer(Ok(page(vec![item("cat", "A cat", 4, 3)], None)));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    vcx.simulate_input("cats");
    vcx.run_until_parked();
    assert_eq!(handle.state().requests.len(), 1, "nothing yet");
    vcx.executor()
        .advance_clock(SEARCH_DEBOUNCE + std::time::Duration::from_millis(1));
    vcx.run_until_parked();
    let state = handle.state();
    assert_eq!(state.requests.len(), 2);
    assert_eq!(state.requests[1].query.as_deref(), Some("cats"));
    let p = picker(&v, vcx);
    assert_eq!(p.read_with(vcx, |p, _| p.items().len()), 1);
}

#[gpui::test]
fn enter_sends_the_focused_gif(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Ok(page(three(), None)));
    let (v, events, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    press(vcx, "down");
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.selected()), 0);
    press(vcx, "enter");
    assert!(v.read_with(vcx, |c, _| c.gif_picker().is_none()));
    assert_eq!(text(&v, vcx), "");
    assert_eq!(
        events.borrow().as_slice(),
        &[ComposerEvent::Submit {
            body: ComposedText::plain("https://gifs.example.test/a.gif")
        }]
    );
}

#[gpui::test]
fn shift_enter_inserts_into_the_draft(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Ok(page(three(), None)));
    let (v, events, _s, vcx) = setup(cx, Some(fake));
    vcx.simulate_input("look ");
    open(&v, vcx);
    press(vcx, "down");
    press(vcx, "shift-enter");
    assert_eq!(text(&v, vcx), "look https://gifs.example.test/a.gif");
    assert!(events.borrow().is_empty(), "insert does not send");
    assert!(v.read_with(vcx, |c, _| c.gif_picker().is_none()));
    // The draft keeps the focus: typing goes on there.
    vcx.simulate_input("!");
    assert_eq!(text(&v, vcx), "look https://gifs.example.test/a.gif!");
}

#[gpui::test]
fn arrow_keys_move_in_the_grid(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Ok(page(
        vec![
            item("a", "A", 4, 3),
            item("b", "B", 4, 3),
            item("c", "C", 4, 3),
            item("d", "D", 4, 3),
            item("e", "E", 4, 3),
            item("f", "F", 4, 3),
        ],
        None,
    )));
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    press(vcx, "down");
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.selected()), 0);
    press(vcx, "right");
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.selected()), 1);
    press(vcx, "down");
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.selected()), 4);
    press(vcx, "left");
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.selected()), 3);
    press(vcx, "up");
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.selected()), 0);
    // Up from the first row returns to the search box.
    press(vcx, "up");
    let p = picker(&v, vcx);
    let focused = vcx.update(|w, cx| p.read(cx).search_focus(cx).is_focused(w));
    assert!(focused);
}

#[gpui::test]
fn only_focused_tile_animates(cx: &mut TestAppContext) {
    use settings::GifAutoplay;
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Ok(page(three(), None)));
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    let p = picker(&v, vcx);
    let modes: Vec<_> = p.read_with(vcx, |p, _| {
        (0..3)
            .map(|i| p.tile_image(i).and_then(|t| t.autoplay_mode()))
            .collect()
    });
    assert_eq!(
        modes,
        vec![
            Some(GifAutoplay::Visible),
            Some(GifAutoplay::Hover),
            Some(GifAutoplay::Hover)
        ]
    );
    press(vcx, "down");
    press(vcx, "right");
    let second: Option<GifAutoplay> =
        p.read_with(vcx, |p, _| p.tile_image(1).and_then(|t| t.autoplay_mode()));
    assert_eq!(second, Some(GifAutoplay::Visible));
}

#[gpui::test]
fn end_of_grid_loads_next_page_once(cx: &mut TestAppContext) {
    let first = page(
        vec![
            item("a", "A", 4, 3),
            item("b", "B", 4, 3),
            item("c", "C", 4, 3),
            item("d", "D", 4, 3),
            item("e", "E", 4, 3),
            item("f", "F", 4, 3),
        ],
        Some("2"),
    );
    let second = page(vec![item("g", "G", 4, 3), item("h", "H", 4, 3)], None);
    let fake = FakeGifSource::new(ready_tabs())
        .with_answer(Ok(first))
        .with_answer(Ok(second));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    assert_eq!(handle.state().requests.len(), 1);
    press(vcx, "down");
    press(vcx, "down");
    press(vcx, "down");
    {
        let state = handle.state();
        assert_eq!(state.requests.len(), 2, "the end of the grid asks once");
        assert_eq!(state.requests[1].cursor.as_deref(), Some("2"));
    }
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.items().len()), 8);
    press(vcx, "down");
    assert_eq!(
        handle.state().requests.len(),
        2,
        "no more pages, no request"
    );
    const { assert!(MAX_ITEMS >= 8) };
}

#[gpui::test]
fn needs_setup_shows_the_card(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(vec![
        setup_tab("one", "Example GIFs"),
        ready_tab("two", "Studio GIFs"),
    ]);
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    let p = picker(&v, vcx);
    assert!(p.read_with(vcx, |p, _| p.setup_shown()));
    assert!(vcx.debug_bounds("gif-setup").is_some());
    assert!(vcx.debug_bounds("gif-setup-enable").is_some());
    // Switching to the ready tab shows the grid again.
    let b = vcx.debug_bounds("gif-tab-1").expect("the second tab");
    vcx.simulate_click(b.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    assert!(!picker(&v, vcx).read_with(vcx, |p, _| p.setup_shown()));
}

#[gpui::test]
fn enable_stores_key_and_reloads(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(vec![setup_tab("one", "Example GIFs")])
        .with_answer(Ok(page(vec![item("a", "A cat", 4, 3)], None)));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    assert!(
        vcx.debug_bounds("gif-tab-0").is_none(),
        "one provider shows no tab row"
    );
    // The key field and Enable fit in the popover: neither is clipped away.
    let popover = vcx
        .debug_bounds("composer-gif-picker")
        .expect("the popover is drawn");
    for selector in ["gif-setup-key-field", "gif-setup-enable"] {
        let b = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        assert!(
            popover.contains(&b.center()),
            "{selector} is inside the visible popover"
        );
    }
    click(vcx, "gif-setup-key-field");
    vcx.simulate_input("secret-key");
    vcx.run_until_parked();
    click(vcx, "gif-setup-enable");
    let state = handle.state();
    assert_eq!(
        state.setups,
        vec![("one".to_string(), "secret-key".to_string())]
    );
    assert_eq!(
        state.tabs[0].state,
        components::gif_picker::GifTabState::Ready
    );
    assert_eq!(state.requests.len(), 1, "the tab reloads after enabling");
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.items().len()), 1);
}

#[gpui::test]
fn enter_searches_at_once(cx: &mut TestAppContext) {
    // Enter in the search box skips the debounce.
    let fake = FakeGifSource::new(ready_tabs())
        .with_answer(Ok(page(three(), None)))
        .with_answer(Ok(page(vec![item("cat", "A cat", 4, 3)], None)));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    vcx.simulate_input("cats");
    press(vcx, "enter");
    let state = handle.state();
    assert_eq!(state.requests.len(), 2, "no pause needed");
    assert_eq!(state.requests[1].query.as_deref(), Some("cats"));
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.items().len()), 1);
}

#[gpui::test]
fn enable_failure_shows_the_error(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(vec![setup_tab("one", "Example GIFs")])
        .with_setup_result(Err(GifError::Failed("bad key".into())));
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    click(vcx, "gif-setup-key-field");
    vcx.simulate_input("wrong-key");
    vcx.run_until_parked();
    click(vcx, "gif-setup-enable");
    let p = picker(&v, vcx);
    assert!(p.read_with(vcx, |p, _| p.setup_shown()), "the card stays");
    assert!(
        vcx.debug_bounds("gif-setup-error").is_some(),
        "the card says enabling failed"
    );
}

#[gpui::test]
fn error_shows_retry(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Err(GifError::Offline));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    let p = picker(&v, vcx);
    assert_eq!(
        p.read_with(vcx, |p, _| p.error().cloned()),
        Some(GifError::Offline)
    );
    assert!(vcx.debug_bounds("gif-error").is_some());
    assert!(vcx.debug_bounds("gif-retry").is_some());
    handle
        .state()
        .answers
        .push_back(Ok(page(vec![item("a", "A cat", 4, 3)], None)));
    click(vcx, "gif-retry");
    assert_eq!(handle.state().requests.len(), 2);
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.items().len()), 1);
}

#[gpui::test]
fn escape_closes_and_calls_closed(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Ok(page(three(), None)));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    press(vcx, "escape");
    assert!(v.read_with(vcx, |c, _| c.gif_picker().is_none()));
    assert_eq!(handle.state().closed, 1);
    let focused = vcx.update(|w, cx| v.read(cx).focus_handle(cx).is_focused(w));
    assert!(focused, "the editor has the focus again");
}

#[gpui::test]
fn no_source_no_button(cx: &mut TestAppContext) {
    let (v, _e, _s, vcx) = setup(cx, None);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("composer-gif").is_none());
    press(vcx, gif_shortcut());
    assert!(v.read_with(vcx, |c, _| c.gif_picker().is_none()));
}

#[gpui::test]
fn gif_slash_opens_picker_locally(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs())
        .with_answer(Ok(page(three(), None)))
        .with_answer(Ok(page(vec![item("cat", "A cat", 4, 3)], None)));
    let handle = fake.clone();
    let (v, _e, slashes, vcx) = setup(cx, Some(fake));
    vcx.simulate_input("/gif cats");
    press(vcx, "enter");
    assert!(v.read_with(vcx, |c, _| c.gif_picker().is_some()));
    assert_eq!(text(&v, vcx), "", "the command leaves the draft");
    assert!(slashes.borrow().is_empty(), "never reaches the provider");
    let p = picker(&v, vcx);
    assert_eq!(p.read_with(vcx, |p, cx| p.query(cx)), "cats");
    let state = handle.state();
    assert_eq!(state.requests.len(), 1);
    assert_eq!(state.requests[0].query.as_deref(), Some("cats"));
}

#[gpui::test]
fn send_keeps_the_draft(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Ok(page(three(), None)));
    let (v, events, _s, vcx) = setup(cx, Some(fake));
    vcx.simulate_input("keep me ");
    open(&v, vcx);
    press(vcx, "down");
    press(vcx, "enter");
    assert_eq!(text(&v, vcx), "keep me ", "the draft is untouched");
    assert_eq!(
        events.borrow().as_slice(),
        &[ComposerEvent::Submit {
            body: ComposedText::plain("https://gifs.example.test/a.gif")
        }]
    );
}

#[gpui::test]
fn fast_typing_loads_once(cx: &mut TestAppContext) {
    // One load for the whole word: the debounce restarts on every keystroke.
    let fake = FakeGifSource::new(ready_tabs())
        .with_answer(Ok(page(three(), None)))
        .with_answer(Ok(page(vec![item("cat", "A cat", 4, 3)], None)));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    for (key, pause) in [("c", 100), ("a", 100), ("t", 0)] {
        vcx.simulate_input(key);
        vcx.executor()
            .advance_clock(std::time::Duration::from_millis(pause));
        vcx.run_until_parked();
    }
    assert_eq!(handle.state().requests.len(), 1, "the pause is still ahead");
    vcx.executor().advance_clock(SEARCH_DEBOUNCE);
    vcx.run_until_parked();
    let state = handle.state();
    assert_eq!(state.requests.len(), 2, "one search for the whole word");
    assert_eq!(state.requests[1].query.as_deref(), Some("cat"));
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.items().len()), 1);
}

#[gpui::test]
fn tabs_come_from_the_source(cx: &mut TestAppContext) {
    let fake = FakeGifSource::new(ready_tabs())
        .with_answer(Ok(page(three(), None)))
        .with_answer(Ok(page(vec![item("studio", "Studio", 4, 3)], None)));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    let p = picker(&v, vcx);
    assert_eq!(
        p.read_with(vcx, |p, _| p
            .tabs()
            .iter()
            .map(|t| t.title.to_string())
            .collect::<Vec<_>>()),
        ["Example GIFs", "Studio GIFs"]
    );
    assert!(
        vcx.debug_bounds("gif-tab-1").is_some(),
        "a tab per provider"
    );
    click(vcx, "gif-tab-1");
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.tab()), 1);
    let state = handle.state();
    assert_eq!(state.requests.len(), 2);
    assert_eq!(state.requests[1].tab.as_ref(), "two");
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.items().len()), 1);
}

/// Asserts the picker shows `error` with Retry, then that Retry asks again.
fn error_state(cx: &mut TestAppContext, error: GifError) {
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Err(error.clone()));
    let handle = fake.clone();
    let (v, _e, _s, vcx) = setup(cx, Some(fake));
    open(&v, vcx);
    let p = picker(&v, vcx);
    assert_eq!(p.read_with(vcx, |p, _| p.error().cloned()), Some(error));
    assert!(vcx.debug_bounds("gif-error").is_some());
    assert!(vcx.debug_bounds("gif-retry").is_some());
    handle
        .state()
        .answers
        .push_back(Ok(page(vec![item("a", "A cat", 4, 3)], None)));
    click(vcx, "gif-retry");
    assert_eq!(handle.state().requests.len(), 2);
    assert_eq!(
        picker(&v, vcx).read_with(vcx, |p, _| p.error().cloned()),
        None
    );
    assert_eq!(picker(&v, vcx).read_with(vcx, |p, _| p.items().len()), 1);
}

#[gpui::test]
fn rate_limited_shows_retry(cx: &mut TestAppContext) {
    error_state(cx, GifError::RateLimited);
}

#[gpui::test]
fn failed_shows_retry(cx: &mut TestAppContext) {
    error_state(cx, GifError::Failed("boom".into()));
}

#[gpui::test]
fn upload_mode_calls_send_gif_upload(cx: &mut TestAppContext) {
    settings_with(cx, r#"{"gifs": {"send_as": "upload"}}"#);
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Ok(page(three(), None)));
    let uploads: Uploads = Rc::default();
    let (v, events, _s, vcx) = setup_with(
        cx,
        Some(fake),
        Capabilities {
            files: true,
            ..Default::default()
        },
        uploads.clone(),
    );
    vcx.simulate_input("keep me ");
    open(&v, vcx);
    press(vcx, "down");
    press(vcx, "enter");
    assert_eq!(
        uploads.borrow().as_slice(),
        &[("a".to_string(), true)],
        "the session uploads the focused GIF"
    );
    assert!(
        events.borrow().is_empty(),
        "the upload path sends no link message"
    );
    assert_eq!(text(&v, vcx), "keep me ");
}

#[gpui::test]
fn link_mode_sends_when_upload_is_off(cx: &mut TestAppContext) {
    settings_with(cx, r#"{"gifs": {"send_as": "link"}}"#);
    let fake = FakeGifSource::new(ready_tabs()).with_answer(Ok(page(three(), None)));
    let uploads: Uploads = Rc::default();
    let (v, events, _s, vcx) = setup_with(
        cx,
        Some(fake),
        Capabilities {
            files: true,
            ..Default::default()
        },
        uploads.clone(),
    );
    open(&v, vcx);
    press(vcx, "down");
    press(vcx, "enter");
    assert!(uploads.borrow().is_empty(), "link mode uploads nothing");
    assert_eq!(
        events.borrow().as_slice(),
        &[ComposerEvent::Submit {
            body: ComposedText::plain("https://gifs.example.test/a.gif")
        }]
    );
}

/// Installs a settings store whose file holds `json`.
fn settings_with(cx: &mut TestAppContext, json: &str) {
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::write(&paths.settings, json).unwrap();
    cx.update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    std::mem::forget(dir);
}
