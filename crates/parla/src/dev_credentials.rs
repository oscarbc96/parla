//! Debug-build credential store: a JSON file instead of the OS keychain.
//!
//! A locally built, self-signed binary has no team ID, so the macOS keychain
//! pins each item to the binary's code hash and prompts again after every
//! rebuild. Like Zed's development credentials provider, debug builds keep
//! credentials and store keys in `<data_dir>/development_credentials.json`
//! (mode 0600, written atomically). Release builds never use it: [`select`]
//! returns [`Backend::Keychain`] whenever `debug_assertions` is off.
//! `PARLA_DEV_USE_KEYCHAIN=1` forces the keychain in a debug build.
//!
//! The file holds the same payloads the keychain would, unencrypted, on the
//! developer's own disk. Contents are never logged.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use store::StoreError;
use store_sqlite::{DbId, Key, KeySource};
use zeroize::{Zeroize, Zeroizing};

use crate::store_keys::entry_url as key_url;

/// Environment variable that forces the real keychain in a debug build.
pub const USE_KEYCHAIN_VAR: &str = "PARLA_DEV_USE_KEYCHAIN";

/// File name inside the data dir.
pub const FILE_NAME: &str = "development_credentials.json";

/// Which secret store the app uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Keychain,
    DevFile,
}

/// Pure selection rule. Release builds always get the keychain; debug builds
/// get the file unless `use_keychain` is set and non-empty.
pub fn select(debug_assertions: bool, use_keychain: Option<&OsStr>) -> Backend {
    if !debug_assertions || use_keychain.is_some_and(|v| !v.is_empty()) {
        Backend::Keychain
    } else {
        Backend::DevFile
    }
}

/// The selection for this build and environment.
pub fn current() -> Backend {
    select(
        cfg!(debug_assertions),
        std::env::var_os(USE_KEYCHAIN_VAR).as_deref(),
    )
}

/// Builds only the chosen store: the other closure is never called, so a
/// debug build never constructs (or touches) the keychain.
pub fn choose<T>(
    backend: Backend,
    keychain: impl FnOnce() -> T,
    dev_file: impl FnOnce() -> T,
) -> T {
    match backend {
        Backend::Keychain => keychain(),
        Backend::DevFile => dev_file(),
    }
}

/// The startup notice for the file backend. Names the path, never contents.
pub fn startup_notice(path: &Path) -> String {
    format!(
        "debug build: credentials in {}; set {USE_KEYCHAIN_VAR}=1 to use the keychain",
        path.display()
    )
}

