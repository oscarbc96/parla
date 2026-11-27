//! File cards under a message: image previews, PDF thumbnails, snippets and
//! plain file rows. The view-model ([`card`]) is pure; [`render_card`] draws it.

use std::path::PathBuf;

use domain::{FileKind, FileRef, ImageRef};
use gpui::{AnyElement, IntoElement, SharedString, Stateful, div, prelude::*, px};
use i18n::Localizer;
use settings::InlineImages;
use themes::Theme;

use crate::media::MediaImage;

/// Preview width asked of the provider and the box an inline image fits into.
pub const PREVIEW_W: u32 = 360;

/// What a file looks like in the timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum Card {
    Tombstone,
    /// Image shown inline.
    Image {
        src: ImageRef,
        name: String,
        size: u64,
    },
    /// Image held back until clicked (`inline_images = on_click`).
    ImagePlaceholder {
        name: String,
        size: u64,
    },
    /// PDF with the provider's thumbnail (no local PDF rendering).
    Pdf {
        thumb: Option<ImageRef>,
        name: String,
        size: u64,
    },
    /// Code or text file: language, numbered lines, Expand and Copy ([`crate::snippet`]).
    Snippet(crate::snippet::SnippetCard),
    /// Video or audio clip: poster, name and a play control ([`crate::clip`]).
    Clip {
        poster: Option<ImageRef>,
        name: String,
        size: u64,
        duration_ms: Option<u64>,
        audio: bool,
    },
    /// Icon, name and size.
    Plain {
        name: String,
        size: u64,
    },
}

fn display_name(f: &FileRef) -> String {
    f.title
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or(&f.name)
        .to_string()
}

/// Builds the card of `f`. `revealed` is true once the user clicked a held-back image.
pub fn card(f: &FileRef, inline: InlineImages, revealed: bool) -> Card {
    card_with(f, inline, revealed, false)
}

/// [`card`] with a snippet shown expanded (`expanded`) or cut to its first lines.
pub fn card_with(f: &FileRef, inline: InlineImages, revealed: bool, expanded: bool) -> Card {
    if f.tombstone {
        return Card::Tombstone;
    }
    let name = display_name(f);
    let size = f.size;
    let thumb = || {
        f.best_thumb(PREVIEW_W)
            .map(|r| ImageRef::Provider(r.url.clone()))
    };
    if crate::clip::is_clip(f) {
        let poster = f
            .video
            .as_ref()
            .and_then(|v| v.poster.as_ref())
            .map(|r| ImageRef::Provider(r.url.clone()))
            .or_else(thumb);
        return Card::Clip {
            poster,
            name,
            size,
            duration_ms: f.video.as_ref().and_then(|v| v.duration_ms),
            audio: f.kind == FileKind::Audio,
        };
    }
    match f.kind {
        FileKind::Image => match (thumb(), inline) {
            (Some(src), InlineImages::Always) => Card::Image { src, name, size },
            (Some(src), InlineImages::OnClick) if revealed => Card::Image { src, name, size },
            (Some(_), InlineImages::OnClick) => Card::ImagePlaceholder { name, size },
            _ => Card::Plain { name, size },
        },
        FileKind::Pdf => Card::Pdf {
            thumb: thumb(),
            name,
            size,
        },
        FileKind::Snippet if f.preview.is_some() => {
            Card::Snippet(crate::snippet::SnippetCard::new(
                name,
                f.preview.as_deref().unwrap_or(""),
                f.lines,
                f.size,
                expanded,
            ))
        }
        _ => Card::Plain { name, size },
    }
}

/// `1.5 MB` style size.
pub fn format_size(bytes: u64, l: &Localizer) -> String {
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < 3 {
        v /= 1024.0;
        u += 1;
    }
    // One decimal, rounded here so the catalog only picks the unit and separators.
    let value = if u == 0 { v } else { (v * 10.0).round() / 10.0 };
    match u {
        0 => t!(l, "size-bytes", value = value),
        1 => t!(l, "size-kb", value = value),
        2 => t!(l, "size-mb", value = value),
        _ => t!(l, "size-gb", value = value),
    }
    .into_owned()
}

/// Outcome of a file action, shown as a toast under the timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum FileNotice {
    /// Saved here; the toast offers "Show in folder".
    Downloaded(PathBuf),
    /// Catalog text from any namespace, rendered in the active language.
    Error(i18n::LocalizedText),
}

/// Where a staged file is on its way out.
#[derive(Clone, Debug, PartialEq)]
pub enum StagedState {
    /// Attached to the draft; it is sent with the next Send.
    Ready,
    /// Being sent. `Some(fraction)` (0.0 to 1.0) when the provider reports bytes,
    /// `None` when it only says done or failed.
    Uploading(Option<f32>),
    /// The provider refused or the connection broke; Retry sends it again.
    Failed(SharedString),
    /// The user cancelled the upload; Retry sends it again.
    Cancelled,
}

