//! Loading theme-family JSON (built-in or user) into the registry.

use std::path::{Path, PathBuf};

use crate::model::{ThemeKind, ThemeRegistry};
use crate::theme_json;

/// A theme file that could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeLoadError {
    /// The offending file.
    pub path: PathBuf,
    /// Human-readable reason.
    pub message: String,
}

/// Parses one theme-family file and inserts its themes (replacing same-named ones).
/// Nothing is registered when the file cannot be parsed.
pub(crate) fn load_family(registry: &ThemeRegistry, bytes: &[u8]) -> anyhow::Result<()> {
    load_family_as(registry, bytes, ThemeKind::User)
}

/// [`load_family`], marking each theme as `kind`.
pub(crate) fn load_family_as(
    registry: &ThemeRegistry,
    bytes: &[u8],
    kind: ThemeKind,
) -> anyhow::Result<()> {
    for theme in theme_json::parse_family(bytes)? {
        registry.insert(theme.with_kind(kind));
    }
    Ok(())
}

/// Loads every `*.json` theme file in `dir` into `registry`, in file-name order.
/// A bad file is reported with its path and skipped; a missing directory yields no errors.
/// A user theme with a built-in's name replaces it.
pub fn load_user_themes(registry: &ThemeRegistry, dir: &Path) -> Vec<ThemeLoadError> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut errors = Vec::new();
    for path in paths {
        let result = std::fs::read(&path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| load_family(registry, &bytes));
        if let Err(err) = result {
            errors.push(ThemeLoadError {
                path,
                message: format!("{err:#}"),
            });
        }
    }
    errors
}
