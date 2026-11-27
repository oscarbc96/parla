mod common;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use components::clip::{
    ClipAction, ClipController, ClipEvent, ClipFailure, ClipKey, ClipState, SPEEDS, control_label,
    failure_text, pick_source, step,
};
use components::files::{Card, card};
use components::platform::{ClipError, Frame, VideoPlayer};
use domain::{FileId, FileKind, FileRef, ProviderExt, RemoteUrl, VideoInfo};
use settings::InlineImages;

fn url(s: &str) -> RemoteUrl {
    RemoteUrl(Arc::from(s))
}

fn clip(low: bool, full: bool) -> FileRef {
    FileRef {
        id: FileId(Arc::from("F9")),
        name: Arc::from("clip.mp4"),
        title: None,
        mime: Arc::from("video/mp4"),
        size: 4096,
        kind: FileKind::Video,
        thumbs: Arc::from([]),
        download: Some(url("https://files.example/orig")),
        preview: None,
        video: Some(VideoInfo {
            mp4_low: low.then(|| url("https://files.example/low.mp4")),
            mp4: full.then(|| url("https://files.example/full.mp4")),
            poster: None,
            duration_ms: Some(1000),
            vtt: Some(url("https://files.example/clip.vtt")),
        }),
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

#[derive(Default)]
struct Log {
    calls: Vec<String>,
    pos: u64,
    playing: bool,
    /// The decoder has no picture yet (as just after a seek).
    no_frame: bool,
}

struct Fake {
    log: Rc<RefCell<Log>>,
    open: Result<(), ClipError>,
}

impl VideoPlayer for Fake {
    fn open(&mut self, path: &Path) -> Result<(), ClipError> {
        self.log
            .borrow_mut()
            .calls
            .push(format!("open {}", path.display()));
        self.open.clone()
    }
    fn play(&mut self) {
        let mut l = self.log.borrow_mut();
        l.calls.push("play".into());
        l.playing = true;
    }
    fn pause(&mut self) {
        let mut l = self.log.borrow_mut();
        l.calls.push("pause".into());
        l.playing = false;
    }
    fn seek(&mut self, ms: u64) {
        let mut l = self.log.borrow_mut();
        l.calls.push(format!("seek {ms}"));
        l.pos = ms;
    }
    fn frame(&mut self) -> Option<Frame> {
        let mut l = self.log.borrow_mut();
        l.calls.push("frame".into());
        (!l.no_frame).then(|| Frame::bgra(2, 2, vec![0; 16]))
    }
    fn position_ms(&self) -> u64 {
        self.log.borrow().pos
    }
    /// As AVPlayer: a paused player ignores the speed.
    fn set_rate(&mut self, rate: f32) {
        let mut l = self.log.borrow_mut();
        if l.playing {
            l.calls.push(format!("rate {rate}"));
        }
    }
    fn set_volume(&mut self, volume: f32) {
        self.log.borrow_mut().calls.push(format!("volume {volume}"));
    }
    fn set_muted(&mut self, muted: bool) {
        self.log.borrow_mut().calls.push(format!("muted {muted}"));
    }
}

fn controller(
    f: &FileRef,
    dir: &Path,
    open: Result<(), ClipError>,
) -> (ClipController, Rc<RefCell<Log>>) {
    let log = Rc::new(RefCell::new(Log::default()));
    let l = log.clone();
    let c = ClipController::new(
        f,
        dir,
        Box::new(move || {
            Box::new(Fake {
                log: l.clone(),
                open: open.clone(),
            })
        }),
    );
    (c, log)
}

fn calls(log: &Rc<RefCell<Log>>) -> Vec<String> {
    log.borrow().calls.clone()
}

#[test]
fn click_downloads_then_plays() {
    let dir = tempfile::tempdir().unwrap();
    let f = clip(true, true);
    let (mut c, log) = controller(&f, dir.path(), Ok(()));
    assert_eq!(c.state(), &ClipState::Poster);
    let ClipAction::Download { url: u, dest } = c.click() else {
        panic!("first click downloads");
    };
    assert_eq!(&*u.0, "https://files.example/low.mp4");
    assert_eq!(
        dest.parent().unwrap(),
        dir.path().join("media").join("clips")
    );
    assert_eq!(dest.extension().unwrap(), "mp4");
    assert_eq!(c.state(), &ClipState::Downloading);
    assert!(
        calls(&log).is_empty(),
        "nothing plays before the download ends"
    );
    c.downloaded();
    assert_eq!(c.state(), &ClipState::Playing);
    assert_eq!(calls(&log).last().unwrap(), "play");
    // A second click pauses, a third resumes.
    assert_eq!(c.click(), ClipAction::None);
    assert_eq!(c.state(), &ClipState::Paused);
    assert_eq!(c.click(), ClipAction::None);
    assert_eq!(c.state(), &ClipState::Playing);
}

#[test]
fn cached_file_skips_download() {
    let dir = tempfile::tempdir().unwrap();
    let f = clip(true, false);
    let (mut c, _) = controller(&f, dir.path(), Ok(()));
    let ClipAction::Download { dest, .. } = c.click() else {
        panic!()
    };
    c.release();
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, b"x").unwrap();
    assert_eq!(c.click(), ClipAction::None);
    assert_eq!(c.state(), &ClipState::Playing);
}

#[test]
fn prefers_mp4_low() {
    let both = clip(true, true);
    assert_eq!(
        &*pick_source(&both).unwrap().0,
        "https://files.example/low.mp4"
    );
    let full = clip(false, true);
    assert_eq!(
        &*pick_source(&full).unwrap().0,
        "https://files.example/full.mp4"
    );
    let neither = clip(false, false);
    assert_eq!(
        &*pick_source(&neither).unwrap().0,
        "https://files.example/orig"
    );
    let mut none = clip(false, false);
    none.download = None;
    assert!(pick_source(&none).is_none());
}

#[test]
fn scroll_away_and_inactive_pause() {
    let dir = tempfile::tempdir().unwrap();
    let f = clip(true, true);
    for ev in [ClipEvent::ScrolledAway, ClipEvent::WindowInactive] {
        assert_eq!(step(ClipState::Playing, ev), ClipState::Paused);
    }
    let (mut c, log) = controller(&f, dir.path(), Ok(()));
    c.click();
    c.downloaded();
    c.scrolled_away();
    assert_eq!(c.state(), &ClipState::Paused);
    assert_eq!(calls(&log).last().unwrap(), "pause");
    c.click();
    assert_eq!(c.state(), &ClipState::Playing);
    c.window_inactive();
    assert_eq!(c.state(), &ClipState::Paused);
    // Returning does not resume by itself.
    assert!(c.tick().is_none());
    assert_eq!(c.state(), &ClipState::Paused);
}

#[test]
fn unsupported_falls_back_to_browser() {
    let dir = tempfile::tempdir().unwrap();
    let f = clip(true, true);
    let (mut c, _) = controller(&f, dir.path(), Err(ClipError::Unsupported));
    c.click();
    c.downloaded();
    assert!(matches!(c.state(), ClipState::Fallback(_)));
    assert_eq!(c.click(), ClipAction::OpenInBrowser);
    let (mut d, _) = controller(&f, dir.path(), Ok(()));
    d.click();
    d.download_failed("offline".into());
    assert_eq!(
        d.state(),
        &ClipState::Fallback(ClipFailure::Download("offline".into()))
    );
    assert_eq!(
        step(
            ClipState::Ready,
            ClipEvent::OpenFailed(ClipError::DecoderFailed(Some("x".into())))
        ),
        ClipState::Fallback(ClipFailure::Open(ClipError::DecoderFailed(Some(
            "x".into()
        ))))
    );
}

#[test]
fn never_autoplays() {
    let dir = tempfile::tempdir().unwrap();
    let f = clip(true, true);
    let (mut c, log) = controller(&f, dir.path(), Ok(()));
    for ev in [
        ClipEvent::Downloaded,
        ClipEvent::Opened,
        ClipEvent::ScrolledAway,
        ClipEvent::WindowInactive,
        ClipEvent::Ended,
    ] {
        assert_eq!(step(ClipState::Poster, ev), ClipState::Poster);
    }
    c.scrolled_away();
    c.window_inactive();
    assert!(c.tick().is_none());
    assert_eq!(c.state(), &ClipState::Poster);
    assert!(calls(&log).is_empty(), "no player activity before a click");
}

#[test]
fn frames_pump_only_while_playing_and_end_rewinds() {
    let dir = tempfile::tempdir().unwrap();
    let f = clip(true, true);
    let (mut c, log) = controller(&f, dir.path(), Ok(()));
    c.click();
    c.downloaded();
    assert!(c.tick().is_some());
    log.borrow_mut().pos = 1000;
    c.tick();
    assert_eq!(c.state(), &ClipState::Paused);
    assert!(calls(&log).contains(&"seek 0".to_string()));
}

#[test]
fn release_drops_player_and_frame() {
    let dir = tempfile::tempdir().unwrap();
    let f = clip(true, true);
    let (mut c, _) = controller(&f, dir.path(), Ok(()));
    c.click();
    c.downloaded();
    c.tick();
    assert!(c.last_frame().is_some());
    c.release();
    assert!(c.last_frame().is_none());
    assert_eq!(c.state(), &ClipState::Poster);
}

#[test]
fn video_and_audio_files_get_a_clip_card() {
    let v = clip(true, true);
    match card(&v, InlineImages::OnClick, false) {
        Card::Clip {
            duration_ms, audio, ..
        } => {
            assert_eq!(duration_ms, Some(1000));
            assert!(!audio);
        }
        c => panic!("{c:?}"),
    }
    let mut a = clip(false, false);
    a.kind = FileKind::Audio;
    a.video = None;
    assert!(matches!(
        card(&a, InlineImages::OnClick, false),
        Card::Clip { audio: true, .. }
    ));
    a.download = None;
    assert!(matches!(
        card(&a, InlineImages::OnClick, false),
        Card::Plain { .. }
    ));
}

#[test]
fn failures_read_in_the_active_language() {
    let f = ClipFailure::Open(ClipError::Unsupported);
    assert_eq!(
        failure_text(&f, &common::en()),
        "Playback is not supported on this system yet."
    );
    assert_eq!(
        failure_text(&f, &common::localizer("ca")),
        "La reproducció encara no és compatible amb aquest sistema."
    );
    assert_eq!(
        failure_text(&ClipFailure::Download("offline".into()), &common::en()),
        "Download failed: offline"
    );
    assert_eq!(
        control_label(&ClipState::Paused, &common::localizer("es")),
        "Reanudar"
    );
}

fn playing(dir: &Path) -> (ClipController, Rc<RefCell<Log>>) {
    let (mut c, log) = controller(&clip(true, true), dir, Ok(()));
    c.click();
    c.downloaded();
    (c, log)
}

#[test]
fn seek_clamps_to_the_clip() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = playing(dir.path());
    c.seek_to(400);
    assert_eq!(c.position_ms(), 400);
    c.seek_by(-5_000);
    assert_eq!(c.position_ms(), 0, "never before the start");
    c.seek_by(5_000);
    assert_eq!(c.position_ms(), 1_000, "never past the end");
    c.seek_fraction(0.25);
    assert_eq!(c.position_ms(), 250);
    c.seek_fraction(7.0);
    assert_eq!(c.position_ms(), 1_000);
    c.seek_fraction(f32::NAN);
    assert_eq!(c.position_ms(), 0);
    assert!(calls(&log).contains(&"seek 250".to_string()));
}

