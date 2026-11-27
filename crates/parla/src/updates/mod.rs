//! Daily update checks against GitHub Releases.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use jiff::Timestamp;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
use url::Url;

pub mod download;
pub mod flow;
pub mod install;
pub mod package;
pub mod self_update;
pub use flow::{UpdateFailure, UpdatePhase};
pub use package::UpdatePackage;
pub use self_update::SelfUpdate;

/// An available update with actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateNotice {
    pub text: String,
    pub download_label: String,
    pub skip_label: String,
    pub url: String,
    /// Version tag to store in `updates.skip` (for example `v0.6.0`).
    pub tag: String,
    /// Where the notice is in the Update and Restart flow.
    pub phase: UpdatePhase,
    /// The **Update** button (or **Try again** after a failure); `None` when
    /// this build cannot install updates itself or while the flow runs.
    pub install_label: Option<String>,
    /// The **Restart to update** button, once the update is in place.
    pub restart_label: Option<String>,
}

/// The release the notice offers and how far the flow got.
struct Offer {
    version: semver::Version,
    url: Url,
    package: Option<UpdatePackage>,
    phase: UpdatePhase,
}

pub const RELEASES_API: &str = "https://api.github.com/repos/oscarbc96/parla/releases/latest";
pub const RELEASES_PAGE_PREFIX: &str = "https://github.com/oscarbc96/parla/releases/";

const DAY_SECS: i64 = 24 * 60 * 60;

/// Owner key of the banner notice a manual check leaves behind.
const UPDATE_CHECK_NOTICE: &str = "update-check";

/// One GitHub Releases check.
pub struct UpdateChecker {
    pub base: Url,
    pub current: semver::Version,
    pub state: PathBuf,
    http: reqwest::Client,
    last_attempt: Mutex<Option<i64>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
// One short-lived value per check; boxing would only complicate matching.
#[allow(clippy::large_enum_variant)]
pub enum UpdateStatus {
    UpToDate,
    Available {
        version: semver::Version,
        url: Url,
        /// The signed macOS package, when the release carries one.
        package: Option<UpdatePackage>,
    },
    Failed,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// Unix seconds of the last successful or failed attempt.
    last_attempt: Option<i64>,
}

impl State {
    fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) {
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = settings::write_atomic(path, &bytes);
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Deserialize)]
struct Release {
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

impl UpdateChecker {
    pub fn new(base: Url, current: semver::Version, state: PathBuf) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        let ua = format!("Parla/{current}");
        if let Ok(v) = HeaderValue::from_str(&ua) {
            headers.insert(USER_AGENT, v);
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(10))
            .build()
            .expect("update http client");
        let last_attempt = State::load(&state).last_attempt;
        Self {
            base,
            current,
            state,
            http,
            last_attempt: Mutex::new(last_attempt),
        }
    }

