//! Slack files into neutral `FileRef`s, including clip renditions
//! (docs/providers/slack/web-client/files.md).

use std::sync::Arc;

use domain::{FileId, FileKind, FileRef, ProviderExt, RemoteUrl, Rendition, VideoInfo};
use slack_api::types::{File, Thumb};

fn url(s: &str) -> RemoteUrl {
    RemoteUrl(Arc::from(s))
}

fn rendition(t: &Thumb) -> Rendition {
    Rendition {
        url: url(&t.url),
        width: t.width,
        height: t.height,
    }
}

/// Clips are detected by `subtype`, never by `mimetype`/`filetype` (both can be empty).
fn kind_of(f: &File) -> FileKind {
    match f.subtype.as_deref() {
        Some("slack_video") => return FileKind::Video,
        Some("slack_audio") => return FileKind::Audio,
        _ => {}
    }
    let mime = f.mimetype.as_deref().unwrap_or("");
    let ft = f.filetype.as_deref().unwrap_or("");
    if f.mode.as_deref() == Some("snippet") || ft == "text" || ft == "snippet" {
        FileKind::Snippet
    } else if mime.starts_with("image/") {
        FileKind::Image
    } else if mime.starts_with("video/") {
        FileKind::Video
    } else if mime.starts_with("audio/") {
        FileKind::Audio
    } else if mime == "application/pdf" || ft == "pdf" {
        FileKind::Pdf
    } else {
        FileKind::Other
    }
}

pub fn map_file(f: &File) -> FileRef {
    let kind = kind_of(f);
    let mime = match (f.mimetype.as_deref().filter(|m| !m.is_empty()), kind) {
        (Some(m), _) => m,
        (None, FileKind::Video) => "video/mp4",
        (None, FileKind::Audio) => "audio/mp4",
        (None, _) => "application/octet-stream",
    };
    let mut thumbs: Vec<Rendition> = f.thumbs.iter().map(rendition).collect();
    if let Some(t) = &f.thumb_pdf {
        thumbs.push(rendition(t));
    }
    let video = (kind == FileKind::Video
        && (f.mp4.is_some() || f.mp4_low.is_some() || f.thumb_video.is_some()))
    .then(|| VideoInfo {
        mp4_low: f.mp4_low.as_deref().map(url),
        mp4: f.mp4.as_deref().map(url),
        poster: f.thumb_video.as_ref().map(rendition),
        duration_ms: f.duration_ms,
        vtt: f.vtt.as_deref().map(url),
    });
    let video = video.or_else(|| {
        (kind == FileKind::Video && f.duration_ms.is_some()).then_some(VideoInfo {
            mp4_low: None,
            mp4: None,
            poster: None,
            duration_ms: f.duration_ms,
            vtt: None,
        })
    });
    FileRef {
        id: FileId(Arc::from(f.id.as_str())),
        name: Arc::from(f.name.as_deref().unwrap_or("")),
        title: f.title.as_deref().map(Arc::from),
        mime: Arc::from(mime),
        size: f.size.unwrap_or(0),
        kind,
        thumbs: thumbs.into(),
        download: f
            .url_private_download
            .as_deref()
            .or(f.url_private.as_deref())
            .map(url),
        preview: f.preview.as_deref().map(Arc::from),
        video,
        tombstone: f.is_tombstone(),
        // `lines` is the whole file; without it the preview plus `lines_more` is.
        lines: f.lines.or_else(|| {
            let shown = f.preview.as_deref().map_or(0, |p| p.lines().count());
            f.lines_more.map(|m| m.saturating_add(shown as u32))
        }),
        ext: ProviderExt::none(),
    }
}