#[test]
fn seek_does_nothing_without_a_player() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = controller(&clip(true, true), dir.path(), Ok(()));
    c.seek_to(500);
    c.seek_by(5_000);
    assert_eq!(c.position_ms(), 0);
    assert_eq!(c.state(), &ClipState::Poster);
    assert!(calls(&log).is_empty());
}

#[test]
fn seeking_keeps_a_paused_clip_paused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = playing(dir.path());
    c.click();
    assert_eq!(c.state(), &ClipState::Paused);
    log.borrow_mut().calls.clear();
    c.seek_by(500);
    assert_eq!(c.state(), &ClipState::Paused);
    assert_eq!(calls(&log), vec!["seek 500".to_string()]);
}

#[test]
fn speed_cycles_and_survives_resume() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = playing(dir.path());
    assert_eq!(SPEEDS, [1.0, 1.5, 2.0]);
    assert_eq!(c.speed(), 1.0);
    assert_eq!(c.cycle_speed(), 1.5);
    assert_eq!(calls(&log).last().unwrap(), "rate 1.5");
    assert_eq!(c.cycle_speed(), 2.0);
    assert_eq!(c.cycle_speed(), 1.0);
    c.cycle_speed();
    c.click();
    log.borrow_mut().calls.clear();
    c.click();
    assert_eq!(c.state(), &ClipState::Playing);
    // Resuming plays at the chosen speed.
    assert_eq!(
        calls(&log),
        vec!["play".to_string(), "rate 1.5".to_string()]
    );
}

