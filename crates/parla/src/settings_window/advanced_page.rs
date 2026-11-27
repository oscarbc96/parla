//! The Advanced page of the Settings window (the SettingsAdvanced board): the
//! memory bounds (image cache, messages kept per conversation, lean mode
//! delay) and a danger zone with Reset all settings and Delete local data,
//! each behind a confirmation, and Logs and diagnostics: the log level, Open
//! logs folder and a redacted Export diagnostics. The message window, the
//! lean delay and the log level apply at once; the image cache the next time
//! Parla starts.

use components::controls::{ButtonKind, button, compact, on_activate_app};
use components::settings_controls::{segment, segmented, settings_row};
use gpui::{Context, div, prelude::*};
use settings::LogLevel;
use themes::{ActiveTheme as _, metrics};

use super::{SettingChange, SettingsView, json_text, owned};

/// Steps of the image cache, in MB.
pub const IMAGE_CACHE_STEPS: &[u32] = &[16, 32, 64, 128, 256];
/// Steps of the messages kept per conversation.
pub const MESSAGE_WINDOW_STEPS: &[u32] = &[100, 200, 500, 1000];
/// Steps of the lean mode delay, in minutes.
pub const LEAN_STEPS: &[u32] = &[1, 5, 10, 30, 60];

/// One edit an Advanced page control makes.
#[derive(Clone, Debug, PartialEq)]
pub enum AdvancedChange {
    ImageCacheMb(u32),
    MessageWindow(u32),
    LeanAfterMinutes(u32),
    LogLevel(LogLevel),
}

impl AdvancedChange {
    pub fn key_path(&self) -> Vec<String> {
        owned(match self {
            Self::ImageCacheMb(_) => &["performance", "image_cache_mb"],
            Self::MessageWindow(_) => &["performance", "message_window"],
            Self::LeanAfterMinutes(_) => &["performance", "lean_after_minutes"],
            Self::LogLevel(_) => &["log", "level"],
        })
    }

    pub fn raw_json(&self) -> String {
        match self {
            Self::ImageCacheMb(v) | Self::MessageWindow(v) | Self::LeanAfterMinutes(v) => {
                json_text(v)
            }
            Self::LogLevel(v) => json_text(v),
        }
    }
}

impl SettingChange for AdvancedChange {
    fn path(&self) -> Vec<String> {
        self.key_path()
    }
    fn raw_json(&self) -> String {
        AdvancedChange::raw_json(self)
    }
}

impl SettingsView {
    pub(super) fn advanced_page(
        &mut self,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let subtle = themes::parla_tokens(cx.theme()).text_subtle;
        let p = self.settings(cx).performance;

        let image = self.stepper_control(
            cx,
            "settings-image-cache",
            p.image_cache_mb,
            t!(l, "settings-unit-mb").into_owned(),
            (
                t!(l, "settings-step-image-cache-dec").into_owned(),
                t!(l, "settings-step-image-cache-inc").into_owned(),
            ),
            IMAGE_CACHE_STEPS,
            |s| s.performance.image_cache_mb,
            AdvancedChange::ImageCacheMb,
        );
        let window = self.stepper_control(
            cx,
            "settings-message-window",
            p.message_window,
            t!(l, "settings-unit-messages").into_owned(),
            (
                t!(l, "settings-step-message-window-dec").into_owned(),
                t!(l, "settings-step-message-window-inc").into_owned(),
            ),
            MESSAGE_WINDOW_STEPS,
            |s| s.performance.message_window,
            AdvancedChange::MessageWindow,
        );
        let lean = self.stepper_control(
            cx,
            "settings-lean",
            p.lean_after_minutes,
            t!(l, "settings-unit-minutes").into_owned(),
            (
                t!(l, "settings-step-lean-dec").into_owned(),
                t!(l, "settings-step-lean-inc").into_owned(),
            ),
            LEAN_STEPS,
            |s| s.performance.lean_after_minutes,
            AdvancedChange::LeanAfterMinutes,
        );
        let memory_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_4)
            .child(settings_row(
                t!(l, "settings-advanced-image-cache").into_owned(),
                Some(
                    t!(l, "settings-advanced-image-cache-help")
                        .into_owned()
                        .into(),
                ),
                image,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-advanced-message-window").into_owned(),
                None,
                window,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-advanced-lean").into_owned(),
                None,
                lean,
                &colors,
                subtle,
            ));

