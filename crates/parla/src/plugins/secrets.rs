//! The plugin secrets store: one entry per plugin and key in the OS keychain
//! (release builds) or the development credentials file (debug builds, the
//! same split the workspace credentials use).
//!
//! - Keychain entries are named by URL `parla://plugin/<id>/<key>`, username
//!   `parla`, like the store keys' `parla://store/...`. GPUI's `AsyncApp` is
//!   not `Send`, so requests hop to the main thread over a channel; callers on
//!   the Tokio runtime wait and never block the main thread.
//! - The names of a plugin's keys are tracked in a small index entry
//!   (`parla://plugin/<id>`, a JSON array), so `remove_all` works. At most
//!   [`MAX_KEYS`] names are kept.
//! - Values are [`SecretText`]: wiped on drop, redacted in `Debug`, and never
//!   part of an error or a log line.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use domain::SecretText;
use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use gpui::App;
use plugin_host::install::PluginSecrets;
use plugin_host::manifest::PluginId;

use crate::dev_credentials::DevFile;

/// Most key names one plugin may have (the index stays small).
pub const MAX_KEYS: usize = 32;
/// Longest key name.
const MAX_KEY_BYTES: usize = 64;
/// Username of every keychain item, as the store keys use.
const USER: &str = "parla";

/// The app's secrets store, behind one object-safe trait so tests script it.
pub trait Secrets: Send + Sync + 'static {
    fn get<'a>(
        &'a self,
        plugin: &str,
        key: &str,
    ) -> BoxFuture<'a, Result<Option<SecretText>, String>>;
    fn set<'a>(
        &'a self,
        plugin: &str,
        key: &str,
        value: SecretText,
    ) -> BoxFuture<'a, Result<(), String>>;
    fn delete<'a>(&'a self, plugin: &str, key: &str) -> BoxFuture<'a, Result<(), String>>;
    /// The plugin's key names, sorted; never the values.
    fn keys<'a>(&'a self, plugin: &str) -> BoxFuture<'a, Result<Vec<String>, String>>;
}

/// `parla://plugin/<id>/<key>`, the keychain entry of one secret.
pub fn entry_url(plugin: &str, key: &str) -> String {
    format!("parla://plugin/{plugin}/{key}")
}

/// `parla://plugin/<id>`, the index of the plugin's key names.
pub fn index_url(plugin: &str) -> String {
    format!("parla://plugin/{plugin}")
}

/// A plugin id (as the manifest rules spell it) and a key name
/// (`[a-z0-9_-]`, at most 64 bytes): both end up in a keychain URL.
fn check_names(plugin: &str, key: &str) -> Result<(), String> {
    if PluginId::parse(plugin).is_none() {
        return Err("invalid plugin id".into());
    }
    if key.is_empty()
        || key.len() > MAX_KEY_BYTES
        || !key.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        })
    {
        return Err("invalid secret key".into());
    }
    Ok(())
}

/// The raw per-URL store: the keychain through the main thread, or the
/// development credentials file.
enum Raw {
    Dev(Arc<DevFile>),
    Keychain(Keychain),
}

impl Raw {
    async fn get(&self, url: &str) -> Result<Option<Vec<u8>>, String> {
        match self {
            Raw::Dev(file) => file
                .get(url)
                .map(|found| found.map(|(_, bytes)| bytes.to_vec()))
                .map_err(|_| "cannot read the development credentials file".to_owned()),
            Raw::Keychain(k) => k.ask(|reply| Request::Read(url.to_owned(), reply)).await,
        }
    }

    async fn put(&self, url: &str, value: &[u8]) -> Result<(), String> {
        match self {
            Raw::Dev(file) => file
                .put(url, USER, value)
                .map_err(|_| "cannot write the development credentials file".to_owned()),
            Raw::Keychain(k) => {
                k.ask(|reply| Request::Write(url.to_owned(), value.to_vec(), reply))
                    .await
            }
        }
    }

    async fn delete(&self, url: &str) -> Result<(), String> {
        match self {
            Raw::Dev(file) => file
                .remove(url)
                .map_err(|_| "cannot write the development credentials file".to_owned()),
            Raw::Keychain(k) => k.ask(|reply| Request::Delete(url.to_owned(), reply)).await,
        }
    }
}