#[test]
fn speed_volume_and_mute_chosen_before_playing_reach_the_player() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = controller(&clip(true, true), dir.path(), Ok(()));
    c.cycle_speed();
    c.set_volume(0.4);
    c.toggle_mute();
    assert!(calls(&log).is_empty(), "no player yet");
    c.click();
    c.downloaded();
    let got = calls(&log);
    for want in ["rate 1.5", "volume 0.4", "muted true", "play"] {
        assert!(got.contains(&want.to_string()), "{want} in {got:?}");
    }
}

#[test]
fn volume_clamps_and_mute_toggles() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = playing(dir.path());
    assert_eq!(c.volume(), 1.0);
    assert!(!c.muted());
    c.set_volume(1.7);
    assert_eq!(c.volume(), 1.0);
    c.set_volume(-1.0);
    assert_eq!(c.volume(), 0.0);
    c.set_volume(0.5);
    assert!(c.toggle_mute());
    assert!(c.muted());
    assert_eq!(calls(&log).last().unwrap(), "muted true");
    // Raising the volume unmutes.
    c.set_volume(0.6);
    assert!(!c.muted());
    assert!(calls(&log).contains(&"muted false".to_string()));
}

#[test]
fn keys_map_to_controls() {
    assert_eq!(ClipKey::from_key("space"), Some(ClipKey::PlayPause));
    assert_eq!(ClipKey::from_key("left"), Some(ClipKey::Back));
    assert_eq!(ClipKey::from_key("right"), Some(ClipKey::Forward));
    assert_eq!(ClipKey::from_key("m"), Some(ClipKey::Mute));
    assert_eq!(ClipKey::from_key("x"), None);
    let dir = tempfile::tempdir().unwrap();
    let (mut c, _) = playing(dir.path());
    c.seek_to(600);
    assert!(c.key(ClipKey::Back));
    assert_eq!(c.position_ms(), 0);
    assert!(c.key(ClipKey::Forward));
    assert_eq!(c.position_ms(), 1_000);
    assert!(c.key(ClipKey::Mute));
    assert!(c.muted());
    // Play and pause go through `click`, which may need a download.
    assert!(!c.key(ClipKey::PlayPause));
}

