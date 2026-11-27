//! App-level actions. Names match the default keymaps (`app::Quit`, `theme::ToggleMode`, ...).

use gpui::actions;
use serde::Serialize;
use std::collections::BTreeMap;

actions!(
    app,
    [
        Quit,
        OpenSettings,
        OpenSettingsFile,
        OpenKeymap,
        ReloadConfig,
        DeleteAllData,
        ShowCrashReports,
        CheckForUpdates,
        About,
        CommandPalette,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        CloseWindow,
        BringAllToFront,
        OpenDocs,
        OpenKeyboardShortcuts,
        ReportIssue
    ]
);
actions!(theme, [ToggleMode]);
actions!(call, [ToggleMute, Leave, ShowWindow]);
actions!(store, [RebuildCache]);
actions!(mcp, [RebuildAuditLog, OpenAuditLog]);
actions!(auth, [SignInAgain, SignOut]);
actions!(
    workspace,
    [
        RemoveWorkspace,
        PauseNotifications,
        SetStatus,
        FocusNextPane,
        FocusPreviousPane,
        PopOutConversation,
        PopOutThread
    ]
);

/// Every action Parla knows how to bind.
pub const KNOWN: &[&str] = &[
    "app::Quit",
    "app::OpenSettings",
    "app::OpenSettingsFile",
    "app::OpenKeymap",
    "app::ReloadConfig",
    "app::DeleteAllData",
    "app::ShowCrashReports",
    "app::CheckForUpdates",
    "app::About",
    "app::CommandPalette",
    "app::Hide",
    "app::HideOthers",
    "app::ShowAll",
    "app::Minimize",
    "app::Zoom",
    "app::CloseWindow",
    "app::BringAllToFront",
    "app::OpenDocs",
    "app::OpenKeyboardShortcuts",
    "app::ReportIssue",
    "workspace::RemoveWorkspace",
    "workspace::PauseNotifications",
    "workspace::SetStatus",
    "workspace::FocusNextPane",
    "workspace::FocusPreviousPane",
    "workspace::PopOutConversation",
    "workspace::PopOutThread",
    "workspace::NextWorkspace",
    "workspace::PreviousWorkspace",
    "workspace::ActivateWorkspace1",
    "workspace::ActivateWorkspace2",
    "workspace::ActivateWorkspace3",
    "workspace::ActivateWorkspace4",
    "workspace::ActivateWorkspace5",
    "workspace::ActivateWorkspace6",
    "workspace::ActivateWorkspace7",
    "workspace::ActivateWorkspace8",
    "workspace::ActivateWorkspace9",
    "theme::ToggleMode",
    "call::ToggleMute",
    "call::Leave",
    "call::ShowWindow",
    "auth::SignInAgain",
    "auth::SignOut",
    "sidebar::SelectNext",
    "sidebar::SelectPrev",
    "sidebar::Open",
    "sidebar::NextUnread",
    "timeline::MarkUnread",
    "timeline::JumpToLatest",
    "timeline::TogglePin",
    "timeline::ToggleSaved",
    "timeline::CopyLink",
    "workspace::Search",
    "workspace::QuickSwitch",
    "workspace::GoBack",
    "workspace::GoForward",
    "workspace::AllUnreads",
    "workspace::AllMentions",
    "workspace::AllScheduled",
    "store::RebuildCache",
    "mcp::RebuildAuditLog",
    "mcp::OpenAuditLog",
    "mcp::ClearAuditLog",
    "mcp::ApproveWrite",
    "mcp::ApproveWriteForSession",
    "mcp::DeclineWrite",
    "timeline::EditMessage",
    "timeline::DeleteMessage",
    "composer::Send",
    "composer::Newline",
    "composer::PlainEnter",
    "composer::EditLast",
    "composer::Cancel",
    "composer::ToggleBroadcast",
    "composer::SendLater",
    "composer::Paste",
    "composer::AcceptCompletion",
    "composer::Bold",
    "composer::Italic",
    "composer::Strike",
    "composer::Code",
    "composer::CodeBlock",
    "composer::Quote",
    "composer::BulletList",
    "composer::NumberedList",
    "composer::Link",
    "text_input::Left",
    "text_input::Right",
    "text_input::Up",
    "text_input::Down",
    "text_input::WordLeft",
    "text_input::WordRight",
    "text_input::LineStart",
    "text_input::LineEnd",
    "text_input::DocStart",
    "text_input::DocEnd",
    "text_input::SelectLeft",
    "text_input::SelectRight",
    "text_input::SelectUp",
    "text_input::SelectDown",
    "text_input::SelectWordLeft",
    "text_input::SelectWordRight",
    "text_input::SelectLineStart",
    "text_input::SelectLineEnd",
    "text_input::SelectDocStart",
    "text_input::SelectDocEnd",
    "text_input::SelectAll",
    "text_input::Backspace",
    "text_input::Delete",
    "text_input::DeleteWordLeft",
    "text_input::Cut",
    "text_input::Copy",
    "text_input::Paste",
    "text_input::Undo",
    "text_input::Redo",
    "timeline::AddReaction",
    "timeline::OpenThread",
    "workspace::ClosePanel",
    "picker::Confirm",
    "picker::Dismiss",
    "picker::SelectLeft",
    "picker::SelectNext",
    "picker::SelectPrev",
    "picker::SelectRight",
];

