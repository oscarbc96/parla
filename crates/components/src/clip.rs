//! Clip playback: a pure state machine ([`step`]) and a [`ClipController`]
//! that drives an OS [`VideoPlayer`].
//!
//! Playback only starts from a click, pauses when the row scrolls out of view
//! or the window goes inactive, and the decoder and its frames are dropped as
//! soon as the row leaves the timeline's working set.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use domain::{FileKind, FileRef, ImageRef, RemoteUrl};
use gpui::{
    AnyElement, App, Bounds, FocusHandle, IntoElement, KeyDownEvent, MouseButton, Pixels,
    SharedString, Stateful, Window, div, prelude::*, px, relative,
};
use i18n::{LanguageIdentifier, Localizer};
use themes::{Theme, metrics};

use crate::controls::{IconButtonSize, focus_style, icon_button_styled, on_activate_app};
use crate::files::{Card, PREVIEW_W, format_size, shell};
use crate::icon::IconName;
use crate::media::{MediaImage, key_hash};
use crate::platform::{ClipError, Frame, VideoPlayer};
use crate::vtt::{Captions, Cue};

/// Last millisecond window treated as the end of the clip.
const END_MARGIN_MS: u64 = 100;

/// Playback speeds the speed control cycles through.
pub const SPEEDS: [f32; 3] = [1.0, 1.5, 2.0];

/// How far Left and Right seek.
pub const SEEK_STEP_MS: u64 = 5_000;

/// A key the focused clip card handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipKey {
    /// Space.
    PlayPause,
    /// Left: back [`SEEK_STEP_MS`].
    Back,
    /// Right: forward [`SEEK_STEP_MS`].
    Forward,
    /// M.
    Mute,
}

impl ClipKey {
    /// The control for a GPUI key name, if any.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "space" => Some(Self::PlayPause),
            "left" => Some(Self::Back),
            "right" => Some(Self::Forward),
            "m" => Some(Self::Mute),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipState {
    /// Poster and a play button. Nothing is fetched or decoded.
    Poster,
    Downloading,
    /// On disk, opening the decoder.
    Ready,
    Playing,
    Paused,
    /// Poster plus "Open in browser"; the failure says why.
    Fallback(ClipFailure),
}

/// Why a clip fell back to the browser; shown through the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipFailure {
    /// The card has no URL or destination to play from.
    NoSource,
    /// The download failed; the source's own description.
    Download(String),
    /// The decoder could not open or play the file.
    Open(ClipError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipEvent {
    Click,
    Downloaded,
    DownloadFailed(String),
    Opened,
    OpenFailed(ClipError),
    ScrolledAway,
    WindowInactive,
    Ended,
}

/// Transition table. Only a click leaves `Poster`, so nothing autoplays.
pub fn step(state: ClipState, ev: ClipEvent) -> ClipState {
    use ClipEvent as E;
    use ClipState as S;
    match (state, ev) {
        (S::Fallback(w), _) => S::Fallback(w),
        (S::Poster, E::Click) => S::Downloading,
        (S::Downloading, E::Downloaded) => S::Ready,
        (S::Downloading | S::Ready, E::DownloadFailed(e)) => S::Fallback(ClipFailure::Download(e)),
        (S::Ready, E::Opened) => S::Playing,
        (S::Ready | S::Playing | S::Paused, E::OpenFailed(e)) => S::Fallback(ClipFailure::Open(e)),
        (S::Playing, E::Click) => S::Paused,
        (S::Paused, E::Click) => S::Playing,
        (S::Playing, E::ScrolledAway | E::WindowInactive | E::Ended) => S::Paused,
        (s, _) => s,
    }
}

/// Rendition to fetch: `mp4_low`, then `mp4`, then the plain download (audio clips).
pub fn pick_source(f: &FileRef) -> Option<&RemoteUrl> {
    f.video
        .as_ref()
        .and_then(|v| v.mp4_low.as_ref().or(v.mp4.as_ref()))
        .or(f.download.as_ref())
}

/// True for files that get the clip card.
pub fn is_clip(f: &FileRef) -> bool {
    matches!(f.kind, FileKind::Video | FileKind::Audio) && !f.tombstone && pick_source(f).is_some()
}

/// `<cache_dir>/media/clips/<hash of url>.mp4`.
pub fn cache_path(cache_dir: &Path, url: &RemoteUrl) -> PathBuf {
    cache_dir
        .join("media")
        .join("clips")
        .join(format!("{}.mp4", key_hash(&url.0)))
}

/// What the view must do after a click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipAction {
    None,
    /// Fetch `url` into `dest` through the provider, then call
    /// [`ClipController::downloaded`] or [`ClipController::download_failed`].
    Download {
        url: RemoteUrl,
        dest: PathBuf,
    },
    OpenInBrowser,
}

pub type PlayerFactory = Box<dyn Fn() -> Box<dyn VideoPlayer>>;

/// Frames a paused clip may wait for a seek to land before it gives up.
/// Bounded, so a decoder that never answers costs at most this many frames.
const SEEK_REFRESH_FRAMES: u8 = 30;

/// Step of the volume keys.
const VOLUME_STEP: f32 = 0.1;

/// A clip control the user worked, handed to the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClipControl {
    PlayPause,
    /// Seek to this fraction (0 to 1) of the clip.
    Seek(f32),
    /// Seek by this many milliseconds.
    SeekBy(i64),
    /// Next speed in [`SPEEDS`].
    Speed,
    Mute,
    /// Set the volume (0 to 1).
    Volume(f32),
    /// Change the volume by this much.
    VolumeBy(f32),
    Captions,
    /// A key on the focused card.
    Key(ClipKey),
}