/// The plugin secrets of the app, in the keychain or the development file.
pub struct PluginSecretStore {
    raw: Raw,
}

impl PluginSecretStore {
    /// Debug builds, `PARLA_DEV_USE_KEYCHAIN` unset: the JSON file.
    pub fn dev(file: Arc<DevFile>) -> Self {
        Self {
            raw: Raw::Dev(file),
        }
    }

    /// Release builds (and an opted-in debug build): the keychain.
    pub fn keychain(cx: &mut App) -> Self {
        Self {
            raw: Raw::Keychain(Keychain::new(cx)),
        }
    }

    /// The plugin's key names from its index entry.
    pub async fn keys(&self, plugin: &str) -> Result<Vec<String>, String> {
        let url = index_url(plugin);
        let Some(bytes) = self.raw.get(&url).await? else {
            return Ok(Vec::new());
        };
        let mut keys: Vec<String> = serde_json::from_slice(&bytes).unwrap_or_default();
        keys.retain(|k| check_names(plugin, k).is_ok());
        keys.sort();
        keys.dedup();
        keys.truncate(MAX_KEYS);
        Ok(keys)
    }

    async fn write_index(&self, plugin: &str, keys: &[String]) -> Result<(), String> {
        let json =
            serde_json::to_vec(keys).map_err(|_| "cannot encode the key index".to_owned())?;
        self.raw.put(&index_url(plugin), &json).await
    }

    /// Stores `value` for `plugin`'s `key`, adding the name to its index.
    pub async fn set(&self, plugin: &str, key: &str, value: SecretText) -> Result<(), String> {
        check_names(plugin, key)?;
        let mut keys = self.keys(plugin).await?;
        let known = keys.iter().any(|k| k == key);
        if !known && keys.len() >= MAX_KEYS {
            return Err("too many secrets".into());
        }
        self.raw
            .put(&entry_url(plugin, key), value.expose().as_bytes())
            .await?;
        if !known {
            keys.push(key.to_owned());
            keys.sort();
            self.write_index(plugin, &keys).await?;
        }
        Ok(())
    }

    /// The secret's value, or `None` when the key is not stored.
    pub async fn get(&self, plugin: &str, key: &str) -> Result<Option<SecretText>, String> {
        check_names(plugin, key)?;
        let bytes = self.raw.get(&entry_url(plugin, key)).await?;
        Ok(bytes.map(|bytes| {
            let text = String::from_utf8_lossy(&bytes);
            SecretText::new(text.trim())
        }))
    }

    /// Removes one secret and its index entry. Removing a missing key is fine.
    pub async fn delete(&self, plugin: &str, key: &str) -> Result<(), String> {
        check_names(plugin, key)?;
        self.raw.delete(&entry_url(plugin, key)).await?;
        let mut keys = self.keys(plugin).await?;
        if keys.iter().any(|k| k == key) {
            keys.retain(|k| k != key);
            self.write_index(plugin, &keys).await?;
        }
        Ok(())
    }

    /// Removes every secret of `plugin` and its index (uninstall).
    pub async fn remove_all(&self, plugin: &str) -> Result<(), String> {
        if PluginId::parse(plugin).is_none() {
            return Err("invalid plugin id".into());
        }
        let mut first_error = None;
        let keys = self.keys(plugin).await.unwrap_or_default();
        for key in keys {
            if let Err(e) = self.raw.delete(&entry_url(plugin, &key)).await
                && first_error.is_none()
            {
                first_error = Some(e);
            }
        }
        if let Err(e) = self.raw.delete(&index_url(plugin)).await
            && first_error.is_none()
        {
            first_error = Some(format!("cannot remove the key index: {e}"));
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Secrets for PluginSecretStore {
    fn get<'a>(
        &'a self,
        plugin: &str,
        key: &str,
    ) -> BoxFuture<'a, Result<Option<SecretText>, String>> {
        let (plugin, key) = (plugin.to_owned(), key.to_owned());
        Box::pin(async move { PluginSecretStore::get(self, &plugin, &key).await })
    }

    fn set<'a>(
        &'a self,
        plugin: &str,
        key: &str,
        value: SecretText,
    ) -> BoxFuture<'a, Result<(), String>> {
        let (plugin, key) = (plugin.to_owned(), key.to_owned());
        Box::pin(async move { PluginSecretStore::set(self, &plugin, &key, value).await })
    }

    fn delete<'a>(&'a self, plugin: &str, key: &str) -> BoxFuture<'a, Result<(), String>> {
        let (plugin, key) = (plugin.to_owned(), key.to_owned());
        Box::pin(async move { PluginSecretStore::delete(self, &plugin, &key).await })
    }

    fn keys<'a>(&'a self, plugin: &str) -> BoxFuture<'a, Result<Vec<String>, String>> {
        let plugin = plugin.to_owned();
        Box::pin(async move { PluginSecretStore::keys(self, &plugin).await })
    }
}