    /// The last attempt: read from `updates.json` once at construction, then
    /// kept in memory (no disk read per `due` call on the main thread).
    fn effective_last_attempt(&self) -> Option<i64> {
        *self.last_attempt.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Unix seconds of the last update check (kept in memory), if any.
    pub fn last_attempt(&self) -> Option<i64> {
        self.effective_last_attempt()
    }

    /// True when no successful or failed attempt happened in the last 24 hours.
    pub fn due(&self, now: Timestamp) -> bool {
        match self.effective_last_attempt() {
            Some(ts) => now.as_second().saturating_sub(ts) >= DAY_SECS,
            None => true,
        }
    }

    /// Seconds until the next due time, or 0 if due now.
    pub fn seconds_until_due(&self, now: Timestamp) -> i64 {
        match self.effective_last_attempt() {
            Some(ts) => {
                let next = ts.saturating_add(DAY_SECS);
                (next - now.as_second()).clamp(0, DAY_SECS)
            }
            None => 0,
        }
    }

    fn record_attempt(&self, now: Timestamp) {
        let ts = now.as_second();
        *self.last_attempt.lock().unwrap_or_else(|e| e.into_inner()) = Some(ts);
        let state = State {
            last_attempt: Some(ts),
        };
        state.save(&self.state);
    }

    pub async fn check(&self, now: Timestamp) -> UpdateStatus {
        let status = self.check_inner().await;
        self.record_attempt(now);
        status
    }

    async fn check_inner(&self) -> UpdateStatus {
        let resp = match self.http.get(self.base.clone()).send().await {
            Ok(r) => r,
            Err(e) => {
                crate::log::debug(&format!("update check failed: {e}"));
                return UpdateStatus::Failed;
            }
        };
        let body = match resp.error_for_status() {
            Ok(r) => match r.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    crate::log::debug(&format!("update check body failed: {e}"));
                    return UpdateStatus::Failed;
                }
            },
            Err(e) => {
                crate::log::debug(&format!("update check status failed: {e}"));
                return UpdateStatus::Failed;
            }
        };
        let release: Release = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                crate::log::debug(&format!("update check parse failed: {e}"));
                return UpdateStatus::Failed;
            }
        };
        Self::classify(&self.current, release)
    }

    fn classify(current: &semver::Version, release: Release) -> UpdateStatus {
        if release.draft || release.prerelease {
            return UpdateStatus::UpToDate;
        }
        if !release.html_url.starts_with(RELEASES_PAGE_PREFIX) {
            return UpdateStatus::UpToDate;
        }
        let tag = release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name);
        let version = match semver::Version::parse(tag) {
            Ok(v) => v,
            Err(_) => return UpdateStatus::UpToDate,
        };
        let url = match Url::parse(&release.html_url) {
            Ok(u) => u,
            Err(_) => return UpdateStatus::UpToDate,
        };
        if &version > current {
            let package = package::package_for(&version, &release.tag_name, &release.assets);
            UpdateStatus::Available {
                version,
                url,
                package,
            }
        } else {
            UpdateStatus::UpToDate
        }
    }
}

/// Whether an available version should be shown given the skipped tag setting.
pub fn should_show_available(version: &semver::Version, skip: Option<&str>) -> bool {
    match skip {
        None => true,
        Some(tag) => {
            let skipped = tag.strip_prefix('v').unwrap_or(tag);
            match semver::Version::parse(skipped) {
                Ok(s) => version > &s,
                Err(_) => true,
            }
        }
    }
}

use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{App, AppContext as _, Context, Global, Task};

use crate::toast::ConfigBanner;

/// Schedules and performs daily update checks.
pub struct UpdateManager {
    checker: Arc<UpdateChecker>,
    timer: Option<Task<()>>,
    /// Set while a check is running, to avoid overlapping checks.
    checking: bool,
    /// What this build can install itself, when anything.
    self_update: Option<Arc<SelfUpdate>>,
    /// The offered release; its phase gates every transition.
    offer: Option<Offer>,
    /// The running download and install; dropping it cancels the job.
    job: Option<Task<()>>,
    /// Counts install attempts: reports of an earlier attempt are ignored.
    attempt: u64,
}

impl Global for UpdateManager {}

impl UpdateManager {
    /// True when a one-shot wait timer is currently armed.
    pub fn has_timer(&self) -> bool {
        self.timer.is_some()
    }

    pub fn install(cx: &mut App, base: Url, state: PathBuf) {
        let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .unwrap_or_else(|_| semver::Version::new(0, 0, 0));
        let checker = Arc::new(UpdateChecker::new(base, current.clone(), state));
        let mgr = cx.new(|_| UpdateManager {
            checker,
            timer: None,
            checking: false,
            self_update: SelfUpdate::detect(&current).map(Arc::new),
            offer: None,
            job: None,
            attempt: 0,
        });
        cx.set_global(UpdateManagerHandle(mgr.clone()));
        mgr.update(cx, |m, cx| m.schedule(cx));
        // "Check for updates" action runs a manual check (always allowed).
        cx.on_action(move |_: &crate::actions::CheckForUpdates, cx| {
            mgr.update(cx, |m, cx| m.manual_check(cx));
        });
    }

    /// Installs using the default endpoint and `<data_dir>/updates.json`.
    pub fn install_default(cx: &mut App) {
        let base = Url::parse(RELEASES_API).expect("releases api url");
        let state = settings::paths::data_dir().join("updates.json");
        Self::install(cx, base, state);
    }

    fn settings_enabled(cx: &App) -> bool {
        cx.try_global::<settings::SettingsStore>()
            .is_none_or(|s| s.current().updates.check)
    }

