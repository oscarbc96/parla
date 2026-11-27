//! Marks a downloaded file as coming from the internet, so the system asks
//! before it runs or opens it: the `com.apple.quarantine` extended attribute
//! on macOS, the `Zone.Identifier` stream (zone 3, internet) on Windows.
//! Linux has no such mark.

use std::path::Path;

/// Marks `path` as downloaded from the internet at `now` (unix seconds, the
/// app clock).
pub fn mark(path: &Path, now: i64) -> std::io::Result<()> {
    imp::mark(path, now)
}

/// `path` carries the mark (tests).
pub fn is_marked(path: &Path) -> bool {
    imp::is_marked(path)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    use std::path::Path;

    const NAME: &[u8] = b"com.apple.quarantine\0";

    fn c_path(path: &Path) -> std::io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
    }

    pub fn mark(path: &Path, secs: i64) -> std::io::Result<()> {
        let secs = secs.max(0);
        // Flags 0083: downloaded, user-approval needed; then time and agent.
        let value = format!("0083;{secs:08x};Parla;");
        let p = c_path(path)?;
        // SAFETY: both strings are NUL-terminated and outlive the call; the
        // value pointer and length describe one valid buffer.
        let r = unsafe {
            libc::setxattr(
                p.as_ptr(),
                NAME.as_ptr().cast(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        if r == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    pub fn is_marked(path: &Path) -> bool {
        let Ok(p) = c_path(path) else { return false };
        // SAFETY: a size query (null buffer, length 0) on NUL-terminated strings.
        let n = unsafe {
            libc::getxattr(
                p.as_ptr(),
                NAME.as_ptr().cast(),
                std::ptr::null_mut(),
                0,
                0,
                0,
            )
        };
        n > 0
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::path::{Path, PathBuf};

    fn stream(path: &Path) -> PathBuf {
        let mut s = path.as_os_str().to_owned();
        s.push(":Zone.Identifier");
        PathBuf::from(s)
    }

    pub fn mark(path: &Path, _now: i64) -> std::io::Result<()> {
        std::fs::write(stream(path), "[ZoneTransfer]\r\nZoneId=3\r\n")
    }

    pub fn is_marked(path: &Path) -> bool {
        std::fs::read_to_string(stream(path)).is_ok_and(|s| s.contains("ZoneId=3"))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use std::path::Path;

    pub fn mark(_: &Path, _: i64) -> std::io::Result<()> {
        Ok(())
    }

    pub fn is_marked(_: &Path) -> bool {
        false
    }
}
