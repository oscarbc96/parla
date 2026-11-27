use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;

use crate::{
    ConfigPaths, Layer, Settings, SettingsError, WorkspaceKey, default_layer, effective_for,
    parse_layers,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ReloadOutcome {
    Unchanged,
    Changed,
    Rejected(SettingsError),
}

/// Effective settings loaded from disk. An invalid edit keeps the last good
/// value and records the error until the file is fixed.
pub struct SettingsStore {
    paths: ConfigPaths,
    current: Settings,
    last_error: Option<SettingsError>,
    warnings: Vec<String>,
    generation: u64,
    /// Per-workspace effective settings, valid for one generation. At most one
    /// entry per workspace; cleared whenever the generation moves.
    workspace_cache: Mutex<(u64, HashMap<WorkspaceKey, Settings>)>,
}

#[cfg(feature = "gpui")]
impl gpui::Global for SettingsStore {}

/// Settings keys an older Parla wrote and no version reads any more. Parla
/// deletes them from the file whenever settings load or reload (`/mcp/transport`: stdio only).
pub const REMOVED_KEYS: &[&str] = &["/mcp/transport"];

/// Only a missing file is empty; any other read error is reported.
fn read_or_empty(path: &std::path::Path) -> Result<String, SettingsError> {
    match fs::read_to_string(path) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(SettingsError::Parse {
            path: path.to_path_buf(),
            line: 1,
            col: 1,
            message: format!("cannot read file: {e}"),
        }),
    }
}

impl SettingsStore {
    /// Load from disk. An invalid file yields defaults plus `last_error`.
    pub fn load(paths: ConfigPaths) -> SettingsStore {
        let mut store = SettingsStore {
            paths,
            current: Settings::default(),
            last_error: None,
            warnings: Vec::new(),
            generation: 0,
            workspace_cache: Mutex::new((0, HashMap::new())),
        };
        store.reload();
        store
    }

    pub fn current(&self) -> &Settings {
        &self.current
    }

    /// Counts effective-settings changes (a reload that changed `settings.json`,
    /// or a workspace file reported by [`note_workspace_changed`](Self::note_workspace_changed)),
    /// so a holder of a derived value knows when to rebuild it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// A `workspaces/<provider>/<id>.json` file changed on disk.
    pub fn note_workspace_changed(&mut self) {
        self.generation += 1;
    }

    /// JSON pointers of keys in `settings.json` that Parla does not read (a
    /// typo, or a removed setting an old file still carries, like
    /// `/mcp/transport`). The file loads; the keys are ignored.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Removes from `settings.json` the keys of [`REMOVED_KEYS`] the file
    /// still carries (a text edit that keeps comments and every other key),
    /// then reloads. Returns the pointers removed. A file that cannot be
    /// written (read-only) keeps the key, which stays ignored; nothing is
    /// returned and there is no error to show.
    pub fn remove_stale_keys(&mut self) -> Vec<String> {
        let mut removed = Vec::new();
        for pointer in REMOVED_KEYS {
            if !self.warnings.iter().any(|w| w == pointer) {
                continue;
            }
            let path: Vec<&str> = pointer.split('/').skip(1).collect();
            if crate::remove_key_in_file(&self.paths.settings, &path).is_ok_and(|done| done) {
                removed.push((*pointer).to_string());
            }
        }
        if !removed.is_empty() {
            self.reload();
        }
        removed
    }

    pub fn last_error(&self) -> Option<&SettingsError> {
        self.last_error.as_ref()
    }

    /// Settings with the workspace's file (`workspaces/<provider>/<id>.json`) layered
    /// on top. Falls back to `current()` if the workspace file is invalid.
    /// The result is cached until the generation moves, so repeated calls do no
    /// disk IO; a fallback is never cached, so a fixed file is picked up.
    pub fn for_workspace(&self, key: &WorkspaceKey) -> Settings {
        let mut cache = self
            .workspace_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cache.0 != self.generation {
            *cache = (self.generation, HashMap::new());
        }
        if let Some(s) = cache.1.get(key) {
            return s.clone();
        }
        match self.read_workspace(key) {
            Some(s) => {
                cache.1.insert(key.clone(), s.clone());
                s
            }
            None => self.current.clone(),
        }
    }

    fn read_workspace(&self, key: &WorkspaceKey) -> Option<Settings> {
        let ws_path = self.paths.workspace_file(key);
        let (Ok(user), Ok(ws)) = (read_or_empty(&self.paths.settings), read_or_empty(&ws_path))
        else {
            return None;
        };
        let base = [
            default_layer(),
            Layer {
                path: &self.paths.settings,
                text: &user,
            },
        ];
        let workspace = Layer {
            path: &ws_path,
            text: &ws,
        };
        effective_for(Some(key), &base, Some(workspace)).ok()
    }

    /// Re-read `settings.json`. A missing file means defaults.
    pub fn reload(&mut self) -> ReloadOutcome {
        let user = match read_or_empty(&self.paths.settings) {
            Ok(t) => t,
            Err(e) => {
                self.last_error = Some(e.clone());
                return ReloadOutcome::Rejected(e);
            }
        };
        let layers = [
            default_layer(),
            Layer {
                path: &self.paths.settings,
                text: &user,
            },
        ];
        let warnings = crate::unknown_keys(&user);
        let warned = std::mem::replace(&mut self.warnings, warnings) != self.warnings;
        match parse_layers(&layers) {
            Ok(next) => {
                let changed = next != self.current || self.last_error.is_some() || warned;
                self.current = next;
                self.last_error = None;
                if changed {
                    self.generation += 1;
                    ReloadOutcome::Changed
                } else {
                    ReloadOutcome::Unchanged
                }
            }
            Err(e) => {
                self.last_error = Some(e.clone());
                ReloadOutcome::Rejected(e)
            }
        }
    }
}
