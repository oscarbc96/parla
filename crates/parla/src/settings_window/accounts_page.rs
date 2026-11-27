//! The Workspaces and privacy page of the Settings window (the
//! SettingsAccounts board): each signed-in workspace with its state, Sign in
//! again and Sign out; the local store's limits and Clear cache; and Delete
//! all data in a danger zone. Sign out, Clear cache and Delete all data ask
//! first. The store limits are read when Parla starts, so they say so. The
//! Privacy section sets Lock store when idle and the crash report mode;
//! both apply at once.

use components::controls::{ButtonKind, button, compact};
use components::settings_controls::{segment, segmented_group, settings_row, status_pill};
use domain::Session;
use gpui::{Context, div, prelude::*};
use settings::CrashReports;
use themes::{ActiveTheme as _, metrics};

use super::{SettingChange, SettingsView, json_text, owned};

/// Steps of the cache limit, in MB.
pub const CACHE_MB_STEPS: &[u32] = &[4, 8, 16, 32, 64, 128];
/// Steps of how long messages are kept, in days.
pub const RETENTION_STEPS: &[u32] = &[7, 30, 90, 180, 365];

/// One edit a store control makes.
#[derive(Clone, Debug, PartialEq)]
pub enum StorageChange {
    CacheMb(u32),
    RetentionDays(u32),
}

impl StorageChange {
    pub fn key_path(&self) -> Vec<String> {
        owned(match self {
            Self::CacheMb(_) => &["storage", "cache_mb"],
            Self::RetentionDays(_) => &["storage", "retention_days"],
        })
    }

    pub fn raw_json(&self) -> String {
        match self {
            Self::CacheMb(v) | Self::RetentionDays(v) => json_text(v),
        }
    }
}

impl SettingChange for StorageChange {
    fn path(&self) -> Vec<String> {
        self.key_path()
    }
    fn raw_json(&self) -> String {
        StorageChange::raw_json(self)
    }
}

/// One edit a Privacy control makes.
#[derive(Clone, Debug, PartialEq)]
pub enum PrivacyChange {
    LockAfterMinutes(u32),
    CrashReports(CrashReports),
}

impl PrivacyChange {
    pub fn key_path(&self) -> Vec<String> {
        owned(match self {
            Self::LockAfterMinutes(_) => &["storage", "lock_after_minutes"],
            Self::CrashReports(_) => &["crash_reports"],
        })
    }

    pub fn raw_json(&self) -> String {
        match self {
            Self::LockAfterMinutes(v) => json_text(v),
            Self::CrashReports(v) => json_text(v),
        }
    }
}

impl SettingChange for PrivacyChange {
    fn path(&self) -> Vec<String> {
        self.key_path()
    }
    fn raw_json(&self) -> String {
        PrivacyChange::raw_json(self)
    }
}

/// How a workspace's session stands, for its row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Connected,
    Expired,
    /// No session runs (it could not start, or is still starting).
    Stopped,
}

