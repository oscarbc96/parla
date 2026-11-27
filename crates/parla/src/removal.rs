//! Deleting local data: one workspace's files, or everything Parla stores.
//! The config directory is never touched.

use std::io;
use std::path::{Path, PathBuf};

use settings::WorkspaceKey;

/// The directories "Delete all data" empties. The config dir is not among them.
#[derive(Clone, Debug)]
pub struct AppDirs {
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl gpui::Global for AppDirs {}

impl AppDirs {
    /// The platform (or `PARLA_DATA_DIR` / `PARLA_CACHE_DIR`) locations.
    pub fn discover() -> Self {
        Self {
            data: settings::paths::data_dir(),
            cache: settings::paths::cache_dir(),
        }
    }
}

/// Why [`remove_dir_all_checked`] refused a path (shown in the log only).
#[derive(Debug)]
enum Unsafe {
    Relative,
    FilesystemRoot,
    HomeDir,
}

/// Removes a directory tree, refusing paths that cannot be Parla's own: empty,
/// relative, a filesystem root or the home directory. A missing path is fine.
pub fn remove_dir_all_checked(path: &Path) -> io::Result<()> {
    let refuse = |why: Unsafe| {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{why:?}"),
        ))
    };
    if !path.is_absolute() {
        return refuse(Unsafe::Relative);
    }
    if path.parent().is_none() {
        return refuse(Unsafe::FilesystemRoot);
    }
    if dirs::home_dir().is_some_and(|h| h == path) {
        return refuse(Unsafe::HomeDir);
    }
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Deletes `<cache>/media/<provider>/<id>/`.
pub fn remove_workspace_media(cache_root: &Path, ws: &WorkspaceKey) -> io::Result<()> {
    remove_dir_all_checked(&crate::media::workspace_media_dir(cache_root, ws))
}