#[test]
fn captions_fetch_once_on_first_toggle_and_follow_the_position() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = playing(dir.path());
    assert!(c.has_captions());
    assert!(!c.captions_on());
    let Some(u) = c.toggle_captions() else {
        panic!("first toggle fetches the captions")
    };
    assert_eq!(&*u.0, "https://files.example/clip.vtt");
    assert!(c.captions_on());
    assert_eq!(c.caption(), None, "nothing until loaded");
    c.captions_loaded(components::vtt::parse(
        "WEBVTT\n\n00:00.000 --> 00:00.500\nFirst\n\n00:00.500 --> 00:01.000\nSecond\n",
    ));
    assert_eq!(c.caption().map(|s| s.as_ref()), Some("First"));
    log.borrow_mut().pos = 700;
    assert_eq!(c.caption().map(|s| s.as_ref()), Some("Second"));
    assert!(c.toggle_captions().is_none());
    assert!(!c.captions_on());
    assert_eq!(c.caption(), None);
    assert!(
        c.toggle_captions().is_none(),
        "loaded once, never refetched"
    );
    assert_eq!(c.caption().map(|s| s.as_ref()), Some("Second"));
}

#[test]
fn no_vtt_no_captions() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = clip(true, true);
    f.video.as_mut().unwrap().vtt = None;
    let (mut c, _) = controller(&f, dir.path(), Ok(()));
    assert!(!c.has_captions());
    assert!(c.toggle_captions().is_none());
    assert!(!c.captions_on());
}

