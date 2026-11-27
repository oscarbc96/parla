//! Free space on the volume that holds the store, for the "Disk nearly full"
//! banner.

use std::path::Path;

/// Minimum gap between two reads, over the whole store: five minutes.
pub const FREE_SPACE_CHECK_MS: i64 = 5 * 60 * 1000;

/// Where free space is read from. The system in the app, a fixed value in tests.
pub trait FreeSpace: Send + Sync {
    /// Bytes a normal user can still write on the volume of `dir`, or `None`
    /// when it cannot be read. A directory that does not exist yet reads its
    /// nearest parent.
    fn free_bytes(&self, dir: &Path) -> Option<u64>;
}

/// `statvfs` (`f_bavail * f_frsize`) on Unix, `GetDiskFreeSpaceExW` on
/// Windows. On APFS, purgeable space (local snapshots, caches the system may
/// clear) is not counted as free.
pub struct SystemFreeSpace;

fn existing(dir: &Path) -> Option<&Path> {
    dir.ancestors().find(|p| p.exists())
}

impl FreeSpace for SystemFreeSpace {
    #[cfg(unix)]
    fn free_bytes(&self, dir: &Path) -> Option<u64> {
        use std::os::unix::ffi::OsStrExt as _;
        let path = std::ffi::CString::new(existing(dir)?.as_os_str().as_bytes()).ok()?;
        let mut st = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
        // SAFETY: `path` is a valid C string and `st` a valid out buffer.
        let ok = unsafe { libc::statvfs(path.as_ptr(), st.as_mut_ptr()) } == 0;
        // SAFETY: statvfs filled the struct when it returned 0.
        let st = ok.then(|| unsafe { st.assume_init() })?;
        Some((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }

    #[cfg(windows)]
    fn free_bytes(&self, dir: &Path) -> Option<u64> {
        use std::os::windows::ffi::OsStrExt as _;
        let wide: Vec<u16> = existing(dir)?
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut avail = 0u64;
        // SAFETY: `wide` is NUL terminated and `avail` outlives the call.
        unsafe {
            windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                windows::core::PCWSTR(wide.as_ptr()),
                Some(&mut avail),
                None,
                None,
            )
        }
        .ok()?;
        Some(avail)
    }

    #[cfg(not(any(unix, windows)))]
    fn free_bytes(&self, _: &Path) -> Option<u64> {
        None
    }
}
