//! The clip card's playback controls in a timeline: buttons, the seek bar,
//! keys on the focused card, focus kept by the composer, and captions fetched
//! on demand with a size cap.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use components::clip::{ClipAction, ClipController};
use components::platform::{ClipError, Frame, VideoPlayer};
use components::timeline::CaptionsDone;
use components::{Timeline, TimelineSource};
use domain::test_support::{cid, member, msg, uid};
use domain::{
    Capabilities, ConversationId, FileId, FileKind, FileRef, HistoryPage, Input, Member, MessageId,
    MessageWindow, ProviderExt, RemoteUrl, UserId, VideoInfo, WorkspaceState, reduce,
};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, IntoElement, KeyDownEvent, Modifiers,
    ParentElement as _, Render, Styled as _, TestAppContext, VisualTestContext, Window, div,
    prelude::*,
};
use settings::{Clock, WorkspaceKey};

const VTT: &str =
    "WEBVTT\n\n00:00.000 --> 00:10.000\nHello there\n\n00:10.000 --> 00:20.000\nLater\n";

fn url(s: &str) -> RemoteUrl {
    RemoteUrl(Arc::from(s))
}

fn clip_file() -> FileRef {
    FileRef {
        id: FileId(Arc::from("V1")),
        name: Arc::from("walkthrough.mp4"),
        title: None,
        mime: Arc::from("video/mp4"),
        size: 4096,
        kind: FileKind::Video,
        thumbs: Arc::from([]),
        download: None,
        preview: None,
        video: Some(VideoInfo {
            mp4_low: Some(url("https://files.example/low.mp4")),
            mp4: None,
            poster: None,
            duration_ms: Some(60_000),
            vtt: Some(url("https://files.example/clip.vtt")),
        }),
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

fn clip_msg() -> MessageId {
    msg("C1", "1000.000001", "").id
}

type Log = Rc<RefCell<(Vec<String>, u64)>>;

struct Fake(Log);

impl VideoPlayer for Fake {
    fn open(&mut self, _: &Path) -> Result<(), ClipError> {
        Ok(())
    }
    fn play(&mut self) {
        self.0.borrow_mut().0.push("play".into());
    }
    fn pause(&mut self) {
        self.0.borrow_mut().0.push("pause".into());
    }
    fn seek(&mut self, ms: u64) {
        let mut l = self.0.borrow_mut();
        l.0.push(format!("seek {ms}"));
        l.1 = ms;
    }
    fn frame(&mut self) -> Option<Frame> {
        None
    }
    fn position_ms(&self) -> u64 {
        self.0.borrow().1
    }
    fn set_rate(&mut self, rate: f32) {
        self.0.borrow_mut().0.push(format!("rate {rate}"));
    }
    fn set_muted(&mut self, muted: bool) {
        self.0.borrow_mut().0.push(format!("muted {muted}"));
    }
    fn set_volume(&mut self, v: f32) {
        self.0.borrow_mut().0.push(format!("volume {v}"));
    }
}

/// What the source's caption fetch answers.
#[derive(Clone)]
enum Answer {
    Captions,
    TooLarge,
}

struct Src {
    state: WorkspaceState,
    fetched: Rc<RefCell<Vec<(String, u64)>>>,
    answer: Answer,
}

impl TimelineSource for Src {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow> {
        self.state.windows.get(c)
    }
    fn member(&self, u: &UserId) -> Option<&Member> {
        self.state.members.get(u)
    }
    fn self_user(&self) -> Option<&UserId> {
        self.state.self_user.as_ref()
    }
    fn capabilities(&self) -> &Capabilities {
        static CAPS: std::sync::OnceLock<Capabilities> = std::sync::OnceLock::new();
        CAPS.get_or_init(Capabilities::default)
    }
    fn load_older(&mut self, _: ConversationId, _: &mut App) {}
    fn edit(&mut self, _: MessageId, _: String, _: &mut App) {}
    fn delete(&mut self, _: MessageId, _: &mut App) {}
    fn react(&mut self, _: MessageId, _: domain::EmojiRef, _: bool, _: &mut App) {}
    fn custom_emoji(&self) -> Vec<domain::CustomEmoji> {
        Vec::new()
    }
    fn fetch_captions(&mut self, url: RemoteUrl, max: u64, done: CaptionsDone, cx: &mut App) {
        self.fetched.borrow_mut().push((url.0.to_string(), max));
        match self.answer {
            Answer::Captions => done(Ok(VTT.as_bytes().to_vec()), cx),
            Answer::TooLarge => done(Err("too large".into()), cx),
        }
    }
}

/// The timeline under a stand-in composer that holds keyboard focus and
/// records the keys it gets; the root records every key that bubbles up.
struct Root {
    composer: FocusHandle,
    timeline: Entity<Timeline<Src>>,
    typed: Rc<RefCell<Vec<String>>>,
    bubbled: Rc<RefCell<Vec<String>>>,
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (typed, bubbled) = (self.typed.clone(), self.bubbled.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(move |ev: &KeyDownEvent, _, _| {
                bubbled.borrow_mut().push(ev.keystroke.key.clone())
            })
            .child(div().flex_1().child(self.timeline.clone()))
            .child(
                div()
                    .id("composer")
                    .track_focus(&self.composer)
                    .on_key_down(move |ev: &KeyDownEvent, _, _| {
                        typed.borrow_mut().push(ev.keystroke.key.clone())
                    })
                    .child("composer"),
            )
    }
}

struct Setup<'a> {
    vcx: &'a mut VisualTestContext,
    log: Log,
    fetched: Rc<RefCell<Vec<(String, u64)>>>,
    typed: Rc<RefCell<Vec<String>>>,
    bubbled: Rc<RefCell<Vec<String>>>,
    _dir: tempfile::TempDir,
}

/// A timeline with one clip; `open` puts a playing controller on it.
fn setup(cx: &mut TestAppContext, answer: Answer, open: bool) -> Setup<'_> {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let mut state = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 1000);
    state.self_user = Some(uid("u1"));
    state.members.insert(uid("u2"), member("u2", "Bob"));
    let mut m = msg("C1", "1000.000001", "clip");
    m.author = Some(uid("u2"));
    m.files = Arc::from([clip_file()]);
    reduce(
        &mut state,
        Input::HistoryLoaded {
            conversation: cid("C1"),
            page: HistoryPage {
                messages: vec![m],
                has_more: false,
            },
        },
    );
    let fetched = Rc::new(RefCell::new(Vec::new()));
    let (typed, bubbled) = (Rc::default(), Rc::default());
    let (f, t, b) = (fetched.clone(), Rc::clone(&typed), Rc::clone(&bubbled));
    let (root, vcx) = cx.add_window_view(move |window, cx| {
        let src = cx.new(move |_| Src {
            state,
            fetched: f,
            answer,
        });
        let timeline = cx.new(|cx| Timeline::new(src, cid("C1"), Clock::H24, cx));
        let composer = cx.focus_handle();
        window.focus(&composer, cx);
        Root {
            composer,
            timeline,
            typed: t,
            bubbled: b,
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let log: Log = Rc::default();
    if open {
        let l = log.clone();
        let mut ctl = ClipController::new(
            &clip_file(),
            dir.path(),
            Box::new(move || Box::new(Fake(l.clone()))),
        );
        assert!(matches!(ctl.click(), ClipAction::Download { .. }));
        ctl.downloaded();
        let timeline = root.read_with(vcx, |r, _| r.timeline.clone());
        timeline.update(vcx, |t, cx| t.insert_clip(clip_file(), clip_msg(), ctl, cx));
    }
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    log.borrow_mut().0.clear();
    Setup {
        vcx,
        log,
        fetched,
        typed,
        bubbled,
        _dir: dir,
    }
}

fn redraw(vcx: &mut VisualTestContext) {
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
}

fn click(vcx: &mut VisualTestContext, sel: &'static str) {
    let at = vcx
        .debug_bounds(sel)
        .unwrap_or_else(|| panic!("{sel}"))
        .center();
    vcx.simulate_click(at, Modifiers::none());
    redraw(vcx);
}

fn calls(log: &Log) -> Vec<String> {
    log.borrow().0.clone()
}

#[gpui::test]
fn buttons_work_without_toggling_the_card(cx: &mut TestAppContext) {
    let s = setup(cx, Answer::Captions, true);
    for sel in [
        "clip-play-pause",
        "clip-seek",
        "clip-speed",
        "clip-mute",
        "clip-volume",
        "clip-captions",
    ] {
        assert!(s.vcx.debug_bounds(sel).is_some(), "{sel} is drawn");
    }
    click(s.vcx, "clip-speed");
    assert_eq!(calls(&s.log), vec!["rate 1.5".to_string()]);
    click(s.vcx, "clip-mute");
    assert_eq!(calls(&s.log).last().unwrap(), "muted true");
    click(s.vcx, "clip-play-pause");
    assert_eq!(calls(&s.log).last().unwrap(), "pause");
    assert!(
        !calls(&s.log).contains(&"play".to_string()),
        "a control press does not also toggle the card"
    );
}

#[gpui::test]
fn the_seek_bar_seeks_where_pressed_and_follows_a_drag(cx: &mut TestAppContext) {
    let s = setup(cx, Answer::Captions, true);
    let b = s.vcx.debug_bounds("clip-seek").unwrap();
    s.vcx.simulate_click(b.center(), Modifiers::none());
    redraw(s.vcx);
    let pos = s.log.borrow().1;
    assert!(
        (28_000..=32_000).contains(&pos),
        "the middle of a minute: {pos}"
    );
    assert!(!calls(&s.log).contains(&"pause".to_string()));
    // Press near the start, drag to three quarters.
    let y = b.center().y;
    let x = |f: f32| b.origin.x + b.size.width * f;
    s.vcx.simulate_mouse_down(
        gpui::point(x(0.1), y),
        gpui::MouseButton::Left,
        Modifiers::none(),
    );
    s.vcx.simulate_mouse_move(
        gpui::point(x(0.75), y),
        Some(gpui::MouseButton::Left),
        Modifiers::none(),
    );
    s.vcx.simulate_mouse_up(
        gpui::point(x(0.75), y),
        gpui::MouseButton::Left,
        Modifiers::none(),
    );
    redraw(s.vcx);
    let pos = s.log.borrow().1;
    assert!((40_000..=50_000).contains(&pos), "dragged to {pos}");
}

#[gpui::test]
fn bars_take_arrow_keys(cx: &mut TestAppContext) {
    let s = setup(cx, Answer::Captions, true);
    // The card, then the seek bar (drawn above the button row).
    for _ in 0..2 {
        s.vcx.update(|w, cx| w.focus_next(cx));
    }
    s.vcx.simulate_keystrokes("right right");
    redraw(s.vcx);
    assert_eq!(s.log.borrow().1, 10_000);
    s.vcx.simulate_keystrokes("end");
    redraw(s.vcx);
    // The end (a playing clip then rewinds, as at its natural end).
    assert!(calls(&s.log).contains(&"seek 60000".to_string()));
    s.vcx.simulate_keystrokes("home");
    redraw(s.vcx);
    assert_eq!(s.log.borrow().1, 0);
    // Play and pause, speed, mute, then the volume bar.
    for _ in 0..4 {
        s.vcx.update(|w, cx| w.focus_next(cx));
    }
    s.vcx.simulate_keystrokes("down down");
    redraw(s.vcx);
    assert!(
        calls(&s.log).iter().any(|c| c.starts_with("volume 0.8")),
        "{:?}",
        calls(&s.log)
    );
    s.vcx.simulate_keystrokes("end");
    redraw(s.vcx);
    assert_eq!(calls(&s.log).last().unwrap(), "volume 1");
}

#[gpui::test]
fn a_click_on_the_card_keeps_typing_in_the_composer(cx: &mut TestAppContext) {
    let s = setup(cx, Answer::Captions, true);
    click(s.vcx, "file-card-V1");
    assert_eq!(calls(&s.log), vec!["pause".to_string()]);
    click(s.vcx, "clip-speed");
    s.vcx.simulate_keystrokes("a");
    assert_eq!(*s.typed.borrow(), ["a"], "the composer keeps focus");
}

#[gpui::test]
fn keys_on_the_card_reached_with_tab(cx: &mut TestAppContext) {
    let s = setup(cx, Answer::Captions, true);
    s.vcx.update(|w, cx| w.focus_next(cx));
    s.vcx.simulate_keystrokes("right");
    redraw(s.vcx);
    assert_eq!(s.log.borrow().1, 5_000);
    s.vcx.simulate_keystrokes("left left");
    redraw(s.vcx);
    assert_eq!(s.log.borrow().1, 0, "clamped at the start");
    s.vcx.simulate_keystrokes("m");
    redraw(s.vcx);
    assert_eq!(calls(&s.log).last().unwrap(), "muted true");
    s.vcx.simulate_keystrokes("space");
    redraw(s.vcx);
    assert_eq!(calls(&s.log).last().unwrap(), "pause");
    assert!(
        s.bubbled.borrow().is_empty(),
        "handled keys stop at the card"
    );
}

#[gpui::test]
fn arrows_on_a_closed_clip_are_not_swallowed(cx: &mut TestAppContext) {
    let s = setup(cx, Answer::Captions, false);
    s.vcx.update(|w, cx| w.focus_next(cx));
    s.vcx.simulate_keystrokes("left right");
    assert_eq!(*s.bubbled.borrow(), ["left", "right"]);
}

#[gpui::test]
fn captions_are_fetched_on_demand_with_a_cap_and_shown(cx: &mut TestAppContext) {
    let s = setup(cx, Answer::Captions, true);
    assert!(
        s.vcx.debug_bounds("clip-caption").is_none(),
        "off by default"
    );
    assert!(
        s.fetched.borrow().is_empty(),
        "nothing fetched before asked"
    );
    click(s.vcx, "clip-captions");
    redraw(s.vcx);
    assert_eq!(
        *s.fetched.borrow(),
        [(
            "https://files.example/clip.vtt".to_string(),
            components::vtt::MAX_BYTES as u64
        )]
    );
    assert!(s.vcx.debug_bounds("clip-caption").is_some());
    click(s.vcx, "clip-captions");
    assert!(s.vcx.debug_bounds("clip-caption").is_none());
    click(s.vcx, "clip-captions");
    assert_eq!(s.fetched.borrow().len(), 1, "fetched once");
}

#[gpui::test]
fn captions_too_large_show_a_short_error(cx: &mut TestAppContext) {
    let s = setup(cx, Answer::TooLarge, true);
    click(s.vcx, "clip-captions");
    redraw(s.vcx);
    assert!(s.vcx.debug_bounds("clip-caption").is_none());
    assert!(s.vcx.debug_bounds("clip-captions-error").is_some());
}