#[test]
fn caption_fetch_failure_lets_the_user_retry() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, _) = playing(dir.path());
    assert!(c.toggle_captions().is_some());
    c.captions_failed();
    assert!(!c.captions_on());
    assert!(c.captions_error(), "a short error shows");
    assert!(c.toggle_captions().is_some(), "a failed fetch is retried");
    assert!(!c.captions_error());
}

#[test]
fn control_texts_read_in_the_active_language() {
    use components::clip::{speed_label, time_label};
    let en = common::en();
    assert_eq!(speed_label(1.0, &en), "1x");
    assert_eq!(speed_label(1.5, &en), "1.5x");
    assert_eq!(speed_label(2.0, &en), "2x");
    assert_eq!(speed_label(1.5, &common::localizer("es")), "1,5x");
    assert_eq!(speed_label(1.5, &common::localizer("ca")), "1,5x");
    assert_eq!(time_label(65_000, Some(125_000)), "1:05 / 2:05");
    assert_eq!(time_label(0, None), "0:00");
    let es = common::localizer("es");
    assert_eq!(components::clip::mute_label(true, &es), "Activar sonido");
    assert_eq!(
        components::clip::captions_label(&common::localizer("ca")),
        "Subtítols"
    );
}

#[test]
fn seeking_while_paused_refreshes_the_picture_then_stops() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = playing(dir.path());
    c.tick();
    c.click();
    assert_eq!(c.state(), &ClipState::Paused);
    assert!(!c.wants_frame(), "a paused clip asks for no frames");
    log.borrow_mut().calls.clear();
    assert!(c.tick().is_none(), "nor pulls any");
    assert!(calls(&log).is_empty());
    // The seek lands a few frames later.
    log.borrow_mut().no_frame = true;
    c.seek_to(500);
    assert!(c.wants_frame(), "the seek asks for a frame");
    c.tick();
    assert!(c.wants_frame(), "still waiting for the picture");
    log.borrow_mut().no_frame = false;
    log.borrow_mut().calls.clear();
    c.tick();
    assert_eq!(calls(&log), vec!["frame".to_string()]);
    assert!(!c.wants_frame(), "the new picture arrived: nothing more");
    assert_eq!(c.state(), &ClipState::Paused);
}

#[test]
fn a_seek_that_never_lands_stops_asking() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = playing(dir.path());
    c.click();
    log.borrow_mut().no_frame = true;
    c.seek_to(500);
    let mut asked = 0;
    while c.wants_frame() {
        c.tick();
        asked += 1;
        assert!(asked < 100, "bounded");
    }
    assert!(asked > 1);
}

#[test]
fn labels_are_built_once_until_what_they_show_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut c, log) = playing(dir.path());
    let en = common::en();
    assert_eq!(c.labels(&en).time, "0:00 / 0:01");
    log.borrow_mut().pos = 400;
    c.labels(&en);
    assert_eq!(c.label_builds(), 1, "same second: reused, not rebuilt");
    log.borrow_mut().pos = 1_000;
    assert_eq!(c.labels(&en).time, "0:01 / 0:01");
    assert_eq!(c.label_builds(), 2);
    c.cycle_speed();
    assert_eq!(c.labels(&en).speed, "1.5x");
    assert_eq!(c.labels(&en).speed_aria, "Playback speed, 1.5x");
    c.set_volume(0.7);
    assert_eq!(c.labels(&en).volume_value, "70%");
    assert_eq!(c.labels(&en).seek_value, "0:01 of 0:01");
    assert_eq!(c.labels(&common::localizer("es")).speed, "1,5x");
}