/// Called when a control is worked; the view routes it to its controller.
/// Returns whether it did anything, so an unused key keeps propagating.
pub type OnClipControl = Rc<dyn Fn(ClipControl, &mut Window, &mut App) -> bool>;

/// Where a bar was drawn and whether a drag on it is in progress.
#[derive(Clone, Default)]
struct Bar {
    bounds: Rc<Cell<Bounds<Pixels>>>,
    dragging: Rc<Cell<bool>>,
}

#[derive(Clone, Default)]
struct Bars {
    seek: Bar,
    volume: Bar,
}

/// Texts of the open card, built when what they show changes (state, speed,
/// volume, the second shown, the language), never per frame.
#[derive(Clone, Default)]
pub struct ClipLabels {
    pub play: SharedString,
    pub time: SharedString,
    pub seek: SharedString,
    pub seek_value: SharedString,
    pub speed: SharedString,
    pub speed_aria: SharedString,
    pub mute: SharedString,
    pub volume: SharedString,
    pub volume_value: SharedString,
    pub captions: SharedString,
    pub captions_error: SharedString,
    pub size: SharedString,
}

#[derive(Clone, PartialEq)]
struct LabelKey {
    lang: LanguageIdentifier,
    playing: bool,
    speed: usize,
    muted: bool,
    volume_pct: u32,
    second: u64,
}

pub struct ClipController {
    state: ClipState,
    source: Option<RemoteUrl>,
    dest: Option<PathBuf>,
    duration_ms: Option<u64>,
    name: SharedString,
    poster: Option<ImageRef>,
    size: u64,
    factory: PlayerFactory,
    player: Option<Box<dyn VideoPlayer>>,
    frame: Option<Frame>,
    /// Frames left to pull a picture after a seek while paused.
    refresh_frames: u8,
    /// Index into [`SPEEDS`].
    speed: usize,
    volume: f32,
    muted: bool,
    /// Caption file, when the provider supplies one.
    vtt: Option<RemoteUrl>,
    captions_on: bool,
    captions_fetching: bool,
    captions_error: bool,
    /// Loaded once, on the first time captions are turned on; kept in memory only.
    captions: Option<Captions>,
    /// Cue shown at the last tick, to notice a change of caption height.
    shown_cue: Option<usize>,
    cue_changed: bool,
    bars: Bars,
    labels: Option<(LabelKey, ClipLabels)>,
    /// Times the labels were built (for tests).
    label_builds: usize,
    on: Option<OnClipControl>,
}

impl ClipController {
    pub fn new(f: &FileRef, cache_dir: &Path, factory: PlayerFactory) -> Self {
        let source = pick_source(f).cloned();
        let dest = source.as_ref().map(|u| cache_path(cache_dir, u));
        let poster = f
            .video
            .as_ref()
            .and_then(|v| v.poster.as_ref())
            .map(|r| ImageRef::Provider(r.url.clone()));
        Self {
            state: ClipState::Poster,
            source,
            dest,
            duration_ms: f.video.as_ref().and_then(|v| v.duration_ms),
            name: SharedString::from(f.name.to_string()),
            poster,
            size: f.size,
            factory,
            player: None,
            frame: None,
            refresh_frames: 0,
            speed: 0,
            volume: 1.0,
            muted: false,
            vtt: f.video.as_ref().and_then(|v| v.vtt.clone()),
            captions_on: false,
            captions_fetching: false,
            captions_error: false,
            captions: None,
            shown_cue: None,
            cue_changed: false,
            bars: Bars::default(),
            labels: None,
            label_builds: 0,
            on: None,
        }
    }

    /// The view's control handler, built once when the clip opens.
    pub fn set_on(&mut self, on: OnClipControl) {
        self.on = Some(on);
    }

    pub fn on(&self) -> Option<&OnClipControl> {
        self.on.as_ref()
    }

