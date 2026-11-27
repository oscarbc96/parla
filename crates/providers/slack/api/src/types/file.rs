//! File objects as returned by `files.info` and inside messages.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumb {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// A file. Every field except `id` may be absent (stubs, tombstones, clips
/// with empty `mimetype`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(from = "RawFile")]
pub struct File {
    pub id: String,
    pub name: Option<String>,
    pub title: Option<String>,
    pub mimetype: Option<String>,
    pub filetype: Option<String>,
    /// `slack_video` / `slack_audio` mark clips.
    pub subtype: Option<String>,
    pub size: Option<u64>,
    pub mode: Option<String>,
    pub url_private: Option<String>,
    pub url_private_download: Option<String>,
    pub permalink: Option<String>,
    /// Every `thumb_<N>` with its `_w` / `_h`.
    pub thumbs: Vec<Thumb>,
    pub thumb_pdf: Option<Thumb>,
    pub thumb_video: Option<Thumb>,
    pub mp4: Option<String>,
    pub mp4_low: Option<String>,
    pub hls: Option<String>,
    pub duration_ms: Option<u64>,
    /// WebVTT captions of a clip.
    pub vtt: Option<String>,
    pub preview: Option<String>,
    /// Lines in the whole file (`lines`); the preview holds the first of them.
    pub lines: Option<u32>,
    /// Lines of the file after the preview (`lines_more`).
    pub lines_more: Option<u32>,
    pub original_w: Option<u32>,
    pub original_h: Option<u32>,
    /// `{public|private: {<channel>: [{ts}]}}`.
    pub shares: Option<Value>,
}

impl File {
    /// `mode == "tombstone"` marks a deleted file.
    pub fn is_tombstone(&self) -> bool {
        self.mode.as_deref() == Some("tombstone")
    }

    /// Timestamp of the message that shares this file in `channel`, if known.
    pub fn share_ts(&self, channel: &str) -> Option<String> {
        let shares = self.shares.as_ref()?.as_object()?;
        shares
            .values()
            .filter_map(|scope| scope.get(channel)?.as_array()?.first()?.get("ts")?.as_str())
            .map(str::to_string)
            .next()
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawFile {
    id: String,
    name: Option<String>,
    title: Option<String>,
    mimetype: Option<String>,
    filetype: Option<String>,
    subtype: Option<String>,
    size: Option<u64>,
    mode: Option<String>,
    url_private: Option<String>,
    url_private_download: Option<String>,
    permalink: Option<String>,
    thumb_pdf: Option<String>,
    thumb_video: Option<String>,
    mp4: Option<String>,
    mp4_low: Option<String>,
    hls: Option<String>,
    duration_ms: Option<u64>,
    vtt: Option<String>,
    preview: Option<String>,
    lines: Option<u32>,
    lines_more: Option<u32>,
    original_w: Option<u32>,
    original_h: Option<u32>,
    shares: Option<Value>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

fn num(rest: &BTreeMap<String, Value>, key: &str) -> Option<u32> {
    rest.get(key)?.as_u64().and_then(|n| u32::try_from(n).ok())
}

impl From<RawFile> for File {
    fn from(r: RawFile) -> File {
        let mut thumbs = Vec::new();
        for (k, v) in &r.rest {
            let Some(n) = k.strip_prefix("thumb_") else {
                continue;
            };
            let Ok(width) = n.parse::<u32>() else {
                continue;
            };
            let Some(url) = v.as_str() else { continue };
            thumbs.push(Thumb {
                url: url.to_string(),
                width: num(&r.rest, &format!("{k}_w")).unwrap_or(width),
                height: num(&r.rest, &format!("{k}_h")).unwrap_or(0),
            });
        }
        thumbs.sort_by_key(|t| t.width);
        let named = |url: &Option<String>, key: &str| {
            url.as_ref().map(|u| Thumb {
                url: u.clone(),
                width: num(&r.rest, &format!("{key}_w")).unwrap_or(0),
                height: num(&r.rest, &format!("{key}_h")).unwrap_or(0),
            })
        };
        let thumb_pdf = named(&r.thumb_pdf, "thumb_pdf");
        let thumb_video = named(&r.thumb_video, "thumb_video");
        File {
            id: r.id,
            name: r.name,
            title: r.title,
            mimetype: r.mimetype,
            filetype: r.filetype,
            subtype: r.subtype,
            size: r.size,
            mode: r.mode,
            url_private: r.url_private,
            url_private_download: r.url_private_download,
            permalink: r.permalink,
            thumbs,
            thumb_pdf,
            thumb_video,
            mp4: r.mp4,
            mp4_low: r.mp4_low,
            hls: r.hls,
            duration_ms: r.duration_ms,
            vtt: r.vtt,
            preview: r.preview,
            lines: r.lines,
            lines_more: r.lines_more,
            original_w: r.original_w,
            original_h: r.original_h,
            shares: r.shares,
        }
    }
}
