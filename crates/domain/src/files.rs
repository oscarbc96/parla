use std::path::PathBuf;
use std::sync::Arc;

use crate::ext::ProviderExt;
use crate::ids::{FileId, RemoteUrl};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Image,
    Video,
    Audio,
    Pdf,
    Snippet,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendition {
    pub url: RemoteUrl,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoInfo {
    pub mp4_low: Option<RemoteUrl>,
    pub mp4: Option<RemoteUrl>,
    pub poster: Option<Rendition>,
    pub duration_ms: Option<u64>,
    /// WebVTT captions, when the provider supplies them.
    pub vtt: Option<RemoteUrl>,
}

#[derive(Clone, Debug)]
pub struct FileRef {
    pub id: FileId,
    pub name: Arc<str>,
    pub title: Option<Arc<str>>,
    pub mime: Arc<str>,
    pub size: u64,
    pub kind: FileKind,
    pub thumbs: Arc<[Rendition]>,
    pub download: Option<RemoteUrl>,
    pub preview: Option<Arc<str>>,
    /// Lines in the whole file, when the provider says (a text file or snippet). The
    /// preview may hold fewer.
    pub lines: Option<u32>,
    pub video: Option<VideoInfo>,
    pub tombstone: bool,
    pub ext: ProviderExt,
}

/// Marks `file` deleted in `files`; true when any of them held it.
pub fn tombstone(files: &mut Arc<[FileRef]>, file: &FileId) -> bool {
    if !files.iter().any(|f| f.id == *file) {
        return false;
    }
    let mut out = files.to_vec();
    for f in &mut out {
        if f.id == *file {
            f.tombstone = true;
        }
    }
    *files = out.into();
    true
}

impl FileRef {
    /// Smallest rendition at least `max_w` wide, else the largest.
    pub fn best_thumb(&self, max_w: u32) -> Option<&Rendition> {
        self.thumbs
            .iter()
            .filter(|r| r.width >= max_w)
            .min_by_key(|r| r.width)
            .or_else(|| self.thumbs.iter().max_by_key(|r| r.width))
    }
}

/// A local file queued for upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upload {
    pub path: PathBuf,
    pub name: Arc<str>,
    pub mime: Arc<str>,
    pub size: u64,
    /// Syntax hint of a code snippet (`rust`, `python`, ...): the provider shows the
    /// file as a snippet of that language. `None` for ordinary files.
    pub filetype: Option<Arc<str>>,
}