    /// Known length of the clip.
    pub fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// True while the decoder is open (playing or paused): the controls show.
    pub fn is_open(&self) -> bool {
        matches!(self.state, ClipState::Playing | ClipState::Paused) && self.player.is_some()
    }

    /// True when the next frame should be drawn: while playing, and for a few
    /// frames after a seek while paused so the picture follows. Otherwise
    /// nothing is requested.
    pub fn wants_frame(&self) -> bool {
        match self.state {
            ClipState::Playing => true,
            ClipState::Paused => self.refresh_frames > 0,
            _ => false,
        }
    }

    /// Plays from where the player is, at the chosen speed. Players start at
    /// normal speed, so the speed is set again after `play`.
    fn start(&mut self) {
        let rate = self.speed();
        if let Some(p) = &mut self.player {
            p.play();
            if rate != 1.0 {
                p.set_rate(rate);
            }
        }
    }

    /// Moves to `ms`, clamped to the clip. Only while a decoder is open;
    /// a paused clip stays paused and its picture follows.
    pub fn seek_to(&mut self, ms: u64) {
        if !matches!(self.state, ClipState::Playing | ClipState::Paused) {
            return;
        }
        let ms = self.duration_ms.map_or(ms, |d| ms.min(d));
        if let Some(p) = &mut self.player {
            p.seek(ms);
            if self.state == ClipState::Paused {
                self.refresh_frames = SEEK_REFRESH_FRAMES;
            }
        }
    }

    /// Moves by `delta_ms` from the current position, clamped to the clip.
    pub fn seek_by(&mut self, delta_ms: i64) {
        let at = self.position_ms();
        let to = if delta_ms < 0 {
            at.saturating_sub(delta_ms.unsigned_abs())
        } else {
            at.saturating_add(delta_ms as u64)
        };
        self.seek_to(to);
    }

    /// Moves to `fraction` (0 to 1) of the clip; needs a known duration.
    pub fn seek_fraction(&mut self, fraction: f32) {
        let Some(d) = self.duration_ms else { return };
        let f = if fraction.is_nan() {
            0.0
        } else {
            fraction.clamp(0.0, 1.0)
        };
        self.seek_to((d as f64 * f as f64).round() as u64);
    }

    /// Current speed, one of [`SPEEDS`].
    pub fn speed(&self) -> f32 {
        SPEEDS[self.speed]
    }