/// The uninstall hook of `plugin_host::install`. Blocking: the installer's
/// own docs require it off the main thread (a keychain request is answered by
/// the main thread, so this must never run there).
impl PluginSecrets for PluginSecretStore {
    fn remove_all(&self, id: &PluginId) -> Result<(), String> {
        futures::executor::block_on(PluginSecretStore::remove_all(self, id.as_str()))
    }
}

/// An in-memory [`Secrets`] for tests and fixtures. Values are plain strings
/// here; nothing leaves the process.
#[derive(Default)]
pub struct MemorySecrets {
    entries: Mutex<BTreeMap<(String, String), String>>,
}

impl std::fmt::Debug for MemorySecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the values: a failing test may print the whole fixture.
        f.debug_struct("MemorySecrets")
            .field("keys", &self.lock().keys().collect::<Vec<_>>())
            .finish()
    }
}

impl MemorySecrets {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<(String, String), String>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stores `value` without a future, for a fixture's first state.
    pub fn insert(&self, plugin: &str, key: &str, value: &str) {
        self.lock()
            .insert((plugin.to_owned(), key.to_owned()), value.to_owned());
    }

    /// The stored value, for assertions.
    pub fn value(&self, plugin: &str, key: &str) -> Option<String> {
        self.lock()
            .get(&(plugin.to_owned(), key.to_owned()))
            .cloned()
    }
}

impl Secrets for MemorySecrets {
    fn get<'a>(
        &'a self,
        plugin: &str,
        key: &str,
    ) -> BoxFuture<'a, Result<Option<SecretText>, String>> {
        let value = self.value(plugin, key).map(SecretText::new);
        Box::pin(async move { Ok(value) })
    }

    fn set<'a>(
        &'a self,
        plugin: &str,
        key: &str,
        value: SecretText,
    ) -> BoxFuture<'a, Result<(), String>> {
        self.insert(plugin, key, value.expose());
        Box::pin(async { Ok(()) })
    }

    fn delete<'a>(&'a self, plugin: &str, key: &str) -> BoxFuture<'a, Result<(), String>> {
        self.lock().remove(&(plugin.to_owned(), key.to_owned()));
        Box::pin(async { Ok(()) })
    }

    fn keys<'a>(&'a self, plugin: &str) -> BoxFuture<'a, Result<Vec<String>, String>> {
        let mut keys: Vec<String> = self
            .lock()
            .keys()
            .filter(|(id, _)| id == plugin)
            .map(|(_, key)| key.clone())
            .collect();
        keys.sort();
        Box::pin(async move { Ok(keys) })
    }
}

// ------------------------------------------------------------- keychain

type Reply<T> = oneshot::Sender<Result<T, String>>;

enum Request {
    Read(String, Reply<Option<Vec<u8>>>),
    Write(String, Vec<u8>, Reply<()>),
    Delete(String, Reply<()>),
}

fn unavailable() -> String {
    "keychain unavailable".to_owned()
}

/// One keychain request at a time, answered on the main thread.
struct Keychain {
    tx: mpsc::UnboundedSender<Request>,
}

