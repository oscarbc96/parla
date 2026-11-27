//! Every bindable action with its live binding: the model behind the keyboard
//! shortcuts sheet (`app::OpenKeyboardShortcuts`) and the Settings Keymap page.
//! Rows come from the default keymap plus the user's `keymap.json` (the same
//! merge `bind_keymap` applies), and an edit writes `keymap.json` through
//! `settings::set_user_binding` / `remove_user_bindings`, so comments stay.

use gpui::{App, Global};
use settings::{
    KeymapBlock, canonical_keys, parse_keymap, remove_user_bindings, resolve_bindings,
    set_user_binding,
};

use crate::actions;

/// The default keymap of this platform.
pub const DEFAULT_KEYMAP: &str = if cfg!(target_os = "macos") {
    include_str!("../../../assets/keymaps/default-macos.json")
} else if cfg!(target_os = "windows") {
    include_str!("../../../assets/keymaps/default-windows.json")
} else {
    include_str!("../../../assets/keymaps/default-linux.json")
};

/// Where a binding comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Default,
    User,
}

/// The area an action belongs to, in the order the sheet shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Area {
    Navigation,
    Messages,
    Composer,
    Calls,
    App,
    Text,
}

impl Area {
    pub const ALL: [Area; 6] = [
        Area::Navigation,
        Area::Messages,
        Area::Composer,
        Area::Calls,
        Area::App,
        Area::Text,
    ];

    /// The message id of the area's title.
    pub fn title_id(self) -> &'static str {
        match self {
            Area::Navigation => "shortcuts-area-navigation",
            Area::Messages => "shortcuts-area-messages",
            Area::Composer => "shortcuts-area-composer",
            Area::Calls => "shortcuts-area-calls",
            Area::App => "shortcuts-area-app",
            Area::Text => "shortcuts-area-text",
        }
    }
}

/// One action and one of its bindings (`keystrokes` is `None` when unbound).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub action: String,
    pub keystrokes: Option<String>,
    pub context: Option<String>,
    pub source: Source,
}

/// The area of action `name`, from its namespace.
pub fn area(name: &str) -> Area {
    match name.split("::").next().unwrap_or_default() {
        "workspace" | "sidebar" | "picker" => Area::Navigation,
        "timeline" => Area::Messages,
        "composer" => Area::Composer,
        "text_input" => Area::Text,
        "call" => Area::Calls,
        _ => Area::App,
    }
}

/// The context a new binding of an unbound action goes in.
fn default_context(name: &str) -> Option<String> {
    let ctx = match name.split("::").next().unwrap_or_default() {
        "composer" => "Composer",
        "timeline" => "Timeline",
        "sidebar" => "Sidebar",
        "text_input" => "TextInput",
        "picker" => "Picker",
        _ => return None,
    };
    Some(ctx.to_string())
}

fn blocks(text: &str) -> Vec<KeymapBlock> {
    parse_keymap(std::path::Path::new("keymap.json"), text).unwrap_or_default()
}

/// Every action this platform can bind, one row per live binding (one
/// unbound row for an action without one), grouped by [`Area`].
/// An action whose default bindings the user removed (with `null` or by
/// giving its keys to another action) shows unbound, as a user change.
pub fn shortcuts(default_text: &str, user_text: &str) -> Vec<Shortcut> {
    let user = blocks(user_text);
    let defaults = blocks(default_text);
    let (bindings, _) = resolve_bindings(&defaults, &user, &actions::is_known);
    let mac = cfg!(target_os = "macos");
    let mut rows = Vec::new();
    for name in actions::KNOWN {
        if actions::unsupported_here(name, mac) || rows.iter().any(|r: &Shortcut| r.action == *name)
        {
            continue;
        }
        let mut any = false;
        for b in bindings.iter().filter(|b| b.action == *name) {
            any = true;
            let from_user = user.iter().any(|u| {
                u.context == b.context && u.bindings.get(&b.keystrokes) == Some(&b.action)
            });
            rows.push(Shortcut {
                action: b.action.clone(),
                keystrokes: Some(b.keystrokes.clone()),
                context: b.context.clone(),
                source: if from_user {
                    Source::User
                } else {
                    Source::Default
                },
            });
        }
        if !any {
            let had_default = defaults
                .iter()
                .find(|d| d.bindings.values().any(|a| a == name));
            rows.push(Shortcut {
                action: name.to_string(),
                keystrokes: None,
                context: match had_default {
                    Some(d) => d.context.clone(),
                    None => default_context(name),
                },
                source: if had_default.is_some() {
                    Source::User
                } else {
                    Source::Default
                },
            });
        }
    }
    rows.sort_by_key(|r| area(&r.action));
    rows
}