    fn skipped(cx: &App) -> Option<String> {
        cx.try_global::<settings::SettingsStore>()
            .and_then(|s| s.current().updates.skip.clone())
    }

    /// Schedules the next automatic check: run now if due, else arm one timer.
    pub fn schedule(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        if !Self::settings_enabled(cx) {
            return;
        }
        let now = Timestamp::from_second(components::now_secs(cx)).unwrap_or(Timestamp::UNIX_EPOCH);
        if self.checker.due(now) {
            self.run_check(false, cx);
        } else {
            let wait = self.checker.seconds_until_due(now).max(1) as u64;
            self.arm_timer(wait, cx);
        }
    }

    fn arm_timer(&mut self, secs: u64, cx: &mut Context<Self>) {
        // Replacing the task drops the old one, keeping at most one timer.
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(secs))
                .await;
            let _ = this.update(cx, |m, cx| m.schedule(cx));
        }));
    }

    /// Manual check from the action/menu: always runs.
    pub fn manual_check(&mut self, cx: &mut Context<Self>) {
        self.run_check(true, cx);
    }

    fn run_check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.checking {
            return;
        }
        if !manual && !Self::settings_enabled(cx) {
            return;
        }
        // Re-arm only after a check completes (no timer while checking).
        self.timer = None;
        self.checking = true;
        let checker = self.checker.clone();
        let now = Timestamp::from_second(components::now_secs(cx)).unwrap_or(Timestamp::UNIX_EPOCH);
        let rt = crate::runtime::handle(cx);
        cx.spawn(async move |this, cx| {
            let status = if let Some(rt) = rt {
                rt.spawn(async move { checker.check(now).await })
                    .await
                    .unwrap_or(UpdateStatus::Failed)
            } else {
                checker.check(now).await
            };
            let _ = this.update(cx, |m, cx| {
                m.checking = false;
                m.apply_status(status, manual, cx);
                m.schedule(cx);
            });
        })
        .detach();
    }

    /// Shows the result of a check. While an update downloads, installs or
    /// waits for the restart, a result never replaces its notice.
    pub fn apply_status(&mut self, status: UpdateStatus, manual: bool, cx: &mut Context<Self>) {
        if self.offer.as_ref().is_some_and(|o| o.phase.holds_notice()) {
            return;
        }
        if cx.has_global::<ConfigBanner>() {
            cx.global_mut::<ConfigBanner>()
                .set_notice(UPDATE_CHECK_NOTICE, None);
        }
        match status {
            UpdateStatus::Available {
                version,
                url,
                package,
            } => {
                if !manual && !should_show_available(&version, Self::skipped(cx).as_deref()) {
                    return;
                }
                self.offer = Some(Offer {
                    version,
                    url,
                    package,
                    phase: UpdatePhase::Offer,
                });
                self.publish(cx);
            }
            UpdateStatus::UpToDate | UpdateStatus::Failed => {
                // Automatic checks stay quiet; a manual one answers.
                if !manual || !cx.has_global::<ConfigBanner>() {
                    return;
                }
                let l = i18n::gpui::localizer(cx);
                let text = if matches!(status, UpdateStatus::UpToDate) {
                    t!(l, "update-up-to-date").into_owned()
                } else {
                    t!(l, "update-check-failed").into_owned()
                };
                cx.global_mut::<ConfigBanner>()
                    .set_notice(UPDATE_CHECK_NOTICE, Some(text));
                cx.refresh_windows();
            }
        }
    }

    /// True when **Update** may be offered for the current release.
    fn can_install(&self) -> bool {
        self.self_update.is_some() && self.offer.as_ref().is_some_and(|o| o.package.is_some())
    }

    /// Puts the notice for the current offer and phase in the banner.
    fn publish(&self, cx: &mut Context<Self>) {
        let Some(offer) = &self.offer else {
            return;
        };
        if !cx.has_global::<ConfigBanner>() {
            return;
        }
        let notice = notice_for(
            cx,
            &offer.version,
            &offer.url,
            offer.phase,
            self.can_install(),
        );
        cx.global_mut::<ConfigBanner>().set_update_notice(notice);
        cx.refresh_windows();
    }

    /// The user clicked **Update** (or **Try again**): download, verify and
    /// install. Nothing runs before this call.
    pub fn start_install(&mut self, cx: &mut Context<Self>) {
        let Some(su) = self.self_update.clone() else {
            return;
        };
        let Some(offer) = self.offer.as_mut() else {
            return;
        };
        let Some(pkg) = offer.package.clone() else {
            return;
        };
        if !offer.phase.start() {
            return;
        }
        self.attempt += 1;
        let attempt = self.attempt;
        self.publish(cx);

        let (tx, mut rx) = futures::channel::mpsc::unbounded::<(u64, u64)>();
        // Report only when the whole percentage moves: at most 100 messages.
        let last = std::sync::atomic::AtomicU64::new(u64::MAX);
        let progress = move |received: u64, total: u64| {
            let percent = received
                .min(total)
                .saturating_mul(100)
                .checked_div(total)
                .unwrap_or(0);
            if last.swap(percent, std::sync::atomic::Ordering::Relaxed) != percent {
                let _ = tx.unbounded_send((received, total));
            }
        };
        let rt = crate::runtime::handle(cx);
        let download = {
            let su = su.clone();
            async move {
                su.downloader
                    .download(&pkg, &su.public_key, &progress)
                    .await
            }
        };
        cx.spawn(async move |this, cx| {
            while let Some((received, total)) = rx.next().await {
                if this
                    .update(cx, |m, cx| {
                        m.download_progress(attempt, received, total, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        self.job = Some(cx.spawn(async move |this, cx| {
            let downloaded = match rt {
                Some(rt) => rt
                    .spawn(download)
                    .await
                    .unwrap_or(Err(download::DownloadError::Io)),
                None => download.await,
            };
            let outcome = match downloaded {
                Err(e) => {
                    crate::log::debug(&format!("update download failed: {e:?}"));
                    Err(UpdateFailure::from_download(&e))
                }
                Ok(dmg) => {
                    // Install only when this attempt is still the current one and
                    // the phase really moved to Installing.
                    let started = this
                        .update(cx, |m, cx| {
                            m.attempt == attempt && m.step(UpdatePhase::downloaded, cx)
                        })
                        .unwrap_or(false);
                    if !started {
                        return;
                    }
                    let su = su.clone();
                    let installed = cx
                        .background_executor()
                        .spawn(async move { (su.install)(&dmg) })
                        .await;
                    installed.map_err(|e| {
                        crate::log::debug(&format!("update install failed: {e:?}"));
                        UpdateFailure::from_install(&e)
                    })
                }
            };
            let _ = this.update(cx, |m, cx| {
                if m.attempt != attempt {
                    return;
                }
                match outcome {
                    Ok(()) => m.step(UpdatePhase::installed, cx),
                    Err(f) => m.step(|p| p.fail(f), cx),
                };
            });
        }));
    }

    fn download_progress(
        &mut self,
        attempt: u64,
        received: u64,
        total: u64,
        cx: &mut Context<Self>,
    ) {
        if attempt == self.attempt {
            self.step(|p| p.progress(received, total), cx);
        }
    }

    /// Applies one phase transition and repaints the notice when it moved.
    /// Returns whether the phase moved.
    fn step(&mut self, f: impl FnOnce(&mut UpdatePhase) -> bool, cx: &mut Context<Self>) -> bool {
        let moved = self.offer.as_mut().is_some_and(|o| f(&mut o.phase));
        if moved {
            self.publish(cx);
        }
        moved
    }

    /// The user clicked **Restart to update**: relaunch the installed bundle.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        let ready = self
            .offer
            .as_ref()
            .is_some_and(|o| matches!(o.phase, UpdatePhase::Ready));
        let Some(su) = self.self_update.clone().filter(|_| ready) else {
            return;
        };
        cx.set_restart_path(su.bundle.clone());
        cx.restart();
    }

    /// Forgets the offer (the user skipped this version).
    fn forget_offer(&mut self) {
        if self.offer.as_ref().is_some_and(|o| !o.phase.holds_notice()) {
            self.offer = None;
        }
    }

    pub fn checker(&self) -> &Arc<UpdateChecker> {
        &self.checker
    }
}

/// The notice for `version` in `phase`. `can_install` says whether this build
/// can install updates itself (the **Update** button).
pub fn notice_for(
    cx: &App,
    version: &semver::Version,
    url: &Url,
    phase: UpdatePhase,
    can_install: bool,
) -> UpdateNotice {
    let l = i18n::gpui::localizer(cx);
    let v = version.to_string();
    let text = match phase {
        UpdatePhase::Offer => t!(l, "update-available", version = v),
        UpdatePhase::Downloading { percent } => t!(
            l,
            "update-downloading",
            version = v,
            percent = percent.to_string()
        ),
        UpdatePhase::Installing => t!(l, "update-installing", version = v),
        UpdatePhase::Ready => t!(l, "update-ready", version = v),
        UpdatePhase::Failed(UpdateFailure::Download) => {
            t!(l, "update-download-failed", version = v)
        }
        UpdatePhase::Failed(UpdateFailure::Signature) => {
            t!(l, "update-signature-invalid", version = v)
        }
        UpdatePhase::Failed(UpdateFailure::Install) => {
            t!(l, "update-install-failed", version = v)
        }
    }
    .into_owned();
    let install_label = (can_install && phase.can_start()).then(|| {
        if matches!(phase, UpdatePhase::Failed(_)) {
            t!(l, "update-retry").into_owned()
        } else {
            t!(l, "update-install").into_owned()
        }
    });
    let restart_label =
        matches!(phase, UpdatePhase::Ready).then(|| t!(l, "update-restart").into_owned());
    UpdateNotice {
        text,
        download_label: t!(l, "update-download").into_owned(),
        skip_label: t!(l, "update-skip").into_owned(),
        url: url.to_string(),
        tag: format!("v{version}"),
        phase,
        install_label,
        restart_label,
    }
}

#[derive(Clone)]
pub struct UpdateManagerHandle(pub gpui::Entity<UpdateManager>);

impl Global for UpdateManagerHandle {}

/// Installs the update scheduler (default endpoint). No-op if already installed.
pub fn install(cx: &mut App) {
    if cx.try_global::<UpdateManagerHandle>().is_some() {
        return;
    }
    UpdateManager::install_default(cx);
}

/// Installs with a custom base URL and state path (for tests).
pub fn install_with(cx: &mut App, base: Url, state: PathBuf) {
    if cx.try_global::<UpdateManagerHandle>().is_some() {
        return;
    }
    UpdateManager::install(cx, base, state);
}

/// Called when `updates.check` or `updates.skip` settings change.
pub fn settings_changed(cx: &mut App) {
    if let Some(h) = cx.try_global::<UpdateManagerHandle>() {
        let h = h.0.clone();
        h.update(cx, |m, cx| m.schedule(cx));
    }
}

/// Installs what the app needs to update itself (tests only; the default
/// install detects it from the running bundle).
pub fn set_self_update(cx: &mut App, update: SelfUpdate) {
    if let Some(h) = cx.try_global::<UpdateManagerHandle>() {
        let h = h.0.clone();
        h.update(cx, |m, _| m.self_update = Some(Arc::new(update)));
    }
}

/// The user clicked **Update**: download, verify and install the offer.
pub fn start_install(cx: &mut App) {
    if let Some(h) = cx.try_global::<UpdateManagerHandle>() {
        let h = h.0.clone();
        h.update(cx, |m, cx| m.start_install(cx));
    }
}

/// The user clicked **Restart to update**.
pub fn restart_to_update(cx: &mut App) {
    if let Some(h) = cx.try_global::<UpdateManagerHandle>() {
        let h = h.0.clone();
        h.update(cx, |m, cx| m.restart(cx));
    }
}

/// Writes `updates.skip` to the user's settings file and clears the notice.
pub fn skip_version(tag: &str, cx: &mut App) {
    if let Some(h) = cx.try_global::<UpdateManagerHandle>() {
        let h = h.0.clone();
        h.update(cx, |m, _| m.forget_offer());
    }
    let path = crate::app_state::settings_path(cx);
    let value = serde_json::to_string(tag).unwrap_or_else(|_| format!("\"{tag}\""));
    if let Err(e) = settings::edit_file(&path, &["updates", "skip"], &value) {
        crate::log::warn(&format!("could not save updates.skip: {:?}", e.kind()));
    }
    if cx.has_global::<ConfigBanner>() {
        cx.global_mut::<ConfigBanner>().clear_update_notice();
        cx.refresh_windows();
    }
    settings_changed(cx);
}