impl SettingsView {
    pub(super) fn accounts_page(
        &mut self,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let subtle = tokens.text_subtle;
        let (success, warning) = (cx.theme().status().success, cx.theme().status().warning);
        let s = self.settings(cx);

        let workspaces = cx
            .try_global::<crate::workspaces::WorkspacesHandle>()
            .map(|h| h.0.clone());
        let mut list = div().flex().flex_col().gap(metrics::SPACE_4);
        let accounts = super::accounts(cx);
        if accounts.is_empty() {
            list = list.child(
                div()
                    .text_size(metrics::FONT_SIZE_SM)
                    .text_color(colors.text_muted)
                    .child(t!(l, "settings-accounts-none").into_owned()),
            );
        }
        for (i, account) in accounts.into_iter().enumerate() {
            let state = match workspaces
                .as_ref()
                .and_then(|w| w.read(cx).session(&account.key))
            {
                Some(session) if session.read(cx).state().session == Session::Expired => {
                    State::Expired
                }
                Some(_) => State::Connected,
                None => State::Stopped,
            };
            let (label, soft, text) = match state {
                State::Connected => (
                    t!(l, "settings-accounts-connected").into_owned(),
                    tokens.success_soft,
                    success,
                ),
                State::Expired => (
                    t!(l, "settings-accounts-expired").into_owned(),
                    tokens.warning_soft,
                    warning,
                ),
                State::Stopped => (
                    t!(l, "settings-accounts-stopped").into_owned(),
                    tokens.bg_sunken,
                    colors.text_muted,
                ),
            };
            let mut actions = div().flex().flex_none().gap(metrics::SPACE_3);
            if state != State::Connected {
                let key = account.key.clone();
                let id = format!("settings-account-{i}-signin");
                let sel = id.clone();
                let el = compact(button(
                    id,
                    t!(l, "settings-accounts-sign-in").into_owned(),
                    ButtonKind::Primary,
                    true,
                    &colors,
                ))
                .debug_selector(move || sel.clone());
                actions = actions.child(self.wire(el, cx, move |_, _, cx| {
                    let key = key.clone();
                    super::with_root(cx, move |root, _, cx| root.sign_in_again_for(&key, cx));
                }));
            }
            let key = account.key.clone();
            let name = account.name.to_string();
            let id = format!("settings-account-{i}-signout");
            let sel = id.clone();
            let el = compact(button(
                id,
                t!(l, "settings-accounts-sign-out").into_owned(),
                ButtonKind::Secondary,
                true,
                &colors,
            ))
            .debug_selector(move || sel.clone());
            actions = actions.child(self.wire(el, cx, move |v, window, cx| {
                let l = i18n::gpui::localizer(cx);
                let key = key.clone();
                v.confirm(
                    t!(l, "settings-accounts-sign-out-title", name = name.clone()).into_owned(),
                    t!(l, "settings-accounts-sign-out-detail").into_owned(),
                    t!(l, "settings-accounts-sign-out").into_owned(),
                    window,
                    cx,
                    move |_, _, cx| {
                        super::with_root(cx, move |root, window, cx| {
                            root.sign_out_key(key, window, cx)
                        });
                    },
                );
            }));
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_5)
                    .p(metrics::SPACE_5)
                    .rounded(metrics::RADIUS_MD)
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.surface_background)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(metrics::SPACE_4)
                            .child(
                                div()
                                    .truncate()
                                    .text_size(metrics::FONT_SIZE_SM)
                                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                    .child(account.name.to_string()),
                            )
                            .child({
                                let sel = format!(
                                    "settings-account-{i}-{}",
                                    match state {
                                        State::Connected => "connected",
                                        State::Expired => "expired",
                                        State::Stopped => "stopped",
                                    }
                                );
                                status_pill(label, soft, text).debug_selector(move || sel.clone())
                            }),
                    )
                    .child(actions),
            );
        }

        let cache = self.stepper_control(
            cx,
            "settings-cache-mb",
            s.storage.cache_mb,
            t!(l, "settings-unit-mb").into_owned(),
            (
                t!(l, "settings-step-cache-dec").into_owned(),
                t!(l, "settings-step-cache-inc").into_owned(),
            ),
            CACHE_MB_STEPS,
            |s| s.storage.cache_mb,
            StorageChange::CacheMb,
        );
        let retention = self.stepper_control(
            cx,
            "settings-retention",
            s.storage.retention_days,
            t!(l, "settings-unit-days").into_owned(),
            (
                t!(l, "settings-step-retention-dec").into_owned(),
                t!(l, "settings-step-retention-inc").into_owned(),
            ),
            RETENTION_STEPS,
            |s| s.storage.retention_days,
            StorageChange::RetentionDays,
        );
        let clear = self.wire(
            compact(button(
                "settings-clear-cache",
                t!(l, "settings-store-clear").into_owned(),
                ButtonKind::Secondary,
                true,
                &colors,
            ))
            .debug_selector(|| "settings-clear-cache".into()),
            cx,
            |v, window, cx| {
                let l = i18n::gpui::localizer(cx);
                v.confirm(
                    t!(l, "settings-store-clear-title").into_owned(),
                    t!(l, "settings-store-clear-detail").into_owned(),
                    t!(l, "settings-store-clear").into_owned(),
                    window,
                    cx,
                    |_, _, cx| super::clear_cache(cx),
                );
            },
        );
        let restart = Some(t!(l, "settings-applies-on-restart").into_owned().into());
        let store_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-store-cache").into_owned(),
                restart.clone(),
                cache,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-store-retention").into_owned(),
                restart,
                retention,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-store-clear-label").into_owned(),
                Some(t!(l, "settings-store-clear-help").into_owned().into()),
                clear,
                &colors,
                subtle,
            ));

        let privacy_body = self.privacy_body(l, cx);

        let delete = self.delete_all_button("settings-delete-all", l, cx);
        let danger_body = settings_row(
            t!(l, "settings-danger-delete").into_owned(),
            Some(t!(l, "settings-danger-delete-help").into_owned().into()),
            delete,
            &colors,
            subtle,
        );

        div()
            .flex()
            .flex_col()
            .child(Self::page_header(
                t!(l, "settings-accounts-title").into_owned(),
                Some(t!(l, "settings-accounts-subtitle").into_owned()),
                &colors,
            ))
            .child(div().h(metrics::SPACE_4))
            .child(Self::section(
                l,
                "settings-accounts-list-title",
                "settings-accounts-list-desc",
                list,
                true,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-store-title",
                "settings-store-desc",
                store_body,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-privacy-title",
                "settings-privacy-desc",
                privacy_body,
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

    /// Lock store when idle and Crash reports, as segmented choices.
    fn privacy_body(&mut self, l: &i18n::Localizer, cx: &mut Context<Self>) -> gpui::Div {
        let colors = *cx.theme().colors();
        let subtle = themes::parla_tokens(cx.theme()).text_subtle;
        let s = self.settings(cx);
        let mut lock_segments = Vec::with_capacity(settings::LOCK_AFTER_STEPS.len());
        for &m in settings::LOCK_AFTER_STEPS {
            let (id, label) = match m {
                0 => (
                    "settings-lock-off".to_string(),
                    t!(l, "settings-lock-off").into_owned(),
                ),
                60 => (
                    "settings-lock-60".to_string(),
                    t!(l, "settings-lock-hour").into_owned(),
                ),
                n => (
                    format!("settings-lock-{n}"),
                    t!(l, "settings-lock-minutes", n = n).into_owned(),
                ),
            };
            let el = segment(id, label, s.storage.lock_after_minutes == m, &colors);
            lock_segments.push(self.wire(el, cx, move |v, _, cx| {
                v.apply(PrivacyChange::LockAfterMinutes(m), cx)
            }));
        }
        let mut crash_segments = Vec::with_capacity(3);
        for (mode, id, label) in [
            (
                CrashReports::Off,
                "settings-crash-off",
                t!(l, "settings-crash-off").into_owned(),
            ),
            (
                CrashReports::Ask,
                "settings-crash-ask",
                t!(l, "settings-crash-ask").into_owned(),
            ),
            (
                CrashReports::Local,
                "settings-crash-local",
                t!(l, "settings-crash-local").into_owned(),
            ),
        ] {
            let el = segment(id, label, s.crash_reports == mode, &colors);
            crash_segments.push(self.wire(el, cx, move |v, _, cx| {
                v.apply(PrivacyChange::CrashReports(mode), cx)
            }));
        }
        div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_4)
            .child(settings_row(
                t!(l, "settings-lock-idle").into_owned(),
                Some(t!(l, "settings-lock-idle-help").into_owned().into()),
                segmented_group(
                    "settings-lock-group",
                    t!(l, "settings-lock-idle").into_owned(),
                    lock_segments,
                    &colors,
                ),
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-crash").into_owned(),
                Some(t!(l, "settings-crash-help").into_owned().into()),
                segmented_group(
                    "settings-crash-group",
                    t!(l, "settings-crash").into_owned(),
                    crash_segments,
                    &colors,
                ),
                &colors,
                subtle,
            ))
    }
}