/// The keys of a binding ("cmd-shift-k") as key caps: glyphs on macOS
/// (⌘ ⇧ K), words elsewhere (Ctrl Shift K). Chords follow one another.
pub fn key_caps(binding: &str, mac: bool) -> Vec<String> {
    let mut caps = Vec::new();
    for chord in binding.split_whitespace() {
        let (mods, key) = if let Some(m) = chord.strip_suffix("--") {
            (m, "-")
        } else {
            chord.rsplit_once('-').unwrap_or(("", chord))
        };
        for m in mods.split('-').filter(|m| !m.is_empty()) {
            caps.push(modifier(m, mac));
        }
        caps.push(key_name(key, mac));
    }
    caps
}

fn capitalised(s: &str) -> String {
    let mut cs = s.chars();
    cs.next()
        .map(|f| f.to_uppercase().chain(cs).collect())
        .unwrap_or_default()
}

fn modifier(m: &str, mac: bool) -> String {
    match (mac, m) {
        (true, "cmd") => "\u{2318}".into(),
        (true, "shift") => "\u{21E7}".into(),
        (true, "alt") => "\u{2325}".into(),
        (true, "ctrl") => "\u{2303}".into(),
        (_, other) => capitalised(other),
    }
}

fn key_name(key: &str, mac: bool) -> String {
    match (mac, key) {
        (_, "up") => "\u{2191}".into(),
        (_, "down") => "\u{2193}".into(),
        (_, "left") => "\u{2190}".into(),
        (_, "right") => "\u{2192}".into(),
        (true, "enter") => "\u{23CE}".into(),
        (true, "backspace") => "\u{232B}".into(),
        (true, "tab") => "\u{21E5}".into(),
        (_, "escape") => "Esc".into(),
        (_, k) if k.chars().count() == 1 => k.to_uppercase(),
        (_, k) => capitalised(k),
    }
}

/// Whether row `s` (titled `title`) matches the filter `query`: by title,
/// action name, keymap keys ("cmd-k") or key caps ("⌘K").
pub fn matches(s: &Shortcut, title: &str, query: &str, mac: bool) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let keys = s.keystrokes.as_deref().unwrap_or_default();
    title.to_lowercase().contains(&q)
        || s.action.to_lowercase().contains(&q)
        || keys.to_lowercase().contains(&q)
        || key_caps(keys, mac).concat().to_lowercase().contains(&q)
}

/// Whether bindings in `a` and `b` can meet: the same context, or one of
/// them global (a context binding shadows a global one while it has focus).
fn contexts_meet(a: Option<&str>, b: Option<&str>) -> bool {
    a == b || a.is_none() || b.is_none()
}

/// The other action `keystrokes` already runs where `row`'s context applies:
/// in the same context (saving replaces it), or a global binding shadowed by
/// a context one, or the other way round.
pub fn conflict<'a>(
    rows: &'a [Shortcut],
    keystrokes: &str,
    row: &Shortcut,
) -> Option<&'a Shortcut> {
    let keys = canonical_keys(keystrokes);
    rows.iter().find(|r| {
        r.action != row.action
            && r.keystrokes.as_deref().map(canonical_keys) == Some(keys.clone())
            && contexts_meet(r.context.as_deref(), row.context.as_deref())
    })
}

/// Whether live row `row` shares its keys with another action's row.
pub fn in_conflict(rows: &[Shortcut], row: &Shortcut) -> bool {
    row.keystrokes
        .as_deref()
        .is_some_and(|k| conflict(rows, k, row).is_some())
}

/// Contexts where the user types text, so a bare key would be swallowed.
const TEXT_CONTEXTS: [&str; 3] = ["Composer", "TextInput", "Picker"];

/// Whether `keystrokes` is a key that types text (a letter, digit, symbol or
/// space, with Shift at most) bound where it would fire while typing: a
/// global binding or a text context. Such a binding is refused.
pub fn needs_modifier(keystrokes: &str, context: Option<&str>) -> bool {
    let typing = context.is_none_or(|c| TEXT_CONTEXTS.contains(&c));
    let Some(first) = keystrokes.split_whitespace().next() else {
        return false;
    };
    let canon = canonical_keys(first);
    let (mods, key) = match canon.strip_suffix("--") {
        Some(m) => (m.to_string(), "-".to_string()),
        None => canon
            .rsplit_once('-')
            .map(|(m, k)| (m.to_string(), k.to_string()))
            .unwrap_or((String::new(), canon.clone())),
    };
    let only_shift = mods.split('-').all(|m| m.is_empty() || m == "shift");
    typing && only_shift && (key.chars().count() == 1 || key == "space")
}

