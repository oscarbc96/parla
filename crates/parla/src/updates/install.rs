//! Install a verified update package over the running macOS app bundle.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[cfg(target_os = "macos")]
use std::ffi::OsStr;

/// Runs external installer tools without a shell.
pub trait Runner: Send + Sync {
    /// Runs argv[0] with the rest as arguments; returns exit success, stdout, stderr.
    fn run(&self, argv: &[OsString]) -> io::Result<Output>;
}

pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, argv: &[OsString]) -> io::Result<Output> {
        let Some((program, args)) = argv.split_first() else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        Command::new(program).args(args).output()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotInstallable {
    NotABundle,
    Translocated,
    OnMountedVolume,
    ReadOnly,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    Attach,
    MissingApp,
    BadSignature,
    WrongBundleId,
    Copy,
    /// The new app could not be swapped in; the installed app is as it was.
    Swap,
    Unsupported,
}

pub const BUNDLE_ID: &str = "com.justparla.parla";

/// The .app bundle that contains `exe` if Parla can replace it.
///
/// The `writable` callback must check whether the current user can write to
/// the supplied path. Both the bundle and its parent must be writable: the
/// new app is staged next to the bundle and swapped in.
#[cfg(target_os = "macos")]
pub fn running_bundle(
    exe: &Path,
    writable: &dyn Fn(&Path) -> bool,
) -> Result<PathBuf, NotInstallable> {
    let macos = exe.parent().ok_or(NotInstallable::NotABundle)?;
    if macos.file_name() != Some(OsStr::new("MacOS")) {
        return Err(NotInstallable::NotABundle);
    }
    let contents = macos.parent().ok_or(NotInstallable::NotABundle)?;
    if contents.file_name() != Some(OsStr::new("Contents")) {
        return Err(NotInstallable::NotABundle);
    }
    let bundle = contents.parent().ok_or(NotInstallable::NotABundle)?;
    if bundle.extension() != Some(OsStr::new("app")) {
        return Err(NotInstallable::NotABundle);
    }
    if exe
        .components()
        .any(|component| component.as_os_str() == OsStr::new("AppTranslocation"))
    {
        return Err(NotInstallable::Translocated);
    }
    if bundle.starts_with("/Volumes") {
        return Err(NotInstallable::OnMountedVolume);
    }
    let parent = bundle.parent().ok_or(NotInstallable::NotABundle)?;
    if !writable(bundle) || !writable(parent) {
        return Err(NotInstallable::ReadOnly);
    }
    Ok(bundle.to_path_buf())
}

#[cfg(not(target_os = "macos"))]
pub fn running_bundle(
    exe: &Path,
    writable: &dyn Fn(&Path) -> bool,
) -> Result<PathBuf, NotInstallable> {
    let _ = (exe, writable);
    Err(NotInstallable::Unsupported)
}

/// Installs an already-verified disk image over `bundle`.
///
/// The installed app is never edited in place. The new app is copied next to
/// it (`Parla.app.new`, same volume), checked with `codesign`, and only then
/// swapped with the installed one in a single atomic exchange (`renamex_np`
/// with `RENAME_SWAP`; two renames with a restore where the filesystem lacks
/// it). Any failure before the swap leaves the installed app untouched and
/// removes the staging copy, so quitting or running out of disk space during
/// the install cannot leave a half-written bundle.
///
/// The image is always detached after a successful attach. A failed detach is
/// logged and does not fail the install. The new app's designated requirement
/// is deliberately not compared with the running app's requirement.
#[cfg(target_os = "macos")]
pub fn install(
    dmg: &Path,
    bundle: &Path,
    mount_dir: &Path,
    runner: &dyn Runner,
) -> Result<(), InstallError> {
    let attach = vec![
        OsString::from("/usr/bin/hdiutil"),
        OsString::from("attach"),
        OsString::from("-nobrowse"),
        OsString::from("-readonly"),
        OsString::from("-noautoopen"),
        OsString::from("-mountpoint"),
        mount_dir.as_os_str().to_owned(),
        dmg.as_os_str().to_owned(),
    ];
    if !succeeded(runner.run(&attach)) {
        return Err(InstallError::Attach);
    }

    let staged = sibling(bundle, "new");
    let result = stage_from_mount(&staged, mount_dir, runner);
    if !detach(mount_dir, runner) {
        crate::log::warn("update: the disk image did not detach");
    }
    let result = result.and_then(|()| verify(&staged, runner));
    match result {
        Ok(()) => swap(&staged, bundle).inspect_err(|_| remove_dir(&staged)),
        Err(error) => {
            remove_dir(&staged);
            Err(error)
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install(
    dmg: &Path,
    bundle: &Path,
    mount_dir: &Path,
    runner: &dyn Runner,
) -> Result<(), InstallError> {
    let _ = (dmg, bundle, mount_dir, runner);
    Err(InstallError::Unsupported)
}

/// `<bundle>.<suffix>` next to the bundle, for example `Parla.app.new`.
#[cfg(target_os = "macos")]
fn sibling(bundle: &Path, suffix: &str) -> PathBuf {
    let mut name = bundle.as_os_str().to_owned();
    name.push(".");
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(target_os = "macos")]
fn remove_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// Checks the mounted app, then copies it to `staged`.
#[cfg(target_os = "macos")]
fn stage_from_mount(
    staged: &Path,
    mount_dir: &Path,
    runner: &dyn Runner,
) -> Result<(), InstallError> {
    let mounted_app = mount_dir.join("Parla.app");
    if !mounted_app.is_dir() {
        return Err(InstallError::MissingApp);
    }
    verify(&mounted_app, runner)?;

    let identifier = vec![
        OsString::from("/usr/libexec/PlistBuddy"),
        OsString::from("-c"),
        OsString::from("Print :CFBundleIdentifier"),
        mounted_app
            .join("Contents/Info.plist")
            .as_os_str()
            .to_owned(),
    ];
    let valid_identifier = runner.run(&identifier).is_ok_and(|output| {
        output.status.success()
            && std::str::from_utf8(&output.stdout).ok().map(str::trim) == Some(BUNDLE_ID)
    });
    if !valid_identifier {
        return Err(InstallError::WrongBundleId);
    }

    // A leftover from an interrupted attempt must not seed this copy.
    remove_dir(staged);
    let copy = vec![
        OsString::from("/usr/bin/rsync"),
        OsString::from("-a"),
        OsString::from("--delete"),
        directory_argument(&mounted_app),
        directory_argument(staged),
    ];
    if !succeeded(runner.run(&copy)) {
        return Err(InstallError::Copy);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn verify(app: &Path, runner: &dyn Runner) -> Result<(), InstallError> {
    let verify = vec![
        OsString::from("/usr/bin/codesign"),
        OsString::from("--verify"),
        OsString::from("--deep"),
        OsString::from("--strict"),
        app.as_os_str().to_owned(),
    ];
    if succeeded(runner.run(&verify)) {
        Ok(())
    } else {
        Err(InstallError::BadSignature)
    }
}

/// Puts `staged` in place of `bundle`. On success `staged` is gone and the
/// old app is removed; on failure `bundle` is as it was.
#[cfg(target_os = "macos")]
pub fn swap(staged: &Path, bundle: &Path) -> Result<(), InstallError> {
    if exchange(staged, bundle).is_ok() {
        // `staged` now holds the previous app.
        remove_dir(staged);
        return Ok(());
    }
    // No atomic exchange here: move the old app aside, then the new one in.
    let old = sibling(bundle, "old");
    remove_dir(&old);
    std::fs::rename(bundle, &old).map_err(|_| InstallError::Swap)?;
    if std::fs::rename(staged, bundle).is_err() {
        let _ = std::fs::rename(&old, bundle);
        return Err(InstallError::Swap);
    }
    remove_dir(&old);
    Ok(())
}

/// Atomically exchanges two paths (`renamex_np` with `RENAME_SWAP`).
#[cfg(target_os = "macos")]
fn exchange(a: &Path, b: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    let c = |p: &Path| {
        std::ffi::CString::new(p.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    };
    let (a, b) = (c(a)?, c(b)?);
    // SAFETY: both arguments are valid NUL-terminated paths for the call.
    let rc = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn detach(mount_dir: &Path, runner: &dyn Runner) -> bool {
    let detach = vec![
        OsString::from("/usr/bin/hdiutil"),
        OsString::from("detach"),
        mount_dir.as_os_str().to_owned(),
    ];
    if succeeded(runner.run(&detach)) {
        return true;
    }

    let force = vec![
        OsString::from("/usr/bin/hdiutil"),
        OsString::from("detach"),
        OsString::from("-force"),
        mount_dir.as_os_str().to_owned(),
    ];
    succeeded(runner.run(&force))
}

#[cfg(target_os = "macos")]
fn directory_argument(path: &Path) -> OsString {
    let mut argument = path.as_os_str().to_owned();
    argument.push("/");
    argument
}

#[cfg(target_os = "macos")]
fn succeeded(result: io::Result<Output>) -> bool {
    result.is_ok_and(|output| output.status.success())
}