/// A file attached to a composer draft (picked, dropped or pasted), shown as a chip.
#[derive(Clone, Debug, PartialEq)]
pub struct StagedFile {
    pub id: u64,
    pub name: SharedString,
    pub size: u64,
    /// The chip shows a thumbnail of the file itself (a decodable image) instead of a type badge.
    pub thumb: Option<PathBuf>,
    /// Set for a snippet created from a long paste: its chip shows the language and line count.
    pub snippet: Option<StagedSnippet>,
    pub state: StagedState,
}

/// What a staged snippet chip shows besides its name.
#[derive(Clone, Debug, PartialEq)]
pub struct StagedSnippet {
    pub language: SharedString,
    pub lines: usize,
}

/// The framed box every card is drawn in.
pub(crate) fn shell(theme: &Theme, id: impl Into<gpui::ElementId>) -> Stateful<gpui::Div> {
    let colors = theme.colors();
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap_1()
        .my_1()
        .p_2()
        .max_w(px(PREVIEW_W as f32) + themes::metrics::SPACE_6)
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .text_xs()
        .text_color(colors.text)
}

/// The type badge text: the extension in caps (`PDF`, `ZIP`), or none when the
/// name has no short extension.
pub fn type_badge_text(name: &str) -> Option<String> {
    let ext = std::path::Path::new(name).extension()?.to_str()?;
    (!ext.is_empty() && ext.chars().count() <= 4).then(|| ext.to_uppercase())
}

/// Draws a card. The caller adds the click handler.
pub fn render_card(
    c: &Card,
    theme: &Theme,
    l: &Localizer,
    id: impl Into<gpui::ElementId>,
    file_id: &SharedString,
    on_download: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> Stateful<gpui::Div> {
    let colors = theme.colors();
    let tokens = themes::parla_tokens(theme);
    let shell = shell(theme, id);
    let label = |name: &str, size: u64| -> AnyElement {
        div()
            .flex()
            .gap_2()
            .child(div().child(name.to_string()))
            .child(
                div()
                    .text_color(colors.text_muted)
                    .child(format_size(size, l)),
            )
            .into_any_element()
    };
    let badge = |text: String| -> AnyElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(themes::metrics::SPACE_7)
            .min_w(themes::metrics::SPACE_7)
            .px(themes::metrics::SPACE_1)
            .rounded(themes::metrics::RADIUS_SM)
            .bg(tokens.signal_soft)
            .text_color(tokens.signal_text)
            .text_size(themes::metrics::FONT_SIZE_3XS)
            .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
            .child(text)
            .into_any_element()
    };
    let download_label: SharedString = t!(l, "file-download").into_owned().into();
    // One download button per card, selected as `file-download-<file id>`.
    let download_row = |name: &str, size: u64| -> gpui::Div {
        let cb = on_download.clone();
        let lbl = download_label.clone();
        let id = file_id.clone();
        div()
            .flex()
            .items_center()
            .gap(themes::metrics::SPACE_2)
            .children(type_badge_text(name).map(badge))
            .child(label(name, size))
            .child(div().flex_1())
            .child(
                crate::controls::icon_button_sized(
                    "file-download",
                    crate::icon::IconName::Download,
                    lbl.clone(),
                    crate::controls::IconButtonSize::Small,
                    colors,
                    move |w, cx| {
                        cx.stop_propagation();
                        cb(w, cx);
                    },
                )
                .debug_selector(move || format!("file-download-{id}"))
                .role(gpui::Role::Button)
                .aria_label(lbl),
            )
    };
    match c {
        Card::Tombstone => shell
            .text_color(colors.text_muted)
            .child(t!(l, "file-deleted").into_owned()),
        Card::Image { src, name, .. } => shell.child(
            MediaImage::new(src.clone(), name.clone())
                .max_size(PREVIEW_W as f32, 240.0)
                .into_any_element(),
        ),
        Card::ImagePlaceholder { name, size } => shell.child(download_row(name, *size)).child(
            div()
                .text_color(colors.text_accent)
                .child(t!(l, "file-click-to-show").into_owned()),
        ),
        Card::Pdf { thumb, name, size } => shell
            .children(thumb.as_ref().map(|t| {
                MediaImage::new(t.clone(), name.clone())
                    .max_size(PREVIEW_W as f32, 240.0)
                    .into_any_element()
            }))
            .child(download_row(name, *size)),
        // Drawn by `snippet::render_snippet`, which needs the timeline's callbacks.
        Card::Snippet(s) => shell.child(div().text_color(colors.text_muted).child(s.name.clone())),
        Card::Clip {
            poster, name, size, ..
        } => shell
            .children(poster.as_ref().map(|t| {
                MediaImage::new(t.clone(), name.clone())
                    .max_size(PREVIEW_W as f32, 240.0)
                    .into_any_element()
            }))
            .child(label(name, *size))
            .child(
                div()
                    .text_color(colors.text_accent)
                    .child(t!(l, "clip-play").into_owned()),
            ),
        Card::Plain { name, size } => shell.child(download_row(name, *size)),
    }
}