/// The user keymap text with `row`'s action bound to `keystrokes` instead of
/// its current binding (a default one is unbound with `null`).
pub fn rebind(user_text: &str, row: &Shortcut, keystrokes: &str) -> Option<String> {
    let ctx = row.context.as_deref();
    let mut text = user_text.to_string();
    if let Some(old) = row.keystrokes.as_deref() {
        if canonical_keys(old) == canonical_keys(keystrokes) {
            return Some(text);
        }
        text = match row.source {
            Source::Default => set_user_binding(&text, ctx, old, None)?,
            Source::User => remove_user_bindings(&text, &|c, k, a| {
                c == ctx
                    && canonical_keys(k) == canonical_keys(old)
                    && a == Some(row.action.as_str())
            })?,
        };
    }
    set_user_binding(&text, ctx, keystrokes, Some(&row.action))
}

/// The user keymap text without the user's bindings of `row`'s action in its
/// context, nor the `null`s that unbound its defaults there.
pub fn reset(user_text: &str, default_text: &str, row: &Shortcut) -> Option<String> {
    let defaults = blocks(default_text);
    let ctx = row.context.as_deref();
    let action = row.action.as_str();
    let default_key = |k: &str| {
        defaults.iter().any(|d| {
            d.context.as_deref() == ctx
                && d.bindings
                    .iter()
                    .any(|(dk, da)| da == action && canonical_keys(dk) == canonical_keys(k))
        })
    };
    remove_user_bindings(user_text, &|c, k, a| {
        c == ctx
            && match a {
                // Its own bindings, and another action's on its default keys.
                Some(a) => a == action || default_key(k),
                None => default_key(k),
            }
    })
}

/// The user keymap text last bound (set by the keymap reload).
#[derive(Default)]
pub struct LiveKeymap {
    pub user: String,
}

impl Global for LiveKeymap {}

/// The rows of the keymap bound now.
pub fn current(cx: &App) -> Vec<Shortcut> {
    let user = cx
        .try_global::<LiveKeymap>()
        .map(|k| k.user.as_str())
        .unwrap_or_default();
    shortcuts(DEFAULT_KEYMAP, user)
}

/// The sheet's groups: every bound action of `rows`, titled in the active language.
pub fn sheet_groups(
    l: &i18n::Localizer,
    rows: &[Shortcut],
    mac: bool,
) -> Vec<components::shortcuts_sheet::ShortcutGroup> {
    use components::shortcuts_sheet::{ShortcutGroup, ShortcutItem};
    let range = workspace_range(rows);
    Area::ALL
        .into_iter()
        .filter_map(|a| {
            let items: Vec<ShortcutItem> = rows
                .iter()
                .filter(|r| area(&r.action) == a)
                .filter_map(|r| {
                    let keys = r.keystrokes.as_deref()?;
                    if let Some(mods) = &range
                        && let Some(n) = r.action.strip_prefix("workspace::ActivateWorkspace")
                    {
                        if n != "1" {
                            return None;
                        }
                        let mut caps: Vec<gpui::SharedString> = key_caps(&format!("{mods}-1"), mac)
                            .into_iter()
                            .map(Into::into)
                            .collect();
                        caps.pop();
                        caps.push("1\u{2026}9".into());
                        return Some(ShortcutItem {
                            label: t!(l, "shortcuts-workspace-range").into_owned().into(),
                            keystrokes: keys.to_string().into(),
                            keys: caps,
                        });
                    }
                    Some(ShortcutItem {
                        label: actions::title(l, &r.action)
                            .unwrap_or_else(|| r.action.clone())
                            .into(),
                        keystrokes: keys.to_string().into(),
                        keys: key_caps(keys, mac).into_iter().map(Into::into).collect(),
                    })
                })
                .collect();
            (!items.is_empty()).then(|| ShortcutGroup {
                title: l.format("parla", a.title_id(), None).into_owned().into(),
                items,
            })
        })
        .collect()
}

/// The shared modifiers when workspaces 1 to 9 each have exactly one key,
/// the same modifiers plus their number (`cmd-1` ... `cmd-9`), so the sheet
/// shows them as one row.
fn workspace_range(rows: &[Shortcut]) -> Option<String> {
    let mut mods: Option<String> = None;
    for n in 1..=9 {
        let action = format!("workspace::ActivateWorkspace{n}");
        let mut keys = rows.iter().filter(|r| r.action == action);
        let k = canonical_keys(keys.next()?.keystrokes.as_deref()?);
        if keys.next().is_some() {
            return None;
        }
        let m = k.strip_suffix(&format!("-{n}"))?.to_string();
        if mods.get_or_insert_with(|| m.clone()) != &m {
            return None;
        }
    }
    mods
}