    /// Next speed in [`SPEEDS`], wrapping; returns it.
    pub fn cycle_speed(&mut self) -> f32 {
        self.speed = (self.speed + 1) % SPEEDS.len();
        let rate = self.speed();
        if let Some(p) = &mut self.player {
            p.set_rate(rate);
        }
        rate
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    /// Sets the volume (clamped to 0 to 1); a volume above zero unmutes.
    pub fn set_volume(&mut self, volume: f32) {
        if volume.is_nan() {
            return;
        }
        self.volume = volume.clamp(0.0, 1.0);
        let unmute = self.muted && self.volume > 0.0;
        if unmute {
            self.muted = false;
        }
        if let Some(p) = &mut self.player {
            p.set_volume(self.volume);
            if unmute {
                p.set_muted(false);
            }
        }
    }

    /// Mutes or unmutes; returns whether it is muted now.
    pub fn toggle_mute(&mut self) -> bool {
        self.muted = !self.muted;
        if let Some(p) = &mut self.player {
            p.set_muted(self.muted);
        }
        self.muted
    }

    /// Runs a key's control and returns whether it applied. Play and pause go
    /// through [`Self::click`] (it may need a download), so
    /// [`ClipKey::PlayPause`] returns false here.
    pub fn key(&mut self, key: ClipKey) -> bool {
        match key {
            ClipKey::PlayPause => false,
            ClipKey::Back | ClipKey::Forward => {
                if !self.is_open() {
                    return false;
                }
                let step = SEEK_STEP_MS as i64;
                self.seek_by(if key == ClipKey::Back { -step } else { step });
                true
            }
            ClipKey::Mute => {
                self.toggle_mute();
                true
            }
        }
    }

    /// True when the provider supplies captions for this clip.
    pub fn has_captions(&self) -> bool {
        self.vtt.is_some()
    }

    pub fn captions_on(&self) -> bool {
        self.captions_on
    }

    /// The last caption fetch failed; cleared by the next toggle.
    pub fn captions_error(&self) -> bool {
        self.captions_error
    }

    /// Turns captions on or off. The first time on returns the caption file to
    /// fetch (at most [`vtt::MAX_BYTES`]), then [`Self::captions_loaded`] or
    /// [`Self::captions_failed`].
    pub fn toggle_captions(&mut self) -> Option<RemoteUrl> {
        let vtt = self.vtt.clone()?;
        self.captions_error = false;
        self.captions_on = !self.captions_on;
        self.shown_cue = None;
        if !self.captions_on || self.captions.is_some() || self.captions_fetching {
            return None;
        }
        self.captions_fetching = true;
        Some(vtt)
    }

    pub fn captions_loaded(&mut self, cues: Vec<Cue>) {
        self.captions_fetching = false;
        self.captions = Some(Captions::new(cues));
    }

    /// The fetch failed or was too large: captions go off with a short error,
    /// and the next toggle fetches again.
    pub fn captions_failed(&mut self) {
        self.captions_fetching = false;
        self.captions_on = false;
        self.captions_error = true;
    }

    /// Caption text at the current position, while captions are on.
    pub fn caption(&self) -> Option<&SharedString> {
        if !self.captions_on {
            return None;
        }
        self.captions
            .as_ref()?
            .at(self.position_ms())
            .map(|c| &c.text)
    }

    /// True once after the caption shown changed (its height may differ).
    pub fn take_cue_changed(&mut self) -> bool {
        std::mem::take(&mut self.cue_changed)
    }

    pub fn state(&self) -> &ClipState {
        &self.state
    }

    pub fn last_frame(&self) -> Option<&Frame> {
        self.frame.as_ref()
    }

    pub fn position_ms(&self) -> u64 {
        self.player.as_ref().map_or(0, |p| p.position_ms())
    }

    /// The card's texts, rebuilt only when what they show changed.
    pub fn labels(&mut self, l: &Localizer) -> &ClipLabels {
        let position = self.position_ms();
        let key = LabelKey {
            lang: l.locale().clone(),
            playing: self.state == ClipState::Playing,
            speed: self.speed,
            muted: self.muted,
            volume_pct: (self.volume * 100.0).round() as u32,
            second: position / 1000,
        };
        if self.labels.as_ref().map(|(k, _)| k) != Some(&key) {
            let labels = self.build_labels(&key, position, l);
            self.labels = Some((key, labels));
            self.label_builds += 1;
        }
        &self.labels.as_ref().expect("built above").1
    }

    /// Times the labels were built so far (for tests).
    pub fn label_builds(&self) -> usize {
        self.label_builds
    }

    fn build_labels(&self, key: &LabelKey, position: u64, l: &Localizer) -> ClipLabels {
        let speed = speed_label(self.speed(), l);
        let total = self.duration_ms.map(format_time).unwrap_or_default();
        let percent = key.volume_pct.to_string();
        ClipLabels {
            play: control_label(&self.state, l).into(),
            time: time_label(position, self.duration_ms).into(),
            seek: t!(l, "clip-seek").into_owned().into(),
            seek_value: t!(
                l,
                "clip-seek-value",
                position = format_time(position),
                total = total
            )
            .into_owned()
            .into(),
            speed_aria: t!(l, "clip-speed-label", speed = speed.as_str())
                .into_owned()
                .into(),
            speed: speed.into(),
            mute: mute_label(self.muted, l).into(),
            volume: t!(l, "clip-volume").into_owned().into(),
            volume_value: t!(l, "clip-volume-value", percent = percent)
                .into_owned()
                .into(),
            captions: captions_label(l).into(),
            captions_error: t!(l, "clip-captions-error").into_owned().into(),
            size: format_size(self.size, l).into(),
        }
    }

    fn apply(&mut self, ev: ClipEvent) {
        self.state = step(std::mem::replace(&mut self.state, ClipState::Poster), ev);
    }

    pub fn click(&mut self) -> ClipAction {
        match self.state.clone() {
            ClipState::Poster => {
                let (Some(url), Some(dest)) = (self.source.clone(), self.dest.clone()) else {
                    self.state = ClipState::Fallback(ClipFailure::NoSource);
                    return ClipAction::None;
                };
                self.apply(ClipEvent::Click);
                if dest.is_file() {
                    self.downloaded();
                    ClipAction::None
                } else {
                    ClipAction::Download { url, dest }
                }
            }
            ClipState::Playing => {
                self.apply(ClipEvent::Click);
                if let Some(p) = &mut self.player {
                    p.pause();
                }
                ClipAction::None
            }
            ClipState::Paused => {
                self.apply(ClipEvent::Click);
                self.refresh_frames = 0;
                self.start();
                ClipAction::None
            }
            ClipState::Fallback(_) => ClipAction::OpenInBrowser,
            ClipState::Downloading | ClipState::Ready => ClipAction::None,
        }
    }

    /// The file is on disk: open the decoder and start playing.
    pub fn downloaded(&mut self) {
        if self.state != ClipState::Downloading {
            return;
        }
        self.apply(ClipEvent::Downloaded);
        let Some(dest) = self.dest.clone() else {
            return;
        };
        let mut p = (self.factory)();
        match p.open(&dest) {
            Ok(()) => {
                self.apply(ClipEvent::Opened);
                p.set_volume(self.volume);
                p.set_muted(self.muted);
                self.player = Some(p);
                self.start();
            }
            Err(e) => self.apply(ClipEvent::OpenFailed(e)),
        }
    }

    pub fn download_failed(&mut self, why: String) {
        self.apply(ClipEvent::DownloadFailed(why));
    }

    fn pause_if_paused(&mut self) {
        if self.state == ClipState::Paused
            && let Some(p) = &mut self.player
        {
            p.pause();
        }
    }

    pub fn scrolled_away(&mut self) {
        self.apply(ClipEvent::ScrolledAway);
        self.pause_if_paused();
    }

    pub fn window_inactive(&mut self) {
        self.apply(ClipEvent::WindowInactive);
        self.pause_if_paused();
    }

    /// Pulls the newest frame. Does nothing unless playing, or paused just
    /// after a seek (until its picture arrives), so a hidden or paused clip
    /// costs no decode work. Detects the end and decoder failure.
    pub fn tick(&mut self) -> Option<&Frame> {
        match self.state {
            ClipState::Playing => {}
            ClipState::Paused if self.refresh_frames > 0 => {
                let p = self.player.as_mut()?;
                match p.frame() {
                    Some(f) => {
                        self.frame = Some(f);
                        self.refresh_frames = 0;
                    }
                    None => self.refresh_frames -= 1,
                }
                self.note_cue();
                return self.frame.as_ref();
            }
            _ => return None,
        }
        let p = self.player.as_mut()?;
        if let Some(e) = p.failure() {
            self.apply(ClipEvent::OpenFailed(e));
            self.release_player();
            return None;
        }
        if let Some(f) = p.frame() {
            self.frame = Some(f);
        }
        let ended = self
            .duration_ms
            .is_some_and(|d| d > 0 && p.position_ms() + END_MARGIN_MS >= d);
        if ended {
            p.pause();
            p.seek(0);
            self.apply(ClipEvent::Ended);
        }
        self.note_cue();
        self.frame.as_ref()
    }

    fn note_cue(&mut self) {
        if !self.captions_on {
            return;
        }
        let now = self
            .captions
            .as_ref()
            .and_then(|c| c.index_at(self.position_ms()));
        if now != self.shown_cue {
            self.shown_cue = now;
            self.cue_changed = true;
        }
    }

    fn release_player(&mut self) {
        self.player = None;
        self.frame = None;
        self.refresh_frames = 0;
    }

    /// Drops the decoder and its frames; back to the poster (a fallback stays).
    pub fn release(&mut self) {
        self.release_player();
        if !matches!(self.state, ClipState::Fallback(_)) {
            self.state = ClipState::Poster;
        }
    }
}

/// `1x`, `1.5x` (`1,5x` in Spanish and Catalan), `2x`.
pub fn speed_label(rate: f32, l: &Localizer) -> String {
    let whole = rate.trunc() as u32;
    let tenths = (rate.fract() * 10.0).round() as u32;
    let fraction = if tenths == 0 {
        "whole".to_string()
    } else {
        tenths.to_string()
    };
    t!(
        l,
        "clip-speed-value",
        whole = whole.to_string(),
        fraction = fraction
    )
    .into_owned()
}

/// `0:12 / 1:05`, or the position alone when the length is unknown.
pub fn time_label(position_ms: u64, duration_ms: Option<u64>) -> String {
    match duration_ms {
        Some(d) => format!("{} / {}", format_time(position_ms), format_time(d)),
        None => format_time(position_ms),
    }
}

/// Text of the mute control: what a click does.
pub fn mute_label(muted: bool, l: &Localizer) -> String {
    if muted {
        t!(l, "clip-unmute")
    } else {
        t!(l, "clip-mute")
    }
    .into_owned()
}

/// Text of the captions control.
pub fn captions_label(l: &Localizer) -> String {
    t!(l, "clip-captions").into_owned()
}

/// `1:05` style clip time.
pub fn format_time(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Text of the play control for `state`.
pub fn control_label(state: &ClipState, l: &Localizer) -> String {
    match state {
        ClipState::Poster => t!(l, "clip-play"),
        ClipState::Downloading => t!(l, "clip-downloading"),
        ClipState::Ready => t!(l, "clip-opening"),
        ClipState::Playing => t!(l, "clip-pause"),
        ClipState::Paused => t!(l, "clip-resume"),
        ClipState::Fallback(_) => t!(l, "clip-open-in-browser"),
    }
    .into_owned()
}

/// Why `failure` happened, in the active language.
pub fn failure_text(failure: &ClipFailure, l: &Localizer) -> String {
    match failure {
        ClipFailure::NoSource => t!(l, "clip-error-no-source"),
        ClipFailure::Download(d) => t!(l, "clip-error-download", detail = d),
        ClipFailure::Open(ClipError::Unsupported) => t!(l, "clip-error-unsupported"),
        ClipFailure::Open(ClipError::FileMissing) => t!(l, "clip-error-file-missing"),
        ClipFailure::Open(ClipError::PathNotUtf8) => t!(l, "clip-error-path"),
        ClipFailure::Open(ClipError::WrongThread) => t!(l, "clip-error-thread"),
        ClipFailure::Open(ClipError::NoPlayableItem) => t!(l, "clip-error-no-item"),
        ClipFailure::Open(ClipError::DecoderFailed(None)) => t!(l, "clip-error-decoder"),
        ClipFailure::Open(ClipError::DecoderFailed(Some(d))) => {
            t!(l, "clip-error-decoder-detail", detail = d)
        }
    }
    .into_owned()
}

/// Draws a [`Card::Clip`] that has no open decoder (poster, downloading,
/// fallback). The caller adds the click handler. An open clip is drawn by
/// [`render_open_clip`].
pub fn render_clip(
    card: &Card,
    state: &ClipState,
    theme: &Theme,
    l: &Localizer,
    id: impl Into<gpui::ElementId>,
) -> Stateful<gpui::Div> {
    let colors = theme.colors();
    let Card::Clip {
        poster,
        name,
        size,
        duration_ms,
        ..
    } = card
    else {
        return shell(theme, id);
    };
    let picture = poster.as_ref().map(|t| {
        MediaImage::new(t.clone(), name.clone())
            .max_size(PREVIEW_W as f32, 240.0)
            .into_any_element()
    });
    let progress = duration_ms.map(format_time).unwrap_or_default();
    let mut el = shell(theme, id).children(picture).child(
        div()
            .flex()
            .gap_2()
            .child(div().child(name.clone()))
            .child(
                div()
                    .text_color(colors.text_muted)
                    .child(format_size(*size, l)),
            )
            .child(div().text_color(colors.text_muted).child(progress)),
    );
    el = el.child(
        div()
            .text_color(colors.text_accent)
            .child(control_label(state, l)),
    );
    if let ClipState::Fallback(why) = state {
        el = el.child(
            div()
                .text_color(colors.text_muted)
                .child(failure_text(why, l)),
        );
    }
    el
}

/// Draws an open clip (playing or paused) with its controls: play and pause,
/// seek bar, time, speed, mute and volume, and captions when the provider
/// supplies them. Its texts and handler come from the controller, so a frame
/// builds no strings. `None` when the clip is not open or has no handler.
pub fn render_open_clip(
    ctl: &mut ClipController,
    seek_focus: &FocusHandle,
    theme: &Theme,
    l: &Localizer,
    id: impl Into<gpui::ElementId>,
) -> Option<Stateful<gpui::Div>> {
    if !ctl.is_open() {
        return None;
    }
    let on = ctl.on.clone()?;
    ctl.labels(l);
    let ctl = &*ctl;
    let labels = &ctl.labels.as_ref()?.1;
    let colors = theme.colors();
    let picture: Option<AnyElement> = match ctl.last_frame() {
        #[cfg(target_os = "macos")]
        Some(Frame {
            width,
            height,
            data: crate::platform::FrameData::Surface(buf),
        }) if *width > 0 && *height > 0 => {
            let scale = (PREVIEW_W as f32 / *width as f32).min(240.0 / *height as f32);
            Some(
                gpui::surface(gpui_pixel_buffer(buf))
                    .w(px(*width as f32 * scale))
                    .h(px(*height as f32 * scale))
                    .into_any_element(),
            )
        }
        _ => ctl.poster.as_ref().map(|t| {
            MediaImage::new(t.clone(), ctl.name.as_ref())
                .max_size(PREVIEW_W as f32, 240.0)
                .into_any_element()
        }),
    };
    let caption = ctl
        .captions_on()
        .then(|| caption_box(ctl.caption().cloned(), theme));
    let error = ctl.captions_error().then(|| {
        div()
            .debug_selector(|| "clip-captions-error".into())
            .text_color(colors.text_muted)
            .child(labels.captions_error.clone())
    });
    Some(
        shell(theme, id)
            .children(picture)
            .children(caption)
            .child(controls(ctl, labels, &on, seek_focus, theme))
            .children(error)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().child(ctl.name.clone()))
                    .child(
                        div()
                            .text_color(colors.text_muted)
                            .child(labels.size.clone()),
                    ),
            ),
    )
}