#[derive(Serialize, Deserialize)]
struct Entry {
    user: String,
    value: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Entries(BTreeMap<String, Entry>);

impl Drop for Entries {
    fn drop(&mut self) {
        for e in self.0.values_mut() {
            e.value.zeroize();
        }
    }
}

/// The JSON file, shared by the credential and key stores so their
/// read-modify-write cycles never interleave.
pub struct DevFile {
    path: PathBuf,
    lock: Mutex<()>,
}

impl DevFile {
    pub fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            lock: Mutex::new(()),
        })
    }

    /// The process-wide file for `path`: every reader and writer shares one
    /// lock, so their read-modify-write cycles never interleave. The path is
    /// fixed for a run (the data dir), so one slot is enough.
    pub(crate) fn shared(path: PathBuf) -> Arc<Self> {
        type Shared = Mutex<Option<(PathBuf, Arc<DevFile>)>>;
        static SHARED: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();
        let mut slot = SHARED
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some((p, file)) = &*slot
            && *p == path
        {
            return file.clone();
        }
        let file = DevFile::new(path.clone());
        *slot = Some((path, file.clone()));
        file
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> std::io::Result<Entries> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => Zeroizing::new(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Entries::default()),
            Err(e) => return Err(e),
        };
        serde_json::from_slice(&bytes).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "development credentials file is unreadable",
            )
        })
    }

    fn write(&self, entries: &Entries) -> std::io::Result<()> {
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        create_private_dir(dir)?;
        let bytes =
            Zeroizing::new(serde_json::to_vec_pretty(entries).map_err(std::io::Error::other)?);
        // `NamedTempFile` is created 0600 on unix, so the secret is never
        // readable by others, even briefly.
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(&bytes)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&self.path).map_err(|e| e.error)?;
        Ok(())
    }

    pub(crate) fn get(&self, url: &str) -> std::io::Result<Option<(String, Zeroizing<Vec<u8>>)>> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let entries = self.read()?;
        Ok(entries
            .0
            .get(url)
            .map(|e| (e.user.clone(), Zeroizing::new(e.value.as_bytes().to_vec()))))
    }

    pub(crate) fn put(&self, url: &str, user: &str, value: &[u8]) -> std::io::Result<()> {
        let value = Zeroizing::new(
            String::from_utf8(value.to_vec())
                .map_err(|_| std::io::Error::other("secret is not utf-8"))?,
        );
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut entries = self.read()?;
        entries.0.insert(
            url.to_string(),
            Entry {
                user: user.to_string(),
                value: value.to_string(),
            },
        );
        self.write(&entries)
    }

    pub(crate) fn remove(&self, url: &str) -> std::io::Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut entries = self.read()?;
        if entries.0.remove(url).is_some() {
            self.write(&entries)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

fn store_err(_: std::io::Error) -> StoreError {
    StoreError::Other("development credentials file unavailable".to_string())
}

/// Store keys in the dev file.
#[derive(Clone)]
pub struct DevKeys(pub Arc<DevFile>);

const KEY_USER: &str = "parla";

impl DevKeys {
    fn store_new(&self, url: &str) -> Result<Key, StoreError> {
        let key = Key::random();
        let hex = key.to_hex();
        self.0
            .put(url, KEY_USER, hex.as_bytes())
            .map_err(store_err)?;
        Ok(key)
    }
}

impl KeySource for DevKeys {
    fn get_or_create(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        let url = key_url(ws);
        Box::pin(async move {
            match self.0.get(&url).map_err(store_err)? {
                Some((_, bytes)) => {
                    let text = Zeroizing::new(String::from_utf8_lossy(&bytes).to_string());
                    // Never overwrite a malformed entry silently.
                    Key::from_hex(text.trim()).ok_or(StoreError::Unreadable)
                }
                None => self.store_new(&url),
            }
        })
    }

    fn replace(&self, ws: &DbId) -> BoxFuture<'_, Result<Key, StoreError>> {
        let url = key_url(ws);
        Box::pin(async move { self.store_new(&url) })
    }

    fn delete(&self, ws: &DbId) -> BoxFuture<'_, Result<(), StoreError>> {
        let url = key_url(ws);
        Box::pin(async move { self.0.remove(&url).map_err(store_err) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::WorkspaceKey;
    use std::cell::Cell;

    fn ws() -> DbId {
        DbId::Workspace(WorkspaceKey::new("fake", "T1"))
    }

    fn file(dir: &tempfile::TempDir) -> Arc<DevFile> {
        DevFile::new(dir.path().join("data").join(FILE_NAME))
    }

    #[test]
    fn keys_round_trip_replace_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let k = DevKeys(file(&dir));
        futures::executor::block_on(async {
            let a = k.get_or_create(&ws()).await.unwrap();
            let b = k.get_or_create(&ws()).await.unwrap();
            assert_eq!(*a.to_hex(), *b.to_hex());
            let c = k.replace(&ws()).await.unwrap();
            assert_ne!(*a.to_hex(), *c.to_hex());
            assert_eq!(*k.get_or_create(&ws()).await.unwrap().to_hex(), *c.to_hex());
            k.delete(&ws()).await.unwrap();
            let d = k.get_or_create(&ws()).await.unwrap();
            assert_ne!(*c.to_hex(), *d.to_hex());
        });
    }

    #[cfg(unix)]
    #[test]
    fn file_is_0600_and_dir_0700() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let f = file(&dir);
        futures::executor::block_on(DevKeys(f.clone()).get_or_create(&ws())).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(f.path()), 0o600);
        assert_eq!(mode(f.path().parent().unwrap()), 0o700);
        // A rewrite keeps the mode and leaves no temp files behind.
        futures::executor::block_on(DevKeys(f.clone()).delete(&ws())).unwrap();
        assert_eq!(mode(f.path()), 0o600);
        assert_eq!(
            std::fs::read_dir(f.path().parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn selection_rules() {
        let set = OsStr::new("1");
        assert_eq!(select(true, None), Backend::DevFile);
        assert_eq!(select(true, Some(OsStr::new(""))), Backend::DevFile);
        assert_eq!(select(true, Some(set)), Backend::Keychain);
        assert_eq!(select(false, None), Backend::Keychain);
        assert_eq!(select(false, Some(set)), Backend::Keychain);
        assert_eq!(select(false, Some(OsStr::new(""))), Backend::Keychain);
    }

    #[test]
    fn release_always_picks_the_keychain() {
        for v in [None, Some(""), Some("1"), Some("0")] {
            assert_eq!(select(false, v.map(OsStr::new)), Backend::Keychain);
        }
    }

    #[test]
    fn keychain_is_never_built_in_debug_default() {
        let touched = Cell::new(0);
        let dev = Cell::new(0);
        choose(
            select(true, None),
            || touched.set(touched.get() + 1),
            || dev.set(dev.get() + 1),
        );
        assert_eq!((touched.get(), dev.get()), (0, 1));
        choose(
            select(true, Some(OsStr::new("1"))),
            || touched.set(touched.get() + 1),
            || dev.set(dev.get() + 1),
        );
        assert_eq!((touched.get(), dev.get()), (1, 1));
    }

    #[test]
    fn notice_names_path_only() {
        let n = startup_notice(Path::new("/x/development_credentials.json"));
        assert_eq!(
            n,
            "debug build: credentials in /x/development_credentials.json; set PARLA_DEV_USE_KEYCHAIN=1 to use the keychain"
        );
    }
}
