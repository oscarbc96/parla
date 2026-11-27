//! Guard: the real window uses the user's UI font and its fallbacks, while the
//! visual harness bundles Inter, so a symbol the system fonts lack renders as a
//! striped "missing glyph" block only in the real app. Two classes:
//!
//! - a text-presentation selector (VS15) after an emoji asks for a glyph no
//!   system font has (the bell and lock tofu); UI strings must not use it;
//! - key symbols (modifiers, return, backspace, tab, arrows, check marks and
//!   the single angle quotes) are drawn as icons through `keycap`, never as
//!   text. Only `keycap.rs` and `parla/src/shortcuts.rs` (which maps keymap
//!   keys to caps that `keycap::glyph_row` draws) may write them; catalog
//!   strings may hold them only under the keys that the UI draws through
//!   `glyph_row`.

use std::path::{Path, PathBuf};

fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn ui_strings_never_force_text_presentation() {
    let mut all = Vec::new();
    files(&root().join("components/src"), "rs", &mut all);
    files(&root().join("parla/src"), "rs", &mut all);
    // Built at run time so this file does not match itself.
    let escaped = format!("{}{}", "\\u{", "FE0E}");
    let bad: Vec<_> = all
        .iter()
        .filter(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            text.contains('\u{FE0E}') || text.to_uppercase().contains(&escaped)
        })
        .collect();
    assert!(
        bad.is_empty(),
        "VS15 (U+FE0E) renders as a missing glyph in the real window: {bad:?}"
    );
}

/// Code points of the symbols the system fonts may lack.
const SYMBOLS: [u32; 18] = [
    0x2318, 0x21E7, 0x2325, 0x2303, 0x23CE, 0x232B, 0x21E5, 0x2713, 0x2714, 0x2190, 0x2191, 0x2192,
    0x2193, 0x2039, 0x203A, 0x2196, 0x21A9, 0x21B5,
];

/// Whether `text` names a symbol, as the character or a `\u{..}` escape.
fn names_symbol(text: &str) -> Option<u32> {
    let upper = text.to_uppercase();
    SYMBOLS.iter().copied().find(|&cp| {
        char::from_u32(cp).is_some_and(|c| text.contains(c))
            || upper.contains(&format!("{}{cp:X}}}", "\\U{"))
    })
}

/// Catalog keys (and the crate that holds them) whose value may hold a symbol:
/// each is drawn through `keycap::glyph_row` or `Glyphs`.
const FTL_ALLOWED: [&str; 5] = [
    "composer-key-cmd-enter-mac",
    "composer-key-cmd-enter",
    "settings-send-key-row-desc",
    "settings-send-key-cmd-enter-mac",
    "settings-send-key-cmd-enter",
];

#[test]
fn key_symbols_and_check_marks_are_icons_not_text() {
    let mut all = Vec::new();
    for dir in ["components/src", "parla/src", "calls/src"] {
        files(&root().join(dir), "rs", &mut all);
    }
    let allowed = ["keycap.rs", "shortcuts.rs"];
    let bad: Vec<String> = all
        .iter()
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            !allowed.contains(&name.as_ref())
                && p.parent()
                    .is_some_and(|d| !d.ends_with("parla/src") || name != "shortcuts.rs")
        })
        .filter_map(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            // Doc and line comments may show the symbols.
            let code: String = text
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            names_symbol(&code).map(|cp| format!("{} (U+{cp:04X})", p.display()))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "key symbols tofu in the real window; use keycap constants and glyph_row, or an icon: {bad:?}"
    );
}

#[test]
fn catalog_strings_hold_symbols_only_under_allowed_keys() {
    let mut all = Vec::new();
    for dir in ["components/i18n", "parla/i18n", "calls/i18n"] {
        files(&root().join(dir), "ftl", &mut all);
    }
    assert!(!all.is_empty());
    let mut bad = Vec::new();
    for p in &all {
        for line in std::fs::read_to_string(p).unwrap().lines() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if names_symbol(value).is_some() && !FTL_ALLOWED.contains(&key.trim()) {
                bad.push(format!("{}: {}", p.display(), key.trim()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "catalog symbols must be drawn through glyph_row; add the key to the allow-list: {bad:?}"
    );
}