impl Keychain {
    fn new(cx: &mut App) -> Self {
        let (tx, mut rx) = mpsc::unbounded::<Request>();
        cx.spawn(async move |cx| {
            use futures::StreamExt as _;
            while let Some(req) = rx.next().await {
                match req {
                    Request::Read(url, reply) => {
                        let r = cx.update(|cx| cx.read_credentials(&url)).await;
                        let _ = reply.send(
                            r.map(|found| found.map(|(_, bytes)| bytes))
                                .map_err(|e| format!("keychain: {e}")),
                        );
                    }
                    Request::Write(url, bytes, reply) => {
                        let r = cx
                            .update(|cx| cx.write_credentials(&url, USER, &bytes))
                            .await;
                        let _ = reply.send(r.map_err(|e| format!("keychain: {e}")));
                    }
                    Request::Delete(url, reply) => {
                        let r = cx.update(|cx| cx.delete_credentials(&url)).await;
                        let _ = reply.send(r.map_err(|e| format!("keychain: {e}")));
                    }
                }
            }
        })
        .detach();
        Self { tx }
    }

    async fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Request) -> Result<T, String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .unbounded_send(make(reply))
            .map_err(|_| unavailable())?;
        rx.await.map_err(|_| unavailable())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &tempfile::TempDir) -> PluginSecretStore {
        PluginSecretStore::dev(DevFile::new(
            dir.path().join("development_credentials.json"),
        ))
    }

    #[test]
    fn secrets_round_trip_on_the_dev_file() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        futures::executor::block_on(async {
            assert!(s.get("giphy", "api_key").await.unwrap().is_none());
            assert!(s.keys("giphy").await.unwrap().is_empty());
            s.set("giphy", "api_key", SecretText::new("first"))
                .await
                .unwrap();
            s.set("giphy", "other", SecretText::new("second"))
                .await
                .unwrap();
            let key = s.get("giphy", "api_key").await.unwrap().unwrap();
            assert_eq!(key.expose(), "first");
            assert_eq!(
                s.keys("giphy").await.unwrap(),
                vec!["api_key".to_string(), "other".to_string()]
            );
            s.delete("giphy", "other").await.unwrap();
            assert_eq!(s.keys("giphy").await.unwrap(), vec!["api_key".to_string()]);
            s.remove_all("giphy").await.unwrap();
            assert!(s.get("giphy", "api_key").await.unwrap().is_none());
            assert!(s.keys("giphy").await.unwrap().is_empty());
        });
    }

    #[test]
    fn secrets_are_namespaced_by_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        futures::executor::block_on(async {
            s.set("giphy", "api_key", SecretText::new("a"))
                .await
                .unwrap();
            s.set("klipy", "api_key", SecretText::new("b"))
                .await
                .unwrap();
            assert_eq!(
                s.get("giphy", "api_key").await.unwrap().unwrap().expose(),
                "a"
            );
            s.remove_all("giphy").await.unwrap();
            assert!(s.get("giphy", "api_key").await.unwrap().is_none());
            assert!(s.get("klipy", "api_key").await.unwrap().is_some());
        });
    }

    #[test]
    fn the_index_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        futures::executor::block_on(async {
            for i in 0..MAX_KEYS {
                s.set("giphy", &format!("k{i}"), SecretText::new("v"))
                    .await
                    .unwrap();
            }
            let err = s
                .set("giphy", "one_more", SecretText::new("v"))
                .await
                .unwrap_err();
            assert_eq!(err, "too many secrets");
            // Overwriting an existing key still works.
            s.set("giphy", "k0", SecretText::new("new")).await.unwrap();
        });
    }

    #[test]
    fn bad_names_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        futures::executor::block_on(async {
            assert!(
                s.set("../x", "api_key", SecretText::new("v"))
                    .await
                    .is_err()
            );
            assert!(s.set("giphy", "../x", SecretText::new("v")).await.is_err());
            assert!(s.set("giphy", "", SecretText::new("v")).await.is_err());
            assert!(s.get("giphy", "API KEY").await.is_err());
            assert!(s.remove_all("not a plugin").await.is_err());
        });
    }

    #[test]
    fn the_file_is_private_and_never_prints_the_value() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(&dir);
        futures::executor::block_on(s.set("giphy", "api_key", SecretText::new("hunter2"))).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let path = dir.path().join("development_credentials.json");
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let memory = MemorySecrets::new();
        memory.insert("giphy", "api_key", "hunter2");
        assert!(!format!("{memory:?}").contains("hunter2"));
    }
}
