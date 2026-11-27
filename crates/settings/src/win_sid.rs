//! Windows user SIDs from process tokens. Not compiled elsewhere.

use std::io;

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
    core::PWSTR,
};

fn err(e: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
}

/// Closes a handle on drop.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this guard and closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The user SID string (`S-1-5-...`) of a process token.
fn token_user_sid(process: HANDLE) -> io::Result<String> {
    let mut token = HANDLE::default();
    // SAFETY: `process` is a valid handle and `token` a valid out pointer.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(err)?;
    let token = Owned(token);

    // First call reports the buffer size (it fails with insufficient buffer).
    let mut len = 0u32;
    // SAFETY: a null buffer with length 0 only asks for the size.
    let _ = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut len) };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    // u64 elements keep the buffer aligned for TOKEN_USER.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` is at least `len` bytes.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )
    }
    .map_err(err)?;
    // SAFETY: the buffer now holds a TOKEN_USER whose SID points inside it.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut wide = PWSTR::null();
    // SAFETY: `sid` is valid for the life of `buf`; `wide` is LocalAlloc'd on success.
    unsafe { ConvertSidToStringSidW(sid, &mut wide) }.map_err(err)?;
    // SAFETY: `wide` is a NUL-terminated string from the call above, freed once.
    let text = unsafe { wide.to_string() };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(wide.0.cast())));
    }
    text.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The SID of the user running this process.
pub fn current_user_sid() -> io::Result<String> {
    // SAFETY: the pseudo handle of the current process needs no closing.
    token_user_sid(unsafe { GetCurrentProcess() })
}

/// The SID of the user running process `pid`. Fails when the process is gone
/// or belongs to someone this process may not inspect.
pub fn process_user_sid(pid: u32) -> io::Result<String> {
    // SAFETY: plain Win32 call; the handle is closed by the guard.
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.map_err(err)?;
    let h = Owned(h);
    token_user_sid(h.0)
}
