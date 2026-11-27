//! The sidebar's own sections and settings popover, for a workspace session:
//! custom sections live under `sidebar.sections` in the workspace's settings
//! file (`workspaces/<provider>/<id>.json`; local, never sent to the
//! provider), the popover's options in `settings.json` like the Appearance
//! page. Both apply at once through the settings reload.

use std::path::Path;

use components::{SectionOp, SidebarOption, apply_section_op};
use gpui::Context;
use settings::{ChangedFile, ConfigPaths, CustomSection, WorkspaceKey};

use crate::session::WorkspaceSession;

/// Writes `sections` under `sidebar.sections` in the workspace's settings
/// file, creating the file and its folder when missing. False when the file
/// is not a JSON object (nothing is written).
pub fn write_sections(
    paths: &ConfigPaths,
    key: &WorkspaceKey,
    sections: &[CustomSection],
) -> std::io::Result<bool> {
    let file = paths.workspace_file(key);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string(sections).map_err(std::io::Error::other)?;
    settings::edit_file(&file, &["sidebar", "sections"], &json)
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

/// The key path and value a popover option writes in `settings.json`.
pub fn option_edit(option: SidebarOption) -> (&'static [&'static str], String) {
    match option {
        SidebarOption::ShowUnreads(on) => (&["sidebar", "show_unreads_section"], json(&on)),
        SidebarOption::Sort(sort) => (&["sidebar", "sort"], json(&sort)),
        SidebarOption::Density(d) => (&["density"], json(&d)),
    }
}

fn write_option(file: &Path, option: SidebarOption) -> std::io::Result<bool> {
    let (path, value) = option_edit(option);
    settings::edit_file(file, path, &value)
}

impl WorkspaceSession {
    /// The user's own sections of this workspace, in order: id and name.
    pub fn custom_sections(&self, cx: &gpui::App) -> Vec<(u32, String)> {
        self.settings(cx)
            .sidebar
            .sections
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect()
    }

    /// The custom section holding `conversation`, if any.
    pub fn section_of(&self, conversation: &domain::ConversationId, cx: &gpui::App) -> Option<u32> {
        components::section_of(&self.settings(cx).sidebar.sections, &conversation.0)
    }

    /// Applies a section edit and saves it in the workspace's file; the
    /// reload shows it. Starts from that file's own sections only, so
    /// sections set in `settings.json` are never copied into it. A deleted
    /// section's folded state goes with it, so a later section that gets the
    /// same id opens unfolded.
    pub fn section_op_now(&mut self, op: SectionOp, cx: &mut Context<Self>) {
        let Some(paths) = crate::app_state::config_paths(cx) else {
            return;
        };
        let key = self.state().workspace.clone();
        let deleted = match op {
            SectionOp::Delete { id } => Some(id),
            _ => None,
        };
        let mut sections = workspace_sections(&paths, &key);
        if !apply_section_op(&mut sections, op) {
            return;
        }
        if let Some(id) = deleted {
            self.forget_section_fold(id, cx);
        }
        match write_sections(&paths, &key, &sections) {
            // Outside this session's update: the reload notifies every view.
            Ok(true) => {
                cx.defer(move |cx| crate::app_state::handle_change(cx, ChangedFile::Workspace(key)))
            }
            Ok(false) => crate::log::error("sidebar sections: the workspace file is not an object"),
            Err(e) => crate::log::error(&format!("sidebar sections: cannot write: {e}")),
        }
    }

    /// The popover's show Unreads and sort as this workspace uses them (its
    /// own file over `settings.json`).
    pub fn sidebar_options(&self, cx: &gpui::App) -> (bool, settings::SidebarSort) {
        let s = &self.settings(cx).sidebar;
        (s.show_unreads_section, s.sort)
    }

    /// Saves a popover option where it takes effect: the workspace's file
    /// when that file sets the key, else `settings.json` (density is always
    /// app-wide); the reload applies it.
    pub fn sidebar_option_now(&mut self, option: SidebarOption, cx: &mut Context<Self>) {
        let Some(paths) = crate::app_state::config_paths(cx) else {
            return;
        };
        let key = self.state().workspace.clone();
        let ws_file = paths.workspace_file(&key);
        let (path, _) = option_edit(option);
        let in_workspace = !matches!(option, SidebarOption::Density(_))
            && settings::layer_value(&ws_file, path).is_some();
        let (file, changed) = if in_workspace {
            (ws_file, ChangedFile::Workspace(key))
        } else {
            (paths.settings.clone(), ChangedFile::Settings)
        };
        match write_option(&file, option) {
            Ok(true) => cx.defer(move |cx| crate::app_state::handle_change(cx, changed)),
            Ok(false) => crate::log::error("sidebar settings: the settings file is not an object"),
            Err(e) => crate::log::error(&format!("sidebar settings: cannot write: {e}")),
        }
    }
}

/// The sections the workspace's own file holds (none when it sets none or
/// cannot be read).
pub fn workspace_sections(paths: &ConfigPaths, key: &WorkspaceKey) -> Vec<CustomSection> {
    settings::layer_value(&paths.workspace_file(key), &["sidebar", "sections"])
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}
