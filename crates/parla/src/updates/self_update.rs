//! What the app needs to install an update itself: a verified download, the
//! running bundle and the install step. Only macOS builds that carry the
//! update public key and run from a writable `Parla.app` have one.

use std::path::{Path, PathBuf};

use super::download::Downloader;
use super::install::{self, InstallError};
use super::package;

/// Installs a verified disk image over the running bundle.
pub type InstallFn = dyn Fn(&Path) -> Result<(), InstallError> + Send + Sync;

pub struct SelfUpdate {
    pub downloader: Downloader,
    /// The base64 minisign public key the download is checked against.
    pub public_key: String,
    /// The `Parla.app` that `App::restart` relaunches.
    pub bundle: PathBuf,
    pub install: Box<InstallFn>,
}

impl SelfUpdate {
    /// `None` unless this build can update itself: macOS, the public key
    /// compiled in, and a writable, non-translocated `Parla.app`.
    pub fn detect(current: &semver::Version) -> Option<Self> {
        let public_key = package::update_public_key()?.to_string();
        let exe = std::env::current_exe().ok()?;
        let bundle = install::running_bundle(&exe, &writable).ok()?;
        let dir = settings::paths::cache_dir().join("updates");
        let target = bundle.clone();
        let cache = dir.clone();
        Some(Self {
            downloader: Downloader::new(dir, current),
            public_key,
            bundle,
            install: Box::new(move |dmg| {
                // A new mount point per attempt: a retry never reuses a folder
                // an earlier, failed attempt may still have mounted.
                clean_stale_mounts(&cache);
                let mount = unique_mount_dir(&cache);
                let _ = std::fs::create_dir_all(&mount);
                let result = install::install(dmg, &target, &mount, &install::SystemRunner);
                let _ = std::fs::remove_dir(&mount);
                let _ = std::fs::remove_file(dmg);
                result
            }),
        })
    }
}

/// `<cache>/mount-<pid>-<n>`, not used by any earlier attempt of this run.
fn unique_mount_dir(cache: &Path) -> PathBuf {
    static ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    cache.join(format!("mount-{}-{n}", std::process::id()))
}

/// Removes empty `mount-*` folders earlier runs left behind. A folder that is
/// still a mount point is busy and stays.
fn clean_stale_mounts(cache: &Path) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with("mount-") {
            let _ = std::fs::remove_dir(entry.path());
        }
    }
}

#[cfg(target_os = "macos")]
fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path for the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(target_os = "macos"))]
fn writable(_path: &Path) -> bool {
    false
}
