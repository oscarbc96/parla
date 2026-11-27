//! Launch at login.
//!
//! The `launch_at_login` setting is mirrored to the OS login item at start and
//! whenever settings reload. The system is only written when its state differs,
//! and a failure is logged rather than stopping the app.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use gpui::{App, Global};
use settings::SettingsStore;

/// The OS login item, behind a seam so tests never touch the real one.
pub trait LoginItem {
    fn set(&self, on: bool) -> anyhow::Result<()>;
    fn get(&self) -> anyhow::Result<bool>;
    /// The program path currently registered with the login item, if any.
    /// Used to detect a stale entry pointing at a moved app. Defaults to
    /// "unknown" so callers that cannot read it keep the old behaviour.
    fn registered_path(&self) -> anyhow::Result<Option<PathBuf>> {
        Ok(None)
    }
}

/// Args registered with the login item. With the menu-bar icon on, a login
/// launch starts behind the icon with no window and shows one when asked to.
/// With the icon off it starts normally, with a window.
pub fn login_args() -> &'static [&'static str] {
    &["--hidden"]
}

/// The executable a macOS login item may register. A Launch Agent needs the
/// binary inside a `.app` bundle: a bare binary has no bundle to open, so there
/// is nothing to register and the caller logs and skips it.
#[cfg(target_os = "macos")]
pub fn login_exe(current_exe: &Path) -> Option<PathBuf> {
    current_exe
        .to_string_lossy()
        .contains(".app/Contents/MacOS/")
        .then(|| current_exe.to_path_buf())
}

/// First entry of `ProgramArguments` in a LaunchAgent plist, if present.
#[cfg(target_os = "macos")]
pub fn first_program_argument(contents: &str) -> Option<PathBuf> {
    let key = contents.find("<key>ProgramArguments</key>")?;
    let after_key = &contents[key..];
    let array_start = after_key.find("<array>")? + "<array>".len();
    let array_rest = &after_key[array_start..];
    let array_end = array_rest.find("</array>")?;
    let array = &array_rest[..array_end];
    let string_start = array.find("<string>")? + "<string>".len();
    let string_rest = &array[string_start..];
    let string_end = string_rest.find("</string>")?;
    let value = string_rest[..string_end].trim();
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(unescape_basic(value)))
    }
}

/// Whether a registered login-item path is stale relative to the current
/// bundle executable. `None` for the current executable means stale.
pub fn is_stale(registered: &Path, current_exe: Option<&Path>) -> bool {
    match current_exe {
        Some(current) => registered != current,
        None => true,
    }
}

#[cfg(target_os = "macos")]
fn unescape_basic(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// The `auto-launch` backed login item. On macOS a Launch Agent registers the
/// bundle's executable and carries `--hidden` in `ProgramArguments`, so a login
/// launch opens no window. SMAppService drops both the path and the args, so it
/// is not used; a bare binary cannot be registered at all.
pub struct AutoLaunchItem {
    inner: AutoLaunch,
}

impl AutoLaunchItem {
    pub fn new() -> anyhow::Result<Self> {
        let exe = std::env::current_exe()?;
        #[cfg(target_os = "macos")]
        let exe = login_exe(&exe)
            .ok_or_else(|| anyhow::anyhow!("parla is not running from an app bundle"))?;
        let mut builder = AutoLaunchBuilder::new();
        builder
            .set_app_name("Parla")
            .set_app_path(&exe.to_string_lossy())
            .set_args(login_args());
        #[cfg(target_os = "macos")]
        builder.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
        Ok(Self {
            inner: builder.build()?,
        })
    }
}

impl LoginItem for AutoLaunchItem {
    fn set(&self, on: bool) -> anyhow::Result<()> {
        if on {
            self.inner.enable()?;
        } else {
            self.inner.disable()?;
        }
        Ok(())
    }

    fn get(&self) -> anyhow::Result<bool> {
        Ok(self.inner.is_enabled()?)
    }

    #[cfg(target_os = "macos")]
    fn registered_path(&self) -> anyhow::Result<Option<PathBuf>> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?;
        let file = home
            .join("Library")
            .join("LaunchAgents")
            .join("Parla.plist");
        if !file.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&file)?;
        Ok(first_program_argument(&contents))
    }
}

/// A recording login item for tests. `fail_next_set` makes the next write fail
/// once, to check that the error is logged and never propagated.
#[derive(Debug, Default)]
pub struct FakeLoginItem {
    state: Cell<bool>,
    sets: RefCell<Vec<bool>>,
    gets: Cell<usize>,
    fail_set: Cell<bool>,
    registered: RefCell<Option<PathBuf>>,
    fail_registered: Cell<bool>,
}

impl FakeLoginItem {
    pub fn new(on: bool) -> Self {
        Self {
            state: Cell::new(on),
            ..Default::default()
        }
    }

    pub fn sets(&self) -> Vec<bool> {
        self.sets.borrow().clone()
    }

    pub fn gets(&self) -> usize {
        self.gets.get()
    }

    pub fn fail_next_set(&self) {
        self.fail_set.set(true);
    }

