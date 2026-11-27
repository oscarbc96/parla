//! Private serde mirrors of the domain types stored as JSON BLOB columns.
//! `ProviderExt` goes through the workspace's `ExtCodec`.

use std::sync::Arc;

use domain::{
    Broadcast, EmojiRef, ExtCodec, ExtKind, FileId, FileKind, FileRef, Mention, OutboxState,
    ProviderExt, Reaction, RemoteUrl, Rendition, UserId, VideoInfo,
};
use serde::{Deserialize, Serialize};
use store::StoreError;

use crate::err::other;

fn arc(s: String) -> Arc<str> {
    Arc::from(s)
}

#[derive(Serialize, Deserialize)]
enum BroadcastRow {
    Here,
    Channel,
    Everyone,
}

#[derive(Serialize, Deserialize)]
enum MentionRow {
    User(String),
    Group(String),
    Broadcast(BroadcastRow),
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct EmojiRow {
    shortcode: String,
    unicode: Option<String>,
    skin_tone: Option<u8>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct ReactionRow {
    emoji: EmojiRow,
    count: u32,
    users: Vec<String>,
    me: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct RenditionRow {
    url: String,
    width: u32,
    height: u32,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct VideoRow {
    mp4_low: Option<String>,
    mp4: Option<String>,
    poster: Option<RenditionRow>,
    duration_ms: Option<u64>,
    vtt: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
enum FileKindRow {
    Image,
    Video,
    Audio,
    Pdf,
    Snippet,
    #[default]
    Other,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct FileRow {
    id: String,
    name: String,
    title: Option<String>,
    mime: String,
    size: u64,
    kind: FileKindRow,
    thumbs: Vec<RenditionRow>,
    download: Option<String>,
    preview: Option<String>,
    video: Option<VideoRow>,
    tombstone: bool,
    #[serde(default)]
    lines: Option<u32>,
    ext: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
enum OutboxStateRow {
    Queued,
    Sending,
    Failed(String),
}

fn rendition_to(r: &Rendition) -> RenditionRow {
    RenditionRow {
        url: r.url.0.to_string(),
        width: r.width,
        height: r.height,
    }
}

fn rendition_from(r: RenditionRow) -> Rendition {
    Rendition {
        url: RemoteUrl(arc(r.url)),
        width: r.width,
        height: r.height,
    }
}

fn emoji_to(e: &EmojiRef) -> EmojiRow {
    EmojiRow {
        shortcode: e.shortcode.to_string(),
        unicode: e.unicode.as_ref().map(|s| s.to_string()),
        skin_tone: e.skin_tone,
    }
}

fn emoji_from(e: EmojiRow) -> EmojiRef {
    EmojiRef {
        shortcode: arc(e.shortcode),
        unicode: e.unicode.map(arc),
        skin_tone: e.skin_tone,
    }
}

fn to_blob<T: Serialize>(rows: &[T]) -> Result<Option<Vec<u8>>, StoreError> {
    if rows.is_empty() {
        return Ok(None);
    }
    serde_json::to_vec(rows).map(Some).map_err(other)
}

fn from_blob<T: for<'de> Deserialize<'de>>(b: Option<Vec<u8>>) -> Result<Vec<T>, StoreError> {
    match b {
        Some(b) if !b.is_empty() => serde_json::from_slice(&b).map_err(other),
        _ => Ok(Vec::new()),
    }
}

pub(crate) fn encode_mentions(m: &[Mention]) -> Result<Option<Vec<u8>>, StoreError> {
    let rows: Vec<MentionRow> = m
        .iter()
        .map(|m| match m {
            Mention::User(u) => MentionRow::User(u.0.to_string()),
            Mention::Group(g) => MentionRow::Group(g.to_string()),
            Mention::Broadcast(b) => MentionRow::Broadcast(match b {
                Broadcast::Here => BroadcastRow::Here,
                Broadcast::Channel => BroadcastRow::Channel,
                Broadcast::Everyone => BroadcastRow::Everyone,
            }),
        })
        .collect();
    to_blob(&rows)
}

pub(crate) fn decode_mentions(b: Option<Vec<u8>>) -> Result<Arc<[Mention]>, StoreError> {
    Ok(from_blob::<MentionRow>(b)?
        .into_iter()
        .map(|m| match m {
            MentionRow::User(u) => Mention::User(UserId(arc(u))),
            MentionRow::Group(g) => Mention::Group(arc(g)),
            MentionRow::Broadcast(b) => Mention::Broadcast(match b {
                BroadcastRow::Here => Broadcast::Here,
                BroadcastRow::Channel => Broadcast::Channel,
                BroadcastRow::Everyone => Broadcast::Everyone,
            }),
        })
        .collect())
}

pub(crate) fn encode_users(u: &[UserId]) -> Result<Option<Vec<u8>>, StoreError> {
    let rows: Vec<String> = u.iter().map(|u| u.0.to_string()).collect();
    to_blob(&rows)
}

pub(crate) fn decode_users(b: Option<Vec<u8>>) -> Result<Arc<[UserId]>, StoreError> {
    Ok(from_blob::<String>(b)?
        .into_iter()
        .map(|s| UserId(arc(s)))
        .collect())
}

pub(crate) fn encode_reactions(r: &[Reaction]) -> Result<Option<Vec<u8>>, StoreError> {
    let rows: Vec<ReactionRow> = r
        .iter()
        .map(|r| ReactionRow {
            emoji: emoji_to(&r.emoji),
            count: r.count,
            users: r.users.iter().map(|u| u.0.to_string()).collect(),
            me: r.me,
        })
        .collect();
    to_blob(&rows)
}

pub(crate) fn decode_reactions(b: Option<Vec<u8>>) -> Result<Arc<[Reaction]>, StoreError> {
    Ok(from_blob::<ReactionRow>(b)?
        .into_iter()
        .map(|r| Reaction {
            emoji: emoji_from(r.emoji),
            count: r.count,
            users: r.users.into_iter().map(|u| UserId(arc(u))).collect(),
            me: r.me,
        })
        .collect())
}

pub(crate) fn encode_files(
    files: &[FileRef],
    codec: &Arc<dyn ExtCodec>,
) -> Result<Option<Vec<u8>>, StoreError> {
    let rows: Vec<FileRow> = files
        .iter()
        .map(|f| FileRow {
            id: f.id.0.to_string(),
            name: f.name.to_string(),
            title: f.title.as_ref().map(|s| s.to_string()),
            mime: f.mime.to_string(),
            size: f.size,
            kind: match f.kind {
                FileKind::Image => FileKindRow::Image,
                FileKind::Video => FileKindRow::Video,
                FileKind::Audio => FileKindRow::Audio,
                FileKind::Pdf => FileKindRow::Pdf,
                FileKind::Snippet => FileKindRow::Snippet,
                FileKind::Other => FileKindRow::Other,
            },
            thumbs: f.thumbs.iter().map(rendition_to).collect(),
            download: f.download.as_ref().map(|u| u.0.to_string()),
            preview: f.preview.as_ref().map(|s| s.to_string()),
            video: f.video.as_ref().map(|v| VideoRow {
                mp4_low: v.mp4_low.as_ref().map(|u| u.0.to_string()),
                mp4: v.mp4.as_ref().map(|u| u.0.to_string()),
                poster: v.poster.as_ref().map(rendition_to),
                duration_ms: v.duration_ms,
                vtt: v.vtt.as_ref().map(|u| u.0.to_string()),
            }),
            tombstone: f.tombstone,
            lines: f.lines,
            ext: codec.encode(ExtKind::File, &f.ext),
        })
        .collect();
    to_blob(&rows)
}

pub(crate) fn decode_files(
    b: Option<Vec<u8>>,
    codec: &Arc<dyn ExtCodec>,
) -> Result<Arc<[FileRef]>, StoreError> {
    Ok(from_blob::<FileRow>(b)?
        .into_iter()
        .map(|f| FileRef {
            id: FileId(arc(f.id)),
            name: arc(f.name),
            title: f.title.map(arc),
            mime: arc(f.mime),
            size: f.size,
            kind: match f.kind {
                FileKindRow::Image => FileKind::Image,
                FileKindRow::Video => FileKind::Video,
                FileKindRow::Audio => FileKind::Audio,
                FileKindRow::Pdf => FileKind::Pdf,
                FileKindRow::Snippet => FileKind::Snippet,
                FileKindRow::Other => FileKind::Other,
            },
            thumbs: f.thumbs.into_iter().map(rendition_from).collect(),
            download: f.download.map(|u| RemoteUrl(arc(u))),
            preview: f.preview.map(arc),
            video: f.video.map(|v| VideoInfo {
                mp4_low: v.mp4_low.map(|u| RemoteUrl(arc(u))),
                mp4: v.mp4.map(|u| RemoteUrl(arc(u))),
                poster: v.poster.map(rendition_from),
                duration_ms: v.duration_ms,
                vtt: v.vtt.map(|u| RemoteUrl(arc(u))),
            }),
            tombstone: f.tombstone,
            lines: f.lines,
            ext: f
                .ext
                .map_or_else(ProviderExt::none, |b| codec.decode(ExtKind::File, &b)),
        })
        .collect())
}

pub(crate) fn encode_outbox_state(s: &OutboxState) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(&match s {
        OutboxState::Queued => OutboxStateRow::Queued,
        OutboxState::Sending => OutboxStateRow::Sending,
        OutboxState::Failed(e) => OutboxStateRow::Failed(e.clone()),
    })
    .map_err(other)
}

pub(crate) fn decode_outbox_state(b: Option<Vec<u8>>) -> Result<OutboxState, StoreError> {
    let Some(b) = b else {
        return Ok(OutboxState::Queued);
    };
    Ok(
        match serde_json::from_slice::<OutboxStateRow>(&b).map_err(other)? {
            OutboxStateRow::Queued => OutboxState::Queued,
            OutboxStateRow::Sending => OutboxState::Sending,
            OutboxStateRow::Failed(e) => OutboxState::Failed(e),
        },
    )
}

/// `ext` column: the codec's bytes, or NULL.
pub(crate) fn encode_ext(
    codec: &Arc<dyn ExtCodec>,
    kind: ExtKind,
    ext: &ProviderExt,
) -> Option<Vec<u8>> {
    codec.encode(kind, ext)
}

/// `p:<url>` for a provider URL, `w:<url>` for a web one.
pub(crate) fn encode_avatar(a: &domain::ImageRef) -> String {
    match a {
        domain::ImageRef::Provider(u) => format!("p:{}", u.0),
        domain::ImageRef::Web(u) => format!("w:{u}"),
    }
}

/// The inverse of [`encode_avatar`]; `None` for anything else.
pub(crate) fn decode_avatar(s: &str) -> Option<domain::ImageRef> {
    if let Some(u) = s.strip_prefix("p:") {
        Some(domain::ImageRef::Provider(domain::RemoteUrl(Arc::from(u))))
    } else {
        s.strip_prefix("w:")
            .map(|u| domain::ImageRef::Web(Arc::from(u)))
    }
}

pub(crate) fn decode_ext(
    codec: &Arc<dyn ExtCodec>,
    kind: ExtKind,
    b: Option<Vec<u8>>,
) -> ProviderExt {
    b.map_or_else(ProviderExt::none, |b| codec.decode(kind, &b))
}

#[cfg(test)]
mod avatar_tests {
    use super::*;
    use domain::{ImageRef, RemoteUrl};

    #[test]
    fn avatars_round_trip() {
        let p = ImageRef::Provider(RemoteUrl("https://x.test/a.png".into()));
        let w = ImageRef::Web("https://y.test/b.png".into());
        assert_eq!(decode_avatar(&encode_avatar(&p)), Some(p));
        assert_eq!(decode_avatar(&encode_avatar(&w)), Some(w));
    }

    #[test]
    fn a_malformed_stored_avatar_decodes_to_none() {
        for bad in [
            "",
            "x",
            "p",
            "w",
            "q:https://x.test/a.png",
            "https://x.test/a.png",
        ] {
            assert_eq!(decode_avatar(bad), None, "{bad:?}");
        }
    }
}
