//! Config-error banner: the latest settings error and keymap/theme warnings.

use std::path::PathBuf;

use gpui::{Global, SharedString};
use settings::SettingsError;

/// Where a warning comes from; each source keeps its own list so a reload of
/// one file never drops the warnings of another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WarningSource {
    Settings,
    Theme,
    Keymap,
    Disk,
}

/// A crash notice with a folder to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrashNotice {
    pub text: String,
    pub show_label: String,
    pub delete_label: String,
    pub dir: PathBuf,
}

use crate::updates::UpdateNotice;

/// Latest problems with the user's config files. Cleared when the files are fixed.
#[derive(Default)]
pub struct ConfigBanner {
    /// The settings file is invalid; the last good settings stay active.
    pub error: Option<SettingsError>,
    /// Dropped keymap bindings and theme problems, already formatted.
    pub warnings: Vec<String>,
    /// Standing notices from the app (the MCP server's problems), already
    /// formatted; replaced by their owner, not by a config reload.
    pub notices: Vec<(&'static str, String)>,
    /// A crash report notice from the previous run.
    pub crash: Option<CrashNotice>,
    /// An available update.
    pub update: Option<UpdateNotice>,
    /// The user closed the banner; shown again on the next new problem.
    pub dismissed: bool,
    /// The warnings per source; `warnings` is rebuilt from all of them.
    sources: [Vec<String>; 4],
}

impl Global for ConfigBanner {}

impl ConfigBanner {
    /// One line for the banner, `path:line:col: message` for parse errors.
    pub fn line(&self) -> Option<SharedString> {
        if self.dismissed {
            return None;
        }
        if let Some(m) = self.message_line() {
            return Some(m);
        }
        if let Some(u) = &self.update {
            return Some(u.text.clone().into());
        }
        self.crash.as_ref().map(|c| c.text.clone().into())
    }

    /// The text of an error, warning or standing notice (no update, no crash).
    pub fn message_line(&self) -> Option<SharedString> {
        if let Some(e) = &self.error {
            return Some(e.to_string().into());
        }
        self.warnings
            .first()
            .or(self.notices.first().map(|(_, t)| t))
            .map(|w| w.clone().into())
    }

    /// Sets or clears the notice under `key` (its owner's id).
    pub fn set_notice(&mut self, key: &'static str, text: Option<String>) {
        self.notices.retain(|(k, _)| *k != key);
        if let Some(t) = text {
            self.notices.push((key, t));
            self.dismissed = false;
        }
    }

    /// Shows the crash notice with "Show" and "Delete" actions.
    pub fn set_crash_notice(
        &mut self,
        text: String,
        show_label: String,
        delete_label: String,
        dir: PathBuf,
    ) {
        self.crash = Some(CrashNotice {
            text,
            show_label,
            delete_label,
            dir,
        });
        self.dismissed = false;
    }

    /// Shows an available update with "Download" and "Skip this version" actions.
    pub fn set_update_notice(&mut self, notice: UpdateNotice) {
        self.update = Some(notice);
        self.dismissed = false;
    }

    /// Clears the update notice.
    pub fn clear_update_notice(&mut self) {
        self.update = None;
    }

    /// Replaces the settings error. A new error shows the banner again.
    pub(crate) fn set_error(&mut self, error: Option<SettingsError>) {
        if self.error != error {
            self.dismissed = false;
        }
        self.error = error;
    }

    /// Replaces the warnings of `source` and rebuilds the combined list. A
    /// changed set shows the banner again.
    pub(crate) fn set_warnings(&mut self, source: WarningSource, warnings: Vec<String>) {
        let slot = &mut self.sources[source as usize];
        if *slot != warnings {
            self.dismissed = false;
            *slot = warnings;
        }
        self.warnings = self.sources.iter().flatten().cloned().collect();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use settings::SettingsError;

    use super::ConfigBanner;

    #[test]
    fn dismissed_crash_yields_to_error() {
        let mut b = ConfigBanner::default();
        b.set_crash_notice(
            "crashed".into(),
            "Show".into(),
            "Delete".into(),
            PathBuf::from("/tmp/crashes"),
        );
        b.dismissed = true;
        b.set_error(Some(SettingsError::Type {
            path: PathBuf::from("settings.json"),
            pointer: "/x".into(),
            message: "bad".into(),
        }));
        assert_eq!(b.line().as_deref(), Some("settings.json: /x: bad"));
    }
}