const DEFAULT_MACOS_KEYMAP: &str = include_str!("../../../assets/keymaps/default-macos.json");
const DEFAULT_LINUX_KEYMAP: &str = include_str!("../../../assets/keymaps/default-linux.json");
const DEFAULT_WINDOWS_KEYMAP: &str = include_str!("../../../assets/keymaps/default-windows.json");

#[derive(Debug, Clone, Serialize)]
pub struct DefaultBinding {
    pub context: Option<String>,
    pub keys: String,
}

#[derive(Debug, Serialize)]
pub struct PlatformDefaults {
    pub linux: Vec<DefaultBinding>,
    pub macos: Vec<DefaultBinding>,
    pub windows: Vec<DefaultBinding>,
}

#[derive(Debug, Serialize)]
pub struct ActionEntry {
    pub action: String,
    pub defaults: PlatformDefaults,
}

/// Actions the platform implements only on macOS (Hide, Hide others, Show
/// all). Elsewhere GPUI leaves some of them `unimplemented!()`, so they have no
/// handler and the command palette does not list them.
pub const MAC_ONLY: &[&str] = &["app::Hide", "app::HideOthers", "app::ShowAll"];

/// Whether `name` is an action of another platform (see [`MAC_ONLY`]).
pub fn unsupported_here(name: &str, mac: bool) -> bool {
    !mac && MAC_ONLY.contains(&name)
}

/// JSON of every registered action with default keys per platform.
pub fn print_actions_json() -> String {
    let mut actions: Vec<&str> = KNOWN.to_vec();
    actions.sort_unstable();
    actions.dedup();

    let macos = bindings_by_action(DEFAULT_MACOS_KEYMAP);
    let linux = bindings_by_action(DEFAULT_LINUX_KEYMAP);
    let windows = bindings_by_action(DEFAULT_WINDOWS_KEYMAP);

    let entries: Vec<ActionEntry> = actions
        .into_iter()
        .map(|action| ActionEntry {
            action: action.to_string(),
            defaults: PlatformDefaults {
                linux: linux.get(action).cloned().unwrap_or_default(),
                macos: macos.get(action).cloned().unwrap_or_default(),
                windows: windows.get(action).cloned().unwrap_or_default(),
            },
        })
        .collect();

    serde_json::to_string_pretty(&entries).expect("actions json") + "\n"
}

fn bindings_by_action(text: &str) -> BTreeMap<String, Vec<DefaultBinding>> {
    let blocks = settings::parse_keymap(std::path::Path::new("keymap.json"), text)
        .expect("valid default keymap");
    let mut map: BTreeMap<String, Vec<DefaultBinding>> = BTreeMap::new();
    for block in blocks {
        for (keys, action) in block.bindings {
            map.entry(action).or_default().push(DefaultBinding {
                context: block.context.clone(),
                keys,
            });
        }
    }
    for bindings in map.values_mut() {
        bindings.sort_by(|a, b| a.context.cmp(&b.context).then(a.keys.cmp(&b.keys)));
    }
    map
}

/// Whether `name` is an action Parla knows how to bind.
pub fn is_known(name: &str) -> bool {
    KNOWN.contains(&name)
}

/// `app::OpenSettings` -> `action-app-open-settings`.
pub fn title_id(name: &str) -> String {
    let mut id = String::from("action-");
    for (i, part) in name.split("::").enumerate() {
        if i > 0 {
            id.push('-');
        }
        for (j, c) in part.chars().enumerate() {
            if c.is_uppercase() {
                if j > 0 {
                    id.push('-');
                }
                id.extend(c.to_lowercase());
            } else if c == '_' {
                id.push('-');
            } else {
                id.push(c);
            }
        }
    }
    id
}

/// The title of action `name` in the active language (command palette, keymap
/// docs), looked up in the app catalog first and then in the components one.
/// On macOS an `-mac` variant of the id wins when it exists (the platform modifier).
pub fn title(l: &i18n::Localizer, name: &str) -> Option<String> {
    let plain = title_id(name);
    let mac = format!("{plain}-mac");
    let ids = if cfg!(target_os = "macos") {
        vec![mac, plain]
    } else {
        vec![plain]
    };
    ids.iter().find_map(|id| {
        ["parla", "components"]
            .into_iter()
            .find(|ns| l.has(ns, id))
            .map(|ns| l.format(ns, id, None).into_owned())
    })
}

// The command palette lists every palette action (see
// `components::palette_action_names`), so actions that are not bound in a
// default keymap are still reachable from the keyboard.

#[cfg(test)]
mod tests {
    use super::title_id;

    #[test]
    fn ids_are_kebab_case() {
        assert_eq!(title_id("app::OpenSettings"), "action-app-open-settings");
        assert_eq!(title_id("picker::SelectLeft"), "action-picker-select-left");
        assert_eq!(title_id("app::Quit"), "action-app-quit");
        assert_eq!(
            title_id("text_input::SelectWordLeft"),
            "action-text-input-select-word-left"
        );
    }
}
