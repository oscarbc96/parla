//! The media viewer's model: the files it steps through (a [`Gallery`] of the
//! images in a conversation's loaded window), and the zoom and pan of the one
//! on screen ([`ViewState`]). Pure and provider-neutral; the app draws the
//! window and [`crate::media::ViewerCache`] decodes the images.

use domain::{FileId, FileKind, FileRef, ImageRef, MessageId, MessageWindow, UserId};
use gpui::SharedString;
use jiff::tz::TimeZone;
use settings::Clock;

use crate::media::MAX_IMAGE_BYTES;

/// Zoom steps (`1.0` is actual size: one image pixel per logical pixel).
pub const ZOOM_STEPS: [f32; 10] = [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0];

/// Smallest and largest zoom.
pub const ZOOM_MIN: f32 = ZOOM_STEPS[0];
pub const ZOOM_MAX: f32 = ZOOM_STEPS[ZOOM_STEPS.len() - 1];

const EPS: f32 = 0.001;

/// Image types the viewer decodes from the original file.
fn decodable(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// The file opens in the viewer: an image (with something to show) or a PDF.
pub fn viewable(f: &FileRef) -> bool {
    if f.tombstone || crate::clip::is_clip(f) {
        return false;
    }
    match f.kind {
        FileKind::Image => source(f).is_some(),
        FileKind::Pdf => true,
        _ => false,
    }
}

/// What the viewer shows of image `f`: the original when it decodes and fits
/// the fetch limit, else the provider's largest rendition. `None` for a PDF or
/// an image with neither.
pub fn source(f: &FileRef) -> Option<ImageRef> {
    if f.kind != FileKind::Image || f.tombstone {
        return None;
    }
    if decodable(&f.mime)
        && f.size <= MAX_IMAGE_BYTES
        && let Some(u) = &f.download
    {
        return Some(ImageRef::Provider(u.clone()));
    }
    f.thumbs
        .iter()
        .max_by_key(|r| r.width)
        .map(|r| ImageRef::Provider(r.url.clone()))
}

/// One file of a gallery and the message it came with.
#[derive(Clone, Debug)]
pub struct ViewerItem {
    pub file: FileRef,
    pub message: MessageId,
    pub sender: SharedString,
}

/// The files the viewer steps through, in timeline order, and the one shown.
#[derive(Clone, Debug)]
pub struct Gallery {
    items: Vec<ViewerItem>,
    index: usize,
    /// Conversation name ("design-system"), for "3 of 8 in ...".
    pub place: Option<SharedString>,
    pub clock: Clock,
    pub tz: TimeZone,
}

impl Gallery {
    /// A gallery of `items` showing `index` (clamped).
    pub fn new(items: Vec<ViewerItem>, index: usize) -> Self {
        let index = index.min(items.len().saturating_sub(1));
        Self {
            items,
            index,
            place: None,
            clock: Clock::H24,
            tz: TimeZone::UTC,
        }
    }

    /// The gallery opened by clicking `clicked` on message `msg`: every image
    /// of the loaded `window`, oldest first. A PDF opens on its own (images
    /// step only between images). `sender` names an author.
    pub fn from_window(
        window: &MessageWindow,
        msg: &MessageId,
        clicked: &FileId,
        sender: impl Fn(Option<&UserId>) -> SharedString,
    ) -> Option<Self> {
        let mut items = Vec::new();
        let mut at = None;
        for m in window.messages() {
            for f in m.files.iter() {
                let hit = &m.id == msg && &f.id == clicked;
                if hit && f.kind == FileKind::Pdf && viewable(f) {
                    let item = ViewerItem {
                        file: f.clone(),
                        message: m.id.clone(),
                        sender: sender(m.author.as_ref()),
                    };
                    return Some(Self::new(vec![item], 0));
                }
                if f.kind != FileKind::Image || !viewable(f) {
                    continue;
                }
                if hit {
                    at = Some(items.len());
                }
                items.push(ViewerItem {
                    file: f.clone(),
                    message: m.id.clone(),
                    sender: sender(m.author.as_ref()),
                });
            }
        }
        at.map(|i| Self::new(items, i))
    }

    pub fn with_place(mut self, place: Option<SharedString>) -> Self {
        self.place = place;
        self
    }

    pub fn with_time(mut self, clock: Clock, tz: TimeZone) -> Self {
        self.clock = clock;
        self.tz = tz;
        self
    }

    pub fn items(&self) -> &[ViewerItem] {
        &self.items
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn current(&self) -> Option<&ViewerItem> {
        self.items.get(self.index)
    }

    pub fn has_prev(&self) -> bool {
        self.index > 0
    }

    pub fn has_next(&self) -> bool {
        self.index + 1 < self.items.len()
    }

    /// Steps to the next file; false at the end (it does not wrap).
    pub fn step_next(&mut self) -> bool {
        if !self.has_next() {
            return false;
        }
        self.index += 1;
        true
    }

    /// Steps to the previous file; false at the start.
    pub fn step_prev(&mut self) -> bool {
        if !self.has_prev() {
            return false;
        }
        self.index -= 1;
        true
    }

    /// Shows file `i` (out of range is ignored).
    pub fn select(&mut self, i: usize) -> bool {
        if i >= self.items.len() || i == self.index {
            return false;
        }
        self.index = i;
        true
    }

    /// Keeps only the files `keep` accepts (a message was deleted). The file
    /// shown stays if kept, else its next neighbour (or the last) shows.
    /// Returns whether anything was removed.
    pub fn retain(&mut self, keep: impl Fn(&ViewerItem) -> bool) -> bool {
        let before = self.items.len();
        let mut removed_before = 0;
        let mut i = 0;
        self.items.retain(|it| {
            let k = keep(it);
            if !k && i < self.index {
                removed_before += 1;
            }
            i += 1;
            k
        });
        if self.items.len() == before {
            return false;
        }
        self.index = (self.index - removed_before).min(self.items.len().saturating_sub(1));
        true
    }

    /// Image sources of the current file and its next neighbour, the only ones
    /// the viewer keeps decoded.
    pub fn wanted(&self) -> Vec<ImageRef> {
        [Some(self.index), self.has_next().then_some(self.index + 1)]
            .into_iter()
            .flatten()
            .filter_map(|i| self.items.get(i).and_then(|it| source(&it.file)))
            .collect()
    }
}

/// How the current image is sized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// The whole image fits the stage (never enlarged).
    Fit,
    /// A fixed scale; `1.0` is actual size.
    Scale(f32),
}

/// Zoom and pan of the image on screen. Sizes are logical pixels: `img` is the
/// image's natural size, `stage` the area it is drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewState {
    pub zoom: Zoom,
    /// Offset of the image centre from the stage centre.
    pub pan: (f32, f32),
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            zoom: Zoom::Fit,
            pan: (0.0, 0.0),
        }
    }
}