    /// Makes the next `registered_path` read fail once.
    pub fn fail_next_registered_path(&self) {
        self.fail_registered.set(true);
    }

    pub fn set_registered_path(&self, path: Option<PathBuf>) {
        *self.registered.borrow_mut() = path;
    }
}

impl LoginItem for FakeLoginItem {
    fn set(&self, on: bool) -> anyhow::Result<()> {
        if self.fail_set.take() {
            anyhow::bail!("login_item_set_failed");
        }
        self.sets.borrow_mut().push(on);
        self.state.set(on);
        // A successful write leaves the registered path matching the current
        // bundle (represented as `None` in tests, where the test binary is not
        // bundled), so a stale entry is not reported again.
        *self.registered.borrow_mut() = None;
        Ok(())
    }

    fn get(&self) -> anyhow::Result<bool> {
        self.gets.set(self.gets.get() + 1);
        Ok(self.state.get())
    }

    fn registered_path(&self) -> anyhow::Result<Option<PathBuf>> {
        if self.fail_registered.take() {
            anyhow::bail!("login_item_registered_path_failed");
        }
        Ok(self.registered.borrow().clone())
    }
}

/// Applies `launch_at_login` to the installed login item.
pub struct LoginItems {
    item: Rc<dyn LoginItem>,
}

impl Global for LoginItems {}

impl LoginItems {
    /// Installs `item` and applies the current setting to it.
    pub fn install(cx: &mut App, item: Rc<dyn LoginItem>) {
        cx.set_global(Self { item });
        Self::apply(cx);
    }

    /// Installs the real login item. A system that cannot provide one is
    /// reported in the log; the rest of the app starts as usual.
    pub fn install_real(cx: &mut App) {
        match AutoLaunchItem::new() {
            Ok(item) => Self::install(cx, Rc::new(item)),
            Err(e) => crate::log::error(&format!("login item unavailable: {e}")),
        }
    }

    /// Re-reads the setting and writes the login item only if it differs.
    pub fn apply(cx: &App) {
        let (Some(this), Some(store)) = (
            cx.try_global::<LoginItems>(),
            cx.try_global::<SettingsStore>(),
        ) else {
            return;
        };
        let want = store.current().launch_at_login;

        // Never touch the item when the setting is off, except to disable it.
        if !want {
            match this.item.get() {
                Ok(false) => return,
                Ok(true) => {}
                Err(e) => crate::log::warn(&format!("could not read login item: {e}")),
            }
            if let Err(e) = this.item.set(false) {
                crate::log::error(&format!("could not disable login item: {e}"));
            }
            return;
        }

        // A translocated launch rewrites the entry to its temporary path,
        // and the next normal launch restores it.
        let stale = match this.item.registered_path() {
            Ok(Some(registered)) => {
                #[cfg(target_os = "macos")]
                {
                    let current =
                        crate::login_item::login_exe(&std::env::current_exe().unwrap_or_default());
                    crate::login_item::is_stale(&registered, current.as_deref())
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = registered;
                    true
                }
            }
            Ok(None) => false,
            Err(e) => {
                crate::log::warn(&format!("could not read login item path: {e}"));
                false
            }
        };

        match this.item.get() {
            Ok(true) if stale => {
                if let Err(e) = this.item.set(true) {
                    crate::log::error(&format!("could not set login item to true: {e}"));
                }
                return;
            }
            Ok(current) if current == want && !stale => return,
            Ok(_) => {}
            Err(e) => crate::log::warn(&format!("could not read login item: {e}")),
        }
        if let Err(e) = this.item.set(true) {
            crate::log::error(&format!("could not set login item to true: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn first_program_argument_reads_auto_launch_shape() {
        let contents = r#"<key>AssociatedBundleIdentifiers</key>
<array></array>
<key>ProgramArguments</key>
<array>
    <string>/Applications/Parla.app/Contents/MacOS/parla</string>
    <string>--hidden</string>
</array>"#;
        assert_eq!(
            first_program_argument(contents),
            Some(PathBuf::from(
                "/Applications/Parla.app/Contents/MacOS/parla"
            ))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn first_program_argument_missing_key_returns_none() {
        let contents = r#"<key>Label</key><string>Parla</string>"#;
        assert_eq!(first_program_argument(contents), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn first_program_argument_empty_string_returns_none() {
        let contents = r#"<key>ProgramArguments</key><array><string></string></array>"#;
        assert_eq!(first_program_argument(contents), None);
    }

    #[test]
    fn is_stale_equal_is_not_stale() {
        let p = PathBuf::from("/Applications/Parla.app/Contents/MacOS/parla");
        assert!(!is_stale(&p, Some(&p)));
    }

    #[test]
    fn is_stale_different_is_stale() {
        let registered = PathBuf::from("/old/Parla.app/Contents/MacOS/parla");
        let current = PathBuf::from("/new/Parla.app/Contents/MacOS/parla");
        assert!(is_stale(&registered, Some(&current)));
    }

    #[test]
    fn is_stale_none_is_stale() {
        let registered = PathBuf::from("/Applications/Parla.app/Contents/MacOS/parla");
        assert!(is_stale(&registered, None));
    }
}
