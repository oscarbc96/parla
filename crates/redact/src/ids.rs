//! Stable real -> synthetic mapping, persisted as JSON with owner-only permissions.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Maps real identifiers to synthetic ones, consistently across captures.
/// The saved file contains real values: it is secret and never leaves the cache dir.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct IdMap {
    /// real ID -> synthetic ID (`T024BE7LD` -> `T0000001`).
    #[serde(default)]
    ids: BTreeMap<String, String>,
    /// prefix letter -> last number handed out.
    #[serde(default)]
    counters: BTreeMap<String, u32>,
    /// real workspace subdomain -> `workspace`, `workspace2`, ...
    #[serde(default)]
    hosts: BTreeMap<String, String>,
    /// fake kind -> real value -> number (names, emails, urls, ...).
    #[serde(default)]
    fakes: BTreeMap<String, BTreeMap<String, u32>>,
}

impl IdMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the map, or return an empty one when the file does not exist.
    pub fn load(path: &Path) -> io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e),
        }
    }

    /// Write the map as JSON with mode 0600 (and its directory with 0700 when created).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            create_private_dir(dir)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        write_private(path, text.as_bytes())
    }

    /// Synthetic ID for `real`: same prefix letter, `{prefix}{n:07}`, stable.
    pub fn id(&mut self, real: &str) -> String {
        if let Some(found) = self.ids.get(real) {
            return found.clone();
        }
        let prefix = real.chars().next().unwrap_or('X').to_string();
        let counter = self.counters.entry(prefix.clone()).or_insert(0);
        *counter += 1;
        let synthetic = format!("{prefix}{:07}", *counter);
        self.ids.insert(real.to_string(), synthetic.clone());
        synthetic
    }

    /// Real ID behind a synthetic one, if the map holds it (the owner's local map only).
    pub fn real_for(&self, synthetic: &str) -> Option<&str> {
        self.ids
            .iter()
            .find(|(_, s)| s.as_str() == synthetic)
            .map(|(real, _)| real.as_str())
    }

    /// Synthetic workspace subdomain: `workspace`, then `workspace2`, `workspace3`, ...
    pub fn host_label(&mut self, real: &str) -> String {
        let key = real.to_ascii_lowercase();
        if let Some(found) = self.hosts.get(&key) {
            return found.clone();
        }
        let n = self.hosts.len() + 1;
        let label = if n == 1 {
            "workspace".to_string()
        } else {
            format!("workspace{n}")
        };
        self.hosts.insert(key, label.clone());
        label
    }

    /// Stable 1-based number for `real` within `kind` (names, emails, urls).
    pub fn fake_number(&mut self, kind: &str, real: &str) -> u32 {
        let bucket = self.fakes.entry(kind.to_string()).or_default();
        if let Some(n) = bucket.get(real) {
            return *n;
        }
        let n = bucket.len() as u32 + 1;
        bucket.insert(real.to_string(), n);
        n
    }

    /// Number of real values mapped so far (counts only, for reporting).
    pub fn len(&self) -> usize {
        self.ids.len() + self.hosts.len() + self.fakes.values().map(BTreeMap::len).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(unix)]
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Write `bytes` to `path`, creating it with mode 0600 and fixing the mode if it existed.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        let mut file = std::fs::File::create(path)?;
        file.write_all(bytes)
    }
}
