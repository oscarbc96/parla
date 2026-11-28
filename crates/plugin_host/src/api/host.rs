//! `parla:plugin/host`: API version, app info, the instance log, the clock
//! and plugin strings. No capability.

use std::time::{SystemTime, UNIX_EPOCH};

use super::parla::plugin::host::{Host, LogLevel};
use super::parla::plugin::types::{self as error, Appearance, Density};
use super::parla::plugin::types::{AppInfo, Timestamp, Version};
use crate::linker::PluginState;

/// `message` is cut here; longer log calls are truncated.
const MAX_LOG_BYTES: usize = 4 * 1024;

impl Host for PluginState {
    async fn api_version(&mut self) -> Result<Version, error::Error> {
        let (major, minor) = parla_plugin_api::API_VERSION;
        Ok(Version {
            major,
            minor,
            patch: 0,
        })
    }

    async fn get_app_info(&mut self) -> Result<AppInfo, error::Error> {
        let info = self.context().backend.app_info();
        Ok(AppInfo {
            app_version: info.app_version,
            platform: info.platform,
            locale: info.locale,
            appearance: match info.appearance {
                crate::backend::Appearance::Light => Appearance::Light,
                crate::backend::Appearance::Dark => Appearance::Dark,
            },
            density: match info.density {
                crate::backend::Density::Compact => Density::Compact,
                crate::backend::Density::Cozy => Density::Cozy,
            },
        })
    }

    async fn log(&mut self, level: LogLevel, message: String) -> Result<(), error::Error> {
        // `debug` is kept only in dev mode, which the host does not have yet.
        let name = match level {
            LogLevel::Debug => return Ok(()),
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };
        self.log_line(name, truncate(&message, MAX_LOG_BYTES));
        Ok(())
    }

    async fn now(&mut self) -> Result<Timestamp, error::Error> {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Ok(since_epoch.as_millis() as i64)
    }

    async fn t(
        &mut self,
        key: String,
        _args: Vec<(String, String)>,
    ) -> Result<String, error::Error> {
        // TODO(plan M8 C5, plugin i18n §17): resolve the plugin's own Fluent
        // key in the user's locale; the key stands in until then.
        Ok(key)
    }
}

/// `text` cut to `max` bytes on a char boundary.
fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lines_are_cut_on_a_char_boundary() {
        assert_eq!(truncate("abc", 4), "abc");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("aé", 2), "a");
        assert_eq!(truncate("aéb", 2), "a");
        assert_eq!(
            truncate(&"x".repeat(MAX_LOG_BYTES + 1), MAX_LOG_BYTES).len(),
            MAX_LOG_BYTES
        );
    }
}
