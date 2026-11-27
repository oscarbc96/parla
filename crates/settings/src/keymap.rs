//! Zed-style keymap files: JSONC array of `{context, bindings}` blocks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::SettingsError;

/// One keymap block as written in a keymap file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KeymapBlock {
    #[serde(default)]
    pub context: Option<String>,
    /// Keys to action names. A `null` action (read as `""`) unbinds a
    /// default binding of the same keys and context.
    #[serde(default, deserialize_with = "bindings_or_null")]
    pub bindings: BTreeMap<String, String>,
}

fn bindings_or_null<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    let raw = BTreeMap::<String, Option<String>>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|(k, v)| (k, v.unwrap_or_default()))
        .collect())
}

/// A resolved, valid binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub keystrokes: String,
    pub action: String,
    pub context: Option<String>,
}

/// A binding that was dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapWarning {
    BadKeystroke {
        keystrokes: String,
        context: Option<String>,
    },
    UnknownAction {
        action: String,
    },
}

/// Parse a keymap file (JSONC). An empty file is an empty keymap.
pub fn parse_keymap(path: &Path, text: &str) -> Result<Vec<KeymapBlock>, SettingsError> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value = crate::load::parse_jsonc::<serde_json::Value>(text).map_err(|e| {
        let (line, col, message) = crate::load::split_position(&e);
        SettingsError::Parse {
            path: path.to_path_buf(),
            line,
            col,
            message,
        }
    })?;
    serde_path_to_error::deserialize(value).map_err(|e| SettingsError::Type {
        path: PathBuf::from(path),
        pointer: crate::load::to_pointer(e.path()),
        message: e.inner().to_string(),
    })
}

/// The OS whose keystroke syntax a keymap file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapOS {
    Mac,
    Windows,
    Linux,
}

impl KeymapOS {
    /// The OS named by a shipped keymap file, the running OS for a user keymap.
    pub fn for_path(path: &Path) -> Self {
        match path.file_name().and_then(|name| name.to_str()) {
            Some("default-macos.json") => Self::Mac,
            Some("default-windows.json") => Self::Windows,
            Some("default-linux.json") => Self::Linux,
            _ => Self::running(),
        }
    }

    /// The OS this binary runs on.
    pub fn running() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    #[cfg(feature = "gpui")]
    fn gpui(self) -> gpui::OperatingSystem {
        match self {
            Self::Mac => gpui::OperatingSystem::Mac,
            Self::Windows => gpui::OperatingSystem::Windows,
            Self::Linux => gpui::OperatingSystem::Linux,
        }
    }
}

#[cfg(feature = "gpui")]
fn valid_keystrokes(keystrokes: &str, os: KeymapOS) -> bool {
    let mut any = false;
    for chord in keystrokes.split_whitespace() {
        any = true;
        if gpui::Keystroke::parse_with_os(chord, os.gpui()).is_err() {
            return false;
        }
    }
    any
}

#[cfg(not(feature = "gpui"))]
fn valid_keystrokes(keystrokes: &str, _os: KeymapOS) -> bool {
    !keystrokes.trim().is_empty()
}

/// `keystrokes` with each chord's modifiers in one order, so `shift-cmd-p`
/// and `cmd-shift-p` compare equal.
pub fn canonical_keys(keystrokes: &str) -> String {
    const ORDER: [&str; 5] = ["ctrl", "alt", "shift", "cmd", "fn"];
    keystrokes
        .split_whitespace()
        .map(|chord| {
            let chord = chord.to_lowercase();
            let (mods, key) = match chord.strip_suffix("--") {
                Some(m) => (m.to_string(), "-".to_string()),
                None => match chord.rsplit_once('-') {
                    Some((m, k)) => (m.to_string(), k.to_string()),
                    None => (String::new(), chord.clone()),
                },
            };
            // GPUI reads `super` and `win` as `cmd` (the platform key).
            let mut mods: Vec<&str> = mods
                .split('-')
                .filter(|m| !m.is_empty())
                .map(|m| match m {
                    "super" | "win" => "cmd",
                    "control" => "ctrl",
                    other => other,
                })
                .collect();
            mods.sort_by_key(|m| ORDER.iter().position(|o| o == m).unwrap_or(ORDER.len()));
            mods.dedup();
            mods.into_iter()
                .chain([key.as_str()])
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Merge default and user blocks, validating each against the OS this binary
/// runs on. See [`resolve_bindings_with_os`] for keymaps of another platform.
pub fn resolve_bindings(
    default: &[KeymapBlock],
    user: &[KeymapBlock],
    known_actions: &dyn Fn(&str) -> bool,
) -> (Vec<Binding>, Vec<KeymapWarning>) {
    let os = KeymapOS::running();
    resolve_bindings_with_os(default, user, os, os, known_actions)
}

/// [`resolve_bindings`] with the OS of each keymap file given explicitly.
pub fn resolve_bindings_with_os(
    default: &[KeymapBlock],
    user: &[KeymapBlock],
    default_os: KeymapOS,
    user_os: KeymapOS,
    known_actions: &dyn Fn(&str) -> bool,
) -> (Vec<Binding>, Vec<KeymapWarning>) {
    let mut bindings: Vec<Binding> = Vec::new();
    let mut warnings = Vec::new();
    let blocks = default
        .iter()
        .map(|block| (block, default_os))
        .chain(user.iter().map(|block| (block, user_os)));
    for (block, os) in blocks {
        // Unbindings first, so a null never removes a binding of the same
        // block that only sorts before it.
        let (nulls, binds): (Vec<_>, Vec<_>) =
            block.bindings.iter().partition(|(_, a)| a.is_empty());
        for (keystrokes, action) in nulls.into_iter().chain(binds) {
            if !valid_keystrokes(keystrokes, os) {
                warnings.push(KeymapWarning::BadKeystroke {
                    keystrokes: keystrokes.clone(),
                    context: block.context.clone(),
                });
                continue;
            }
            let canon = canonical_keys(keystrokes);
            if action.is_empty() {
                bindings.retain(|b| {
                    !(canonical_keys(&b.keystrokes) == canon && b.context == block.context)
                });
                continue;
            }
            if !known_actions(action) {
                warnings.push(KeymapWarning::UnknownAction {
                    action: action.clone(),
                });
                continue;
            }
            let binding = Binding {
                keystrokes: keystrokes.clone(),
                action: action.clone(),
                context: block.context.clone(),
            };
            match bindings
                .iter_mut()
                .find(|b| canonical_keys(&b.keystrokes) == canon && b.context == binding.context)
            {
                Some(slot) => *slot = binding,
                None => bindings.push(binding),
            }
        }
    }
    (bindings, warnings)
}