        let level = self.settings(cx).log.level;
        let level_seg =
            |this: &Self, cx: &mut Context<Self>, id: &'static str, label: String, v: LogLevel| {
                let el = segment(id, label, level == v, &colors);
                this.wire(el, cx, move |view, _, cx| {
                    view.apply(AdvancedChange::LogLevel(v), cx)
                })
            };
        let level_control = segmented(
            [
                level_seg(
                    self,
                    cx,
                    "settings-log-error",
                    t!(l, "settings-log-error").into_owned(),
                    LogLevel::Error,
                ),
                level_seg(
                    self,
                    cx,
                    "settings-log-info",
                    t!(l, "settings-log-info").into_owned(),
                    LogLevel::Info,
                ),
                level_seg(
                    self,
                    cx,
                    "settings-log-debug",
                    t!(l, "settings-log-debug").into_owned(),
                    LogLevel::Debug,
                ),
            ],
            &colors,
        );
        let open_logs = on_activate_app(
            compact(button(
                "settings-open-logs",
                t!(l, "settings-open-logs").into_owned(),
                ButtonKind::Secondary,
                true,
                &colors,
            ))
            .debug_selector(|| "settings-open-logs".into()),
            |_, cx| cx.reveal_path(&crate::log::path_in(&settings::paths::data_dir())),
        );
        let export = self.wire(
            compact(button(
                "settings-export-diagnostics",
                t!(l, "settings-export-diagnostics").into_owned(),
                ButtonKind::Secondary,
                true,
                &colors,
            ))
            .debug_selector(|| "settings-export-diagnostics".into()),
            cx,
            |v, _, cx| v.export_diagnostics(cx),
        );
        let logs_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_4)
            .child(settings_row(
                t!(l, "settings-log-level").into_owned(),
                Some(t!(l, "settings-log-level-help").into_owned().into()),
                level_control,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-open-logs-label").into_owned(),
                Some(t!(l, "settings-open-logs-help").into_owned().into()),
                open_logs,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-export-label").into_owned(),
                Some(t!(l, "settings-export-help").into_owned().into()),
                export,
                &colors,
                subtle,
            ));

        let reset = self.danger_button(
            "settings-reset-settings",
            t!(l, "settings-danger-reset").into_owned(),
            cx,
            |v, window, cx| {
                let l = i18n::gpui::localizer(cx);
                v.confirm(
                    t!(l, "settings-danger-reset-title").into_owned(),
                    t!(l, "settings-danger-reset-detail").into_owned(),
                    t!(l, "settings-danger-reset").into_owned(),
                    window,
                    cx,
                    |v, _, cx| v.reset_settings(cx),
                );
            },
        );
        let delete = self.delete_all_button("settings-advanced-delete-all", l, cx);
        let danger_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-danger-reset-label").into_owned(),
                Some(t!(l, "settings-danger-reset-help").into_owned().into()),
                reset,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-danger-delete").into_owned(),
                Some(t!(l, "settings-danger-delete-help").into_owned().into()),
                delete,
                &colors,
                subtle,
            ));

        div()
            .flex()
            .flex_col()
            .child(Self::page_header(
                t!(l, "settings-advanced-title").into_owned(),
                Some(t!(l, "settings-advanced-subtitle").into_owned()),
                &colors,
            ))
            .child(div().h(metrics::SPACE_4))
            .child(Self::section(
                l,
                "settings-memory-title",
                "settings-memory-desc",
                memory_body,
                true,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-logs-title",
                "settings-logs-desc",
                logs_body,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-danger-title",
                "settings-danger-desc",
                danger_body,
                false,
                cx,
            ))
            .pb(metrics::SPACE_8)
    }

    /// Writes the redacted diagnostics file next to the log on the background
    /// executor, then shows it.
    fn export_diagnostics(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings(cx);
        let now = components::now_secs(cx);
        let dir = settings::paths::data_dir();
        let job = cx.background_executor().spawn(async move {
            crate::diagnostics::export(&dir, &settings, now, crate::providers::redaction_rules())
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |v, cx| {
                match result {
                    Ok(path) => {
                        v.error = None;
                        cx.reveal_path(&path);
                    }
                    Err(e) => {
                        crate::log::error(&format!("settings: cannot export diagnostics: {e}"));
                        let l = i18n::gpui::localizer(cx);
                        v.error = Some(
                            t!(l, "settings-error-export", reason = e.to_string()).into_owned(),
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Writes the commented template over `settings.json` (themes and the
    /// keymap stay); the watcher reloads it like any edit.
    fn reset_settings(&mut self, cx: &mut Context<Self>) {
        match settings::reset_file(&self.path) {
            Ok(()) => {
                self.error = None;
                self.pending.clear();
            }
            Err(e) => {
                crate::log::error(&format!("settings: cannot reset: {e}"));
                let l = i18n::gpui::localizer(cx);
                self.error =
                    Some(t!(l, "settings-error-write", reason = e.to_string()).into_owned());
            }
        }
        cx.notify();
    }
}
