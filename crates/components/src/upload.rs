//! Upload preparation: size limit, file metadata and pasted-image temp files.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use domain::Upload;
use i18n::Localizer;

/// Files above this size are refused (1 GB).
pub const MAX_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024;

/// Paths split into uploads to start and the ones refused.
#[derive(Debug, Default)]
pub struct Planned {
    pub accepted: Vec<Upload>,
    pub refused: Vec<Refusal>,
}

/// Why a path was not uploaded; rendered through the catalog by [`refusals_text`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotAFile(String),
    TooLarge(String),
    Unreadable { name: String, error: String },
}

impl Refusal {
    pub fn text(&self, l: &Localizer) -> String {
        match self {
            Self::NotAFile(name) => t!(l, "upload-not-a-file", name = name),
            Self::TooLarge(name) => t!(l, "upload-too-large", name = name),
            Self::Unreadable { name, error } => {
                t!(l, "upload-unreadable", name = name, error = error)
            }
        }
        .into_owned()
    }
}

/// All `refused` reasons on one line.
pub fn refusals_text(refused: &[Refusal], l: &Localizer) -> String {
    refused
        .iter()
        .map(|r| r.text(l))
        .collect::<Vec<_>>()
        .join("; ")
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Small extension table; everything else is `application/octet-stream`.
pub fn mime_for(p: &Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log") => "text/plain",
        Some("json") => "application/json",
        Some("mp4") => "video/mp4",
        Some("mp3") => "audio/mpeg",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Stats each path: files within the limit are accepted, the rest refused with a reason.
pub fn plan(paths: &[PathBuf]) -> Planned {
    let mut out = Planned::default();
    for p in paths {
        let name = name_of(p);
        match std::fs::metadata(p) {
            Ok(m) if !m.is_file() => out.refused.push(Refusal::NotAFile(name)),
            Ok(m) if m.len() > MAX_UPLOAD_BYTES => out.refused.push(Refusal::TooLarge(name)),
            Ok(m) => out.accepted.push(Upload {
                path: p.clone(),
                name: Arc::from(name),
                mime: Arc::from(mime_for(p)),
                size: m.len(),
                filetype: None,
            }),
            Err(e) => out.refused.push(Refusal::Unreadable {
                name,
                error: e.to_string(),
            }),
        }
    }
    out
}

/// `<cache_dir>/parla/uploads`, where pasted images wait for their upload.
pub fn uploads_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("parla")
        .join("uploads")
}

/// Writes a pasted image to `<dir>/<uuid>.png` (re-encoding other formats).
pub fn write_pasted_image(dir: &Path, img: &gpui::Image) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    if img.format == gpui::ImageFormat::Png {
        std::fs::write(&path, &img.bytes)?;
    } else {
        let decoded = image::load_from_memory(&img.bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        decoded
            .save_with_format(&path, image::ImageFormat::Png)
            .map_err(io::Error::other)?;
    }
    Ok(path)
}

/// Writes snippet text to `<dir>/<uuid>.<ext>`, where it waits for its upload.
pub fn write_snippet(dir: &Path, text: &str, ext: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    // The extension comes from the language table, never from typed text.
    let ext: String = ext.chars().filter(char::is_ascii_alphanumeric).collect();
    let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Removes the paths that live directly in `dir` (the pasted-image temp files).
pub fn cleanup_temp(paths: &[PathBuf], dir: &Path) {
    for p in paths {
        if p.parent() == Some(dir) {
            let _ = std::fs::remove_file(p);
        }
    }
}