/// The caption showing now, on the sunken ground; empty between cues.
fn caption_box(text: Option<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .debug_selector(|| "clip-caption".into())
        .min_h(metrics::SPACE_8)
        .px(metrics::SPACE_4)
        .py(metrics::SPACE_2)
        .rounded(metrics::RADIUS_XS)
        .bg(themes::parla_tokens(theme).bg_sunken)
        .text_color(theme.colors().text)
        .text_size(metrics::FONT_SIZE_SM)
        .text_center()
        .children(text)
}

/// Seek bar over a row of play and pause, time, speed, mute, volume and captions.
fn controls(
    ctl: &ClipController,
    labels: &ClipLabels,
    on: &OnClipControl,
    seek_focus: &FocusHandle,
    theme: &Theme,
) -> gpui::Div {
    let colors = theme.colors();
    let fraction = match ctl.duration_ms {
        Some(d) if d > 0 => (ctl.position_ms() as f32 / d as f32).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let seek = bar(
        BarKind::Seek,
        fraction,
        labels.seek.clone(),
        labels.seek_value.clone(),
        &ctl.bars.seek,
        Some(seek_focus),
        theme,
        on,
    );
    let volume = if ctl.muted() { 0.0 } else { ctl.volume() };
    let (play_icon, mute_icon) = (
        if ctl.state == ClipState::Playing {
            IconName::Pause
        } else {
            IconName::Play
        },
        if ctl.muted() {
            IconName::VolumeOff
        } else {
            IconName::Volume
        },
    );
    let row = div()
        .flex()
        .items_center()
        .gap_2()
        .child(icon_control(
            "clip-play-pause",
            play_icon,
            labels.play.clone(),
            None,
            theme,
            on,
            ClipControl::PlayPause,
        ))
        .child(
            div()
                .flex_1()
                .text_color(colors.text_muted)
                .child(labels.time.clone()),
        )
        .child(speed_control(labels, theme, on))
        .child(icon_control(
            "clip-mute",
            mute_icon,
            labels.mute.clone(),
            Some(ctl.muted()),
            theme,
            on,
            ClipControl::Mute,
        ))
        .child(bar(
            BarKind::Volume,
            volume,
            labels.volume.clone(),
            labels.volume_value.clone(),
            &ctl.bars.volume,
            None,
            theme,
            on,
        ))
        .children(ctl.has_captions().then(|| {
            icon_control(
                "clip-captions",
                IconName::Captions,
                labels.captions.clone(),
                Some(ctl.captions_on()),
                theme,
                on,
                ClipControl::Captions,
            )
        }));
    div().flex().flex_col().child(seek).child(row)
}

/// Stops a press on a control from moving keyboard focus (the composer keeps
/// it) and from reaching the card's own click.
fn keep_focus(el: Stateful<gpui::Div>) -> Stateful<gpui::Div> {
    el.on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
}

/// An icon button; `toggled` draws a toggle (muted, captions on) in the
/// accent so its state reads at a glance.
fn icon_control(
    id: &'static str,
    icon: IconName,
    label: SharedString,
    toggled: Option<bool>,
    theme: &Theme,
    on: &OnClipControl,
    control: ClipControl,
) -> Stateful<gpui::Div> {
    let colors = theme.colors();
    let pressed = toggled == Some(true);
    let on = on.clone();
    let el = icon_button_styled(
        id,
        icon,
        label.clone(),
        IconButtonSize::Small,
        Some(if pressed {
            colors.text_on_accent
        } else {
            colors.text_accent
        }),
        false,
        colors,
        move |window, cx| {
            cx.stop_propagation();
            on(control, window, cx);
        },
    )
    .debug_selector(move || id.into())
    .role(gpui::Role::Button)
    .aria_label(label)
    .when(pressed, |d| d.bg(colors.element_background))
    .when_some(toggled, |d, t| {
        d.aria_toggled(if t {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
    });
    keep_focus(el)
}

/// The speed as text (`1.5x`); a click steps to the next speed.
fn speed_control(labels: &ClipLabels, theme: &Theme, on: &OnClipControl) -> Stateful<gpui::Div> {
    let colors = theme.colors();
    let on = on.clone();
    let el = on_activate_app(
        div()
            .id("clip-speed")
            .debug_selector(|| "clip-speed".into())
            .role(gpui::Role::Button)
            .aria_label(labels.speed_aria.clone())
            .flex()
            .items_center()
            .min_h(metrics::LAYOUT_ICON_BUTTON_SM)
            .px(metrics::SPACE_3)
            .rounded(metrics::RADIUS_XS)
            .cursor_pointer()
            .text_color(colors.text_accent)
            .hover(|s| s.bg(colors.element_hover))
            .tab_stop(true)
            .focusable()
            .focus_visible(focus_style(colors))
            .child(labels.speed.clone()),
        move |window, cx| {
            cx.stop_propagation();
            on(ClipControl::Speed, window, cx);
        },
    );
    keep_focus(el)
}

#[derive(Clone, Copy, PartialEq)]
enum BarKind {
    Seek,
    Volume,
}

impl BarKind {
    fn id(self) -> &'static str {
        match self {
            BarKind::Seek => "clip-seek",
            BarKind::Volume => "clip-volume",
        }
    }

    fn at(self, fraction: f32) -> ClipControl {
        match self {
            BarKind::Seek => ClipControl::Seek(fraction),
            BarKind::Volume => ClipControl::Volume(fraction),
        }
    }

    /// Left, Right, Home and End on both; Up and Down on the volume.
    fn key(self, key: &str) -> Option<ClipControl> {
        let step = SEEK_STEP_MS as i64;
        Some(match (self, key) {
            (_, "home") => self.at(0.0),
            (_, "end") => self.at(1.0),
            (BarKind::Seek, "left") => ClipControl::SeekBy(-step),
            (BarKind::Seek, "right") => ClipControl::SeekBy(step),
            (BarKind::Volume, "left" | "down") => ClipControl::VolumeBy(-VOLUME_STEP),
            (BarKind::Volume, "right" | "up") => ClipControl::VolumeBy(VOLUME_STEP),
            _ => return None,
        })
    }
}

/// A slider: a thin track filled to `fraction` in a hit box at least an icon
/// button high. A press or a drag sets the fraction under the pointer; it is
/// a tab stop with the focus ring and keys.
#[allow(clippy::too_many_arguments)]
fn bar(
    kind: BarKind,
    fraction: f32,
    label: SharedString,
    value: SharedString,
    state: &Bar,
    focus: Option<&FocusHandle>,
    theme: &Theme,
    on: &OnClipControl,
) -> Stateful<gpui::Div> {
    let colors = theme.colors();
    let tokens = themes::parla_tokens(theme);
    let record = state.bounds.clone();
    let track = div()
        .relative()
        .w_full()
        .h(metrics::SPACE_2)
        .rounded(metrics::RADIUS_FULL)
        .bg(tokens.border_control)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(fraction))
                .rounded(metrics::RADIUS_FULL)
                .bg(colors.text_accent),
        )
        .child(
            gpui::canvas(move |b, _, _| record.set(b), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        );
    let fraction_at = {
        let bounds = state.bounds.clone();
        move |x: Pixels| {
            let b = bounds.get();
            (b.size.width > px(0.0)).then(|| (x - b.origin.x) / b.size.width)
        }
    };
    let (down_on, move_on, key_on) = (on.clone(), on.clone(), on.clone());
    let (down_at, move_at) = (fraction_at.clone(), fraction_at);
    let (drag_down, drag_move, drag_up, drag_out) = (
        state.dragging.clone(),
        state.dragging.clone(),
        state.dragging.clone(),
        state.dragging.clone(),
    );
    let id = kind.id();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Slider)
        .aria_label(label)
        .aria_value(value)
        .aria_numeric_value(f64::from(fraction))
        .aria_min_numeric_value(0.0)
        .aria_max_numeric_value(1.0)
        .flex()
        .items_center()
        .min_h(metrics::LAYOUT_ICON_BUTTON_SM)
        .px(metrics::SPACE_1)
        .rounded(metrics::RADIUS_XS)
        .cursor_pointer()
        .map(|d| match focus {
            Some(h) => d.track_focus(h),
            None => d.tab_stop(true).focusable(),
        })
        .focus_visible(focus_style(colors))
        .map(|d| match kind {
            BarKind::Volume => d.w(metrics::LAYOUT_CLIP_VOLUME).flex_none(),
            BarKind::Seek => d.w_full(),
        })
        .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
            if let Some(f) = down_at(ev.position.x) {
                drag_down.set(true);
                down_on(kind.at(f), window, cx);
            }
        })
        .on_mouse_move(move |ev, window, cx| {
            if !drag_move.get() {
                return;
            }
            if ev.pressed_button != Some(MouseButton::Left) {
                drag_move.set(false);
                return;
            }
            if let Some(f) = move_at(ev.position.x) {
                move_on(kind.at(f), window, cx);
            }
        })
        .on_mouse_up(MouseButton::Left, move |_, _, _| drag_up.set(false))
        .on_mouse_up_out(MouseButton::Left, move |_, _, _| drag_out.set(false))
        .on_key_down(move |ev: &KeyDownEvent, window, cx| {
            let m = &ev.keystroke.modifiers;
            if m.platform || m.control || m.alt || m.shift {
                return;
            }
            if let Some(c) = kind.key(&ev.keystroke.key)
                && key_on(c, window, cx)
            {
                cx.stop_propagation();
            }
        })
        .child(track)
}

/// Retains the decoder's pixel buffer for GPUI without copying its pixels.
#[cfg(target_os = "macos")]
fn gpui_pixel_buffer(
    buf: &objc2_core_foundation::CFRetained<objc2_core_video::CVPixelBuffer>,
) -> objc2_core_foundation::CFRetained<objc2_core_video::CVPixelBuffer> {
    buf.clone()
}