/// Scale that fits `img` into `stage`, at most actual size.
pub fn fit_scale(img: (f32, f32), stage: (f32, f32)) -> f32 {
    if img.0 <= 0.0 || img.1 <= 0.0 || stage.0 <= 0.0 || stage.1 <= 0.0 {
        return 1.0;
    }
    (stage.0 / img.0).min(stage.1 / img.1).min(1.0)
}

impl ViewState {
    /// The scale in force.
    pub fn scale(&self, img: (f32, f32), stage: (f32, f32)) -> f32 {
        match self.zoom {
            Zoom::Fit => fit_scale(img, stage),
            Zoom::Scale(s) => s,
        }
    }

    /// The scale as a whole percentage, for the zoom label.
    pub fn percent(&self, img: (f32, f32), stage: (f32, f32)) -> u32 {
        (self.scale(img, stage) * 100.0).round() as u32
    }

    /// The next zoom step up.
    pub fn zoom_in(&mut self, img: (f32, f32), stage: (f32, f32)) {
        let cur = self.scale(img, stage);
        let next = ZOOM_STEPS
            .iter()
            .copied()
            .find(|s| *s > cur + EPS)
            .unwrap_or(ZOOM_MAX.max(cur));
        self.set(Zoom::Scale(next), img, stage);
    }

    /// The next zoom step down.
    pub fn zoom_out(&mut self, img: (f32, f32), stage: (f32, f32)) {
        let cur = self.scale(img, stage);
        let next = ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|s| *s < cur - EPS)
            .unwrap_or(ZOOM_MIN.min(cur));
        self.set(Zoom::Scale(next), img, stage);
    }

    /// One image pixel per logical pixel.
    pub fn actual_size(&mut self, img: (f32, f32), stage: (f32, f32)) {
        self.set(Zoom::Scale(1.0), img, stage);
    }

    /// Back to fit-to-window, centred.
    pub fn fit(&mut self) {
        *self = Self::default();
    }

    fn set(&mut self, zoom: Zoom, img: (f32, f32), stage: (f32, f32)) {
        self.zoom = zoom;
        self.pan = self.clamped(self.pan, img, stage);
    }

    /// How far the image may move from the centre on each axis: half of what
    /// overflows the stage (0 when it fits).
    pub fn pan_limit(&self, img: (f32, f32), stage: (f32, f32)) -> (f32, f32) {
        let s = self.scale(img, stage);
        (
            ((img.0 * s - stage.0) / 2.0).max(0.0),
            ((img.1 * s - stage.1) / 2.0).max(0.0),
        )
    }

    /// The image is larger than the stage, so it can be dragged.
    pub fn can_pan(&self, img: (f32, f32), stage: (f32, f32)) -> bool {
        let (x, y) = self.pan_limit(img, stage);
        x > 0.0 || y > 0.0
    }

    fn clamped(&self, pan: (f32, f32), img: (f32, f32), stage: (f32, f32)) -> (f32, f32) {
        let (lx, ly) = self.pan_limit(img, stage);
        (pan.0.clamp(-lx, lx), pan.1.clamp(-ly, ly))
    }

    /// Moves the image by `(dx, dy)`, kept inside its limits.
    pub fn pan_by(&mut self, d: (f32, f32), img: (f32, f32), stage: (f32, f32)) {
        self.pan = self.clamped((self.pan.0 + d.0, self.pan.1 + d.1), img, stage);
    }

    /// Keeps the pan inside its limits for `stage` (the window was resized).
    pub fn clamp(&mut self, img: (f32, f32), stage: (f32, f32)) {
        self.pan = self.clamped(self.pan, img, stage);
    }

    /// The image's drawn size and its top-left corner inside the stage. A pan
    /// left over from a larger image or smaller stage is clamped here.
    pub fn layout(&self, img: (f32, f32), stage: (f32, f32)) -> ((f32, f32), (f32, f32)) {
        let s = self.scale(img, stage);
        let (w, h) = (img.0 * s, img.1 * s);
        let pan = self.clamped(self.pan, img, stage);
        (
            (w, h),
            ((stage.0 - w) / 2.0 + pan.0, (stage.1 - h) / 2.0 + pan.1),
        )
    }
}
