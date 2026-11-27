use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::WorkspaceKey;

pub(crate) const TEMPLATE: &str =
    include_str!("../../../assets/settings/initial_user_settings.jsonc");

/// Locations of every user-editable config file and directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub root: PathBuf,
    pub settings: PathBuf,
    /// The JSON schema of `settings.json`, written beside it for editors.
    pub schema: PathBuf,
    pub keymap: PathBuf,
    pub themes_dir: PathBuf,
    pub workspaces_dir: PathBuf,
    pub plugins_dir: PathBuf,
}

impl ConfigPaths {
    /// Paths under an explicit root.
    pub fn at(root: impl Into<PathBuf>) -> ConfigPaths {
        let root = root.into();
        ConfigPaths {
            settings: root.join("settings.json"),
            schema: root.join("settings.schema.json"),
            keymap: root.join("keymap.json"),
            themes_dir: root.join("themes"),
            workspaces_dir: root.join("workspaces"),
            plugins_dir: root.join("plugins"),
            root,
        }
    }

    /// Per-workspace settings file: `workspaces/<provider>/<id>.json`. Characters
    /// that could leave the directory are replaced by `_`.
    pub fn workspace_file(&self, key: &WorkspaceKey) -> PathBuf {
        let (provider, id) = key.path_parts();
        self.workspaces_dir
            .join(provider)
            .join(format!("{id}.json"))
    }

    /// `PARLA_CONFIG_DIR` if set and non-empty, else the platform config dir
    /// (`~/.config/parla`, `%APPDATA%\parla`, `$XDG_CONFIG_HOME/parla`).
    pub fn discover() -> ConfigPaths {
        if let Some(dir) = std::env::var_os("PARLA_CONFIG_DIR").filter(|v| !v.is_empty()) {
            return Self::at(dir);
        }
        Self::at(platform_root())
    }

    /// Create the directories and write `settings.json` (commented template)
    /// and `keymap.json` (`[]`), each only when missing.
    pub fn ensure_exists(&self) -> io::Result<()> {
        for dir in [
            &self.root,
            &self.themes_dir,
            &self.workspaces_dir,
            &self.plugins_dir,
        ] {
            fs::create_dir_all(dir)?;
        }
        write_if_missing(&self.settings, TEMPLATE)?;
        write_if_missing(&self.keymap, "[]")?;
        self.write_schema()
    }

    /// Writes the schema the `$schema` pointer of `settings.json` names, when
    /// it differs from what is there, so completion follows the app version.
    fn write_schema(&self) -> io::Result<()> {
        let mut text =
            serde_json::to_string_pretty(&crate::settings_schema()).map_err(io::Error::other)?;
        text.push('\n');
        if fs::read_to_string(&self.schema).is_ok_and(|old| old == text) {
            return Ok(());
        }
        // Temp file then rename: an editor never reads half a schema.
        let tmp = self.schema.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(tmp, &self.schema)
    }
}

fn write_if_missing(path: &Path, text: &str) -> io::Result<()> {
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => io::Write::write_all(&mut f, text.as_bytes()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

fn platform_root() -> PathBuf {
    if cfg!(windows) {
        return dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("parla");
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(x).join("parla");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("parla")
}

/// `PARLA_DATA_DIR` if set and non-empty, else the platform data dir joined with `parla`.
pub fn data_dir() -> PathBuf {
    env_or("PARLA_DATA_DIR", dirs::data_dir)
}

/// `PARLA_CACHE_DIR` if set and non-empty, else the platform cache dir joined with `parla`.
pub fn cache_dir() -> PathBuf {
    env_or("PARLA_CACHE_DIR", dirs::cache_dir)
}

fn env_or(var: &str, platform: fn() -> Option<PathBuf>) -> PathBuf {
    if let Some(dir) = std::env::var_os(var).filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    platform()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("parla")
}

/// Runtime files (sockets), `<data_dir>/run`.
pub fn run_dir() -> PathBuf {
    data_dir().join("run")
}

/// Where the app listens for `parla mcp` shims: `run_dir()/mcp.sock` on Unix,
/// a per-user named pipe on Windows.
pub fn mcp_socket_path() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(windows_pipe_name(&windows_user_sid()))
    }
    #[cfg(not(windows))]
    {
        run_dir().join("mcp.sock")
    }
}

/// `\\.\pipe\parla-mcp-<16 hex of SHA-256 of the user SID>`. The name depends
/// on the user only, so two users on one machine never share a pipe.
pub fn windows_pipe_name(user_sid: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(user_sid.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!(r"\\.\pipe\parla-mcp-{hex}")
}

#[cfg(windows)]
fn windows_user_sid() -> String {
    use std::sync::OnceLock;
    static SID: OnceLock<String> = OnceLock::new();
    SID.get_or_init(|| crate::win_sid::current_user_sid().unwrap_or_default())
        .clone()
}

/// Whether `p` fits a Unix socket address (`sun_path`: 104 bytes on macOS,
/// 108 elsewhere, NUL included). Always true on Windows.
pub fn socket_path_fits(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let limit = if cfg!(target_os = "macos") { 104 } else { 108 };
        p.as_os_str().as_bytes().len() < limit
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        true
    }
}
