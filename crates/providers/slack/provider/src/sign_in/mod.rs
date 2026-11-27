//! The provider's sign-in screens: consent notice, "Sign in with browser window"
//! (login webview capture) and "Paste token and cookie". Nothing here logs or
//! shows a credential; errors are fixed texts.

mod consent;
mod finish;
mod manual;
mod picker;

use std::path::PathBuf;
use std::sync::Arc;

use components::sign_in::{
    SignIn, SignInError, SignInField, SignInOption, SignInResult, SignInText,
};
use components::workspace_picker::RefreshStep;
use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Task, Window};
use settings::WorkspaceKey;
use slack_auth::webview::CaptureFailure;

pub use finish::{complete_capture, sign_out};
pub use manual::complete_manual;
pub use picker::{PreparedPicker, SlackAdder, early_session, prepare_picker, refresh_teams};

use crate::factory::CredentialBackend;

/// Option ids.
pub const OPTION_WINDOW: &str = "window";
pub const OPTION_PASTE: &str = "paste";

/// A sign-in text from the provider catalog (namespace `slack`).
pub(crate) fn text(id: &str) -> SignInText {
    i18n::LocalizedText::new("slack", id).into()
}

pub use consent::notice;

/// What the screen shows for an auth failure. `AuthError` Display never
/// includes a secret; the detail of the generic case is that fixed text.
pub(crate) fn auth_error_text(e: &slack_auth::AuthError) -> SignInText {
    use slack_auth::AuthError;
    match e {
        AuthError::NoTeams => text("signin-error-no-teams"),
        AuthError::BadConfig => text("signin-error-config"),
        AuthError::Expired => text("signin-error-expired"),
        other => i18n::LocalizedText::new("slack", "signin-error-failed")
            .with("detail", other.to_string())
            .into(),
    }
}

/// Maps a capture failure to what the screen shows. A refused window falls
/// back to manual paste with the reason.
pub fn capture_failure(f: CaptureFailure) -> SignInError {
    match f {
        CaptureFailure::DisallowedUserAgent => SignInError::SwitchTo {
            option: OPTION_PASTE.to_string(),
            reason: text("signin-error-refused"),
        },
        CaptureFailure::NoCookie => SignInError::Failed(text("signin-error-no-cookie")),
        CaptureFailure::NoTeams => SignInError::Failed(text("signin-error-no-teams")),
        CaptureFailure::Closed => SignInError::Failed(text("signin-error-closed")),
    }
}

type Relayout = std::rc::Rc<std::cell::RefCell<Option<Box<dyn Fn(&Window)>>>>;

pub struct SlackSignIn {
    rt: tokio::runtime::Handle,
    accounts_path: PathBuf,
    backend: Arc<dyn CredentialBackend>,
    top_inset: std::cell::Cell<f32>,
    /// Re-lays out the open provider window for the current viewport.
    relayout: Relayout,
}

impl SlackSignIn {
    pub fn new(
        rt: tokio::runtime::Handle,
        accounts_path: PathBuf,
        backend: Arc<dyn CredentialBackend>,
    ) -> SlackSignIn {
        SlackSignIn {
            rt,
            accounts_path,
            backend,
            top_inset: std::cell::Cell::new(0.0),
            relayout: Default::default(),
        }
    }
}

fn failed<T: 'static>(id: &str) -> Task<Result<T, SignInError>> {
    Task::ready(Err(SignInError::Failed(text(id))))
}

/// Parses the session cookie kept as a header line (it is never cloned as a
/// secret type, only re-parsed from its zeroized text).
/// The session cookie as the webview holds it now, else `fallback`. A chooser
/// sign-in starts without one: each team's login link creates or updates it.
pub(crate) fn fresh_header(
    driver: &dyn slack_auth::webview::WebviewDriver,
    fallback: &str,
) -> zeroize::Zeroizing<String> {
    let now = driver
        .cookies()
        .ok()
        .and_then(|c| slack_auth::webview::cookie_from_jar(&c));
    zeroize::Zeroizing::new(match now {
        Some(c) => c.header_value(),
        None => fallback.to_string(),
    })
}

pub(crate) fn parse_cookie(header: &str) -> Result<slack_api::SessionCookie, SignInError> {
    slack_api::SessionCookie::parse(header)
        .map_err(|_| SignInError::Failed(text("signin-error-no-cookie")))
}

impl SignIn for SlackSignIn {
    fn provider_name(&self) -> String {
        "Slack".to_string()
    }

    fn notice(&self) -> Vec<SignInText> {
        notice()
    }

    fn options(&self) -> Vec<SignInOption> {
        let mut out = Vec::new();
        if cfg!(target_os = "macos") {
            out.push(SignInOption {
                id: OPTION_WINDOW.to_string(),
                label: text("signin-option-window"),
                fields: vec![],
                help: None,
                copy: None,
            });
        }
        out.push(SignInOption {
            id: OPTION_PASTE.to_string(),
            label: text("signin-option-paste"),
            fields: vec![
                SignInField {
                    id: "cookie".to_string(),
                    label: text("signin-field-cookie"),
                    secret: true,
                },
                SignInField {
                    id: "config".to_string(),
                    label: text("signin-field-console"),
                    secret: true,
                },
            ],
            help: Some(text("signin-help-paste")),
            copy: Some(slack_auth::MANUAL_CONSOLE_SNIPPET.to_string()),
        });
        out
    }

    fn run(
        &self,
        option: &str,
        values: Vec<(String, String)>,
        progress: UnboundedSender<RefreshStep>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        match option {
            OPTION_PASTE => {
                let get = |id: &str| {
                    values
                        .iter()
                        .find(|(k, _)| k == id)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default()
                };
                let (cookie, config) = (get("cookie"), get("config"));
                let backend = self.backend.clone();
                let path = self.accounts_path.clone();
                let rt = self.rt.clone();
                let _ = progress;
                cx.spawn(async move |_| {
                    rt.spawn(
                        async move { complete_manual(&*backend, &path, &cookie, &config).await },
                    )
                    .await
                    .unwrap_or_else(|_| Err(SignInError::Failed(text("signin-error-interrupted"))))
                    .map(SignInResult::Workspaces)
                })
            }
            OPTION_WINDOW => self.run_window(progress, window, cx),
            _ => {
                let _ = progress;
                failed("signin-error-unknown-option")
            }
        }
    }

    fn set_top_inset(&self, inset: f32) {
        self.top_inset.set(inset);
    }

    fn layout_window(&self, window: &Window) {
        if let Some(f) = self.relayout.borrow().as_ref() {
            f(window);
        }
    }

    fn sign_out(&self, ws: &WorkspaceKey, cx: &mut App) -> Task<Result<(), String>> {
        let backend = self.backend.clone();
        let path = self.accounts_path.clone();
        let rt = self.rt.clone();
        let ws = ws.clone();
        cx.spawn(async move |_| {
            rt.spawn(async move { sign_out(&*backend, &path, &ws).await })
                .await
                .unwrap_or_else(|_| Err("Sign-out was interrupted.".to_string()))
        })
    }
}

#[cfg(target_os = "macos")]
impl SlackSignIn {
    fn run_window(
        &self,
        progress: UnboundedSender<RefreshStep>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        use slack_auth::webview::{CaptureState, LoginWebview, run_capture_with};

        let mut wv = match LoginWebview::open(window, cx) {
            Ok(w) => w,
            Err(_) => return failed("signin-error-window-open"),
        };
        let Some(urls) = wv.take_urls() else {
            return failed("signin-error-window-open");
        };
        // Below the host's top strip and our header; re-laid out on every resize.
        let top = gpui::px(components::sign_in::overlay_top(self.top_inset.get()));
        let layout = move |wv: &LoginWebview, window: &Window| {
            let size = window.viewport_size();
            wv.set_bounds(gpui::Bounds::new(
                gpui::point(gpui::px(0.0), top),
                gpui::size(size.width, (size.height - top).max(gpui::px(0.0))),
            ));
        };
        layout(&wv, window);
        let wv = std::rc::Rc::new(wv);
        {
            // Weak: dropping the sign-in task still closes the webview.
            let wv = std::rc::Rc::downgrade(&wv);
            *self.relayout.borrow_mut() = Some(Box::new(move |window| {
                if let Some(wv) = wv.upgrade() {
                    layout(&wv, window);
                }
            }));
        }
        let slot = self.relayout.clone();
        let backend = self.backend.clone();
        let path = self.accounts_path.clone();
        let rt = self.rt.clone();
        let rt_lookup = self.rt.clone();
        window.spawn(cx, async move |_| {
            let lookup = HandoffLookup { rt: rt_lookup };
            let mut urls = urls;
            let mut fell_back = std::collections::HashSet::new();
            let mut listed_once: Option<Option<Vec<slack_api::SigninWorkspace>>> = None;
            loop {
                let state =
                    run_capture_with(&wv.driver(), &mut urls, &lookup, &mut fell_back).await;
                let ready = match state {
                    CaptureState::SessionReady(cookie) => {
                        let listed = if let Some(listed) = &listed_once {
                            listed.clone()
                        } else {
                            let listed = list_workspaces(&rt, &cookie).await;
                            listed_once = Some(listed.clone());
                            listed
                        };
                        Some((listed, zeroize::Zeroizing::new(cookie.header_value())))
                    }
                    // Slack's chooser listed the workspaces before any session cookie
                    // exists: each team is opened through its login link, and the
                    // cookie is read from the webview when it is saved.
                    CaptureState::Chooser(list) => {
                        Some((Some(list), zeroize::Zeroizing::new(String::new())))
                    }
                    CaptureState::Failed(f) => {
                        close_webview(wv, &slot);
                        return Err(capture_failure(f));
                    }
                    _ => {
                        close_webview(wv, &slot);
                        return Err(SignInError::Failed(text("signin-error-closed")));
                    }
                };
                if let Some((listed, header)) = ready {
                    {
                        let added = picker::added_ids(&path);
                        let captured = read_client_workspace(&wv.driver())
                            .await
                            .unwrap_or_default();
                        let captured_ids: Vec<String> = captured
                            .iter()
                            .map(|w| w.workspace.id.0.to_string())
                            .collect();
                        match picker::listed_action(listed.clone(), &added, &captured_ids) {
                            picker::ListedAction::KeepWaiting => {
                                if !captured.is_empty() {
                                    return handle_done(
                                        SignInCtx {
                                            wv,
                                            slot: &slot,
                                            rt,
                                            backend,
                                            path,
                                            urls,
                                            header,
                                        },
                                        captured,
                                        listed,
                                    )
                                    .await;
                                }
                                let _ = wv.set_visible(true);
                                continue;
                            }
                            picker::ListedAction::CaptureDirect { team_id } => {
                                let outcome =
                                    capture_single_new(&wv.driver(), &mut urls, &team_id, |v| {
                                        let _ = wv.set_visible(v);
                                    })
                                    .await;
                                let workspace = match outcome {
                                    Ok(workspace) => workspace,
                                    Err(e) => {
                                        close_webview(wv, &slot);
                                        return Err(e);
                                    }
                                };
                                let cookie_parsed = parse_cookie(
                                    fresh_header(&wv.driver(), header.as_str()).as_str(),
                                )?;
                                let result = rt
                                    .spawn(async move {
                                        complete_capture(
                                            &*backend,
                                            &path,
                                            cookie_parsed,
                                            vec![*workspace],
                                        )
                                        .await
                                    })
                                    .await
                                    .unwrap_or_else(|_| {
                                        Err(SignInError::Failed(text("signin-error-interrupted")))
                                    });
                                close_webview(wv, &slot);
                                return result.map(SignInResult::Workspaces);
                            }
                            picker::ListedAction::RefreshAllAdded { rows, refresh } => {
                                let driver = wv.driver();
                                let refreshed = refresh_all_added(
                                    &driver,
                                    &mut urls,
                                    &refresh,
                                    &rows,
                                    |v| {
                                        let _ = wv.set_visible(v);
                                    },
                                    |workspace| {
                                        let cookie = fresh_header(&wv.driver(), header.as_str());
                                        let backend = backend.clone();
                                        let path = path.clone();
                                        let rt = rt.clone();
                                        async move {
                                            let cookie_parsed = parse_cookie(cookie.as_str())?;
                                            rt.spawn(async move {
                                                complete_capture(
                                                    &*backend,
                                                    &path,
                                                    cookie_parsed,
                                                    vec![*workspace],
                                                )
                                                .await
                                            })
                                            .await
                                            .unwrap_or_else(|_| {
                                                Err(SignInError::Failed(text(
                                                    "signin-error-interrupted",
                                                )))
                                            })
                                        }
                                    },
                                    &progress,
                                )
                                .await;
                                let _ = wv.set_visible(false);
                                close_webview(wv, &slot);
                                return Ok(SignInResult::Restart(refreshed));
                            }
                            picker::ListedAction::SaveDirect { .. }
                            | picker::ListedAction::Pick(_) => {
                                return handle_done(
                                    SignInCtx {
                                        wv,
                                        slot: &slot,
                                        rt,
                                        backend,
                                        path,
                                        urls,
                                        header,
                                    },
                                    captured,
                                    listed,
                                )
                                .await;
                            }
                        }
                    }
                }
            }
        })
    }
}

#[cfg(target_os = "macos")]
async fn read_client_workspace(
    driver: &dyn slack_auth::webview::WebviewDriver,
) -> Option<Vec<slack_auth::DiscoveredWorkspace>> {
    let raw = driver
        .eval(slack_auth::READ_TOKENS_SCRIPT)
        .await
        .ok()
        .flatten()?;
    let found = slack_auth::parse_local_config(&raw).ok()?;
    (!found.is_empty()).then_some(found)
}

/// What the webview capture flows hand from one step to the next: the webview, how to
/// relayout it, the runtime and account store, and the navigation/header state.
#[cfg(target_os = "macos")]
struct SignInCtx<'a> {
    wv: std::rc::Rc<slack_auth::webview::LoginWebview>,
    slot: &'a Relayout,
    rt: tokio::runtime::Handle,
    backend: Arc<dyn CredentialBackend>,
    path: PathBuf,
    urls: futures::channel::mpsc::UnboundedReceiver<url::Url>,
    header: zeroize::Zeroizing<String>,
}

#[cfg(target_os = "macos")]
async fn handle_done(
    ctx: SignInCtx<'_>,
    mut found: Vec<slack_auth::DiscoveredWorkspace>,
    listed: Option<Vec<slack_api::SigninWorkspace>>,
) -> Result<SignInResult, SignInError> {
    let SignInCtx {
        wv,
        slot,
        rt,
        backend,
        path,
        urls,
        header,
    } = ctx;
    if let Some(listed) = listed {
        let mut rows = picker::choices(
            listed,
            &picker::added_ids(&path),
            &std::collections::HashSet::new(),
        );
        let captured_ids: Vec<String> =
            found.iter().map(|w| w.workspace.id.0.to_string()).collect();
        let plan = picker::plan(&rows, &captured_ids);
        if plan.pick {
            let refresh: Vec<String> = rows
                .iter()
                .filter(|r| r.added)
                .map(|r| r.id.clone())
                .collect();
            for r in &mut rows {
                if r.added {
                    r.updating = true;
                    r.updated = false;
                }
            }
            return open_picker(
                SignInCtx {
                    wv,
                    slot,
                    rt,
                    backend,
                    path,
                    urls,
                    header,
                },
                rows,
                Vec::new(),
                refresh,
            );
        }
        found.retain(|w| plan.save.contains(&w.workspace.id.0.to_string()));
    }
    // No picker needed: save what the capture holds.
    let cookie = parse_cookie(fresh_header(&wv.driver(), header.as_str()).as_str())?;
    let result = rt
        .spawn(async move { complete_capture(&*backend, &path, cookie, found).await })
        .await
        .unwrap_or_else(|_| Err(SignInError::Failed(text("signin-error-interrupted"))));
    close_webview(wv, slot);
    result.map(SignInResult::Workspaces)
}

/// Closes a webview nobody else holds (the single-workspace flow).
#[cfg(target_os = "macos")]
fn close_webview(wv: std::rc::Rc<slack_auth::webview::LoginWebview>, slot: &Relayout) {
    wv.sign_in_done();
    slot.borrow_mut().take();
    if let Ok(wv) = std::rc::Rc::try_unwrap(wv) {
        wv.close();
    }
}

/// Hides the webview and hands the window to the native picker, keeping the
/// webview alive (hidden) for the rows that need the user.
#[cfg(target_os = "macos")]
fn open_picker(
    ctx: SignInCtx<'_>,
    rows: Vec<components::workspace_picker::WorkspaceChoice>,
    refreshed: Vec<WorkspaceKey>,
    refresh: Vec<String>,
) -> Result<SignInResult, SignInError> {
    let SignInCtx {
        wv,
        slot,
        rt,
        backend,
        path,
        urls,
        header,
    } = ctx;
    if wv.set_visible(false).is_err() {
        close_webview(wv, slot);
        return Err(SignInError::Failed(text("signin-error-closed")));
    }
    let adder = std::rc::Rc::new(picker::SlackAdder::new(
        rt,
        backend,
        path,
        wv.clone(),
        urls,
        header,
    ));
    Ok(SignInResult::Pick(
        components::sign_in::WorkspacePickerSession {
            choices: rows,
            adder,
            refreshed,
            refresh,
        },
    ))
}

/// Lists the workspaces the session can open, or `None` when the lookup failed.
/// The cookie is parsed again inside the blocking task because `SessionCookie`
/// is not `Clone`.
#[cfg(target_os = "macos")]
async fn list_workspaces(
    rt: &tokio::runtime::Handle,
    cookie: &slack_api::SessionCookie,
) -> Option<Vec<slack_api::SigninWorkspace>> {
    let line = zeroize::Zeroizing::new(cookie.header_value());
    rt.spawn(async move {
        let cookie = slack_api::SessionCookie::parse(line.as_str()).ok()?;
        let base = url::Url::parse("https://slack.com/").ok()?;
        slack_api::find_workspaces(&base, &cookie).await.ok()
    })
    .await
    .ok()
    .flatten()
}

#[cfg(not(target_os = "macos"))]
impl SlackSignIn {
    fn run_window(
        &self,
        _: UnboundedSender<RefreshStep>,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        failed("signin-error-window-unavailable")
    }
}

/// A capture that needs the user must show the webview and resume.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn capture_needs_resume(outcome: &slack_auth::webview::TeamOutcome) -> bool {
    matches!(outcome, slack_auth::webview::TeamOutcome::NeedsUser)
}

/// Captures one new workspace in the sign-in webview: navigates to it, shows the
/// webview and resumes if the user must finish sign-in, then maps the final
/// outcome. A resume never needs the user again, so a `NeedsUser` outcome at the
/// end means the capture could not complete.
#[cfg(any(target_os = "macos", test))]
pub(crate) async fn capture_single_new<D, S, V>(
    driver: &D,
    urls: &mut S,
    team_id: &str,
    mut set_visible: V,
) -> Result<Box<slack_auth::DiscoveredWorkspace>, SignInError>
where
    D: slack_auth::webview::WebviewDriver,
    S: futures::Stream<Item = url::Url> + Unpin,
    V: FnMut(bool),
{
    use slack_auth::webview::{Start, TeamOutcome, capture_team};
    set_visible(false);
    let mut outcome = capture_team(driver, urls, team_id, Start::Navigate).await;
    if capture_needs_resume(&outcome) {
        set_visible(true);
        outcome = capture_team(driver, urls, team_id, Start::Resume).await;
        set_visible(false);
    }
    match outcome {
        TeamOutcome::Captured(workspace) => Ok(workspace),
        TeamOutcome::Failed(f) => Err(capture_failure(f)),
        // A resumed capture never returns NeedsUser (see sequence::capture_team).
        _ => Err(SignInError::Failed(text("signin-error-closed"))),
    }
}

/// Refreshes added workspaces in the sign-in webview: navigates to each,
/// shows the webview and resumes if the user must finish sign-in, saves each
/// captured workspace, and reports every saved key as it is saved.
#[cfg(any(target_os = "macos", test))]
pub(crate) async fn refresh_all_added<D, S, V, Save, SaveFut>(
    driver: &D,
    urls: &mut S,
    ids: &[String],
    rows: &[components::workspace_picker::WorkspaceChoice],
    mut set_visible: V,
    mut save: Save,
    progress: &futures::channel::mpsc::UnboundedSender<components::workspace_picker::RefreshStep>,
) -> Vec<components::sign_in::Candidate>
where
    D: slack_auth::webview::WebviewDriver,
    S: futures::Stream<Item = url::Url> + Unpin,
    V: FnMut(bool),
    Save: FnMut(Box<slack_auth::DiscoveredWorkspace>) -> SaveFut,
    SaveFut: std::future::Future<Output = Result<Vec<components::sign_in::Candidate>, SignInError>>,
{
    use components::workspace_picker::{RefreshOutcome, RefreshStep};
    use slack_auth::webview::{CaptureFailure, Start, TeamOutcome, capture_team};

    set_visible(false);
    let mut refreshed = Vec::new();
    for id in ids {
        let mut outcome = capture_team(driver, urls, id, Start::Navigate).await;
        if capture_needs_resume(&outcome) {
            set_visible(true);
            outcome = capture_team(driver, urls, id, Start::Resume).await;
            set_visible(false);
        }
        match outcome {
            TeamOutcome::Captured(workspace) => {
                // A save error (a bad cookie) ends the refresh; keys saved so
                // far were already reported.
                let Ok(added) = save(workspace).await else {
                    break;
                };
                {
                    for candidate in added.into_iter().filter(|c| c.ready) {
                        let _ = progress.unbounded_send(RefreshStep {
                            id: id.clone(),
                            outcome: RefreshOutcome::Updated(candidate.key.clone()),
                        });
                        let name = rows
                            .iter()
                            .find(|r| r.id == *candidate.key.id.0)
                            .map(|r| r.name.clone())
                            .unwrap_or_default();
                        refreshed.push(components::sign_in::Candidate {
                            key: candidate.key,
                            name,
                            ready: true,
                        });
                    }
                }
            }
            TeamOutcome::Failed(CaptureFailure::Closed) => break,
            // A resumed capture never returns NeedsUser; other failures are skipped.
            _ => {}
        }
    }
    refreshed
}

/// Finds a workspace's team id from its subdomain with `signin.findWorkspaces`, so the
/// desktop-app handoff page can be replaced by the web client.
#[cfg(target_os = "macos")]
struct HandoffLookup {
    rt: tokio::runtime::Handle,
}

#[cfg(target_os = "macos")]
impl slack_auth::webview::TeamLookup for HandoffLookup {
    fn team_id(
        &self,
        domain: &str,
        cookie: &slack_api::SessionCookie,
    ) -> futures::future::LocalBoxFuture<'_, Option<String>> {
        let domain = domain.to_string();
        let line = zeroize::Zeroizing::new(cookie.header_value().to_string());
        let rt = self.rt.clone();
        Box::pin(async move {
            rt.spawn(async move {
                let cookie = slack_api::SessionCookie::parse(line.as_str()).ok()?;
                let base = url::Url::parse("https://slack.com/").ok()?;
                let all = slack_api::find_workspaces(&base, &cookie).await.ok()?;
                all.into_iter().find(|w| w.domain == domain).map(|w| w.id)
            })
            .await
            .ok()
            .flatten()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use components::workspace_picker::{RefreshOutcome, WorkspaceChoice};
    use futures::StreamExt as _;
    use settings::{ProviderId, WorkspaceId, WorkspaceKey};
    use slack_auth::webview::testing::{C2, CONFIG2, FakeWebview, dcookie, scripted};
    use slack_auth::webview::{CaptureFailure, TeamOutcome};

    fn choice(id: &str, name: &str) -> WorkspaceChoice {
        WorkspaceChoice {
            id: id.to_string(),
            name: name.to_string(),
            domain: format!("{name}.slack.com"),
            icon: None,
            sso: false,
            two_factor: false,
            signed_out: false,
            added: true,
            updated: false,
            updating: true,
            update_error: false,
            needs_sign_in: false,
        }
    }

    fn key(id: &str) -> WorkspaceKey {
        WorkspaceKey {
            provider: ProviderId("slack".to_string()),
            id: WorkspaceId(id.to_string()),
        }
    }

    #[test]
    fn needs_user_requires_a_resume_in_refresh_all_added() {
        assert!(capture_needs_resume(&TeamOutcome::NeedsUser));
        assert!(!capture_needs_resume(&TeamOutcome::Failed(
            CaptureFailure::Closed
        )));
    }

    #[test]
    fn direct_capture_maps_failures_to_their_messages() {
        assert_eq!(
            capture_failure(CaptureFailure::NoTeams),
            SignInError::Failed(text("signin-error-no-teams"))
        );
        assert_eq!(
            capture_failure(CaptureFailure::NoCookie),
            SignInError::Failed(text("signin-error-no-cookie"))
        );
        assert_eq!(
            capture_failure(CaptureFailure::Closed),
            SignInError::Failed(text("signin-error-closed"))
        );
    }

    #[tokio::test]
    async fn single_new_needs_user_shows_webview_resumes_and_captures() {
        let fake = FakeWebview::new(Some(CONFIG2), vec![dcookie()]);
        // Navigate emits SSO (NeedsUser) then the ready client; resume consumes the latter.
        let mut urls = scripted(
            &fake,
            vec![(C2, vec!["https://beta.slack.com/sso/saml/start", C2])],
        );
        let mut visible = Vec::new();
        let out = capture_single_new(&fake, &mut urls, "T0000002", |v| visible.push(v)).await;
        assert!(out.is_ok(), "expected capture, got {out:?}");
        assert_eq!(out.unwrap().workspace.id.0, "T0000002");
        // Hidden for navigate, shown to resume, hidden after resume.
        assert_eq!(visible, vec![false, true, false]);
    }

    #[tokio::test]
    async fn single_new_maps_capture_failure() {
        let fake = FakeWebview::new(Some(CONFIG2), vec![]);
        let mut urls = scripted(&fake, vec![(C2, vec![C2])]);
        let out = capture_single_new(&fake, &mut urls, "T0000002", |_| {}).await;
        assert_eq!(
            out.unwrap_err(),
            SignInError::Failed(text("signin-error-no-cookie"))
        );
    }

    #[tokio::test]
    async fn refresh_all_added_resumes_and_reports_each_saved_key() {
        let fake = FakeWebview::new(Some(CONFIG2), vec![dcookie()]);
        let mut urls = scripted(
            &fake,
            vec![(C2, vec!["https://beta.slack.com/sso/saml/start", C2])],
        );
        let mut visible = Vec::new();
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        let rows = vec![choice("T0000002", "Beta")];
        let ids = vec!["T0000002".to_string()];
        let refreshed = refresh_all_added(
            &fake,
            &mut urls,
            &ids,
            &rows,
            |v| visible.push(v),
            |ws| {
                let id = ws.workspace.id.0.clone();
                async move {
                    Ok(vec![components::sign_in::Candidate {
                        key: key(&id),
                        name: "Beta".to_string(),
                        ready: true,
                    }])
                }
            },
            &tx,
        )
        .await;
        drop(tx);

        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].key, key("T0000002"));
        assert_eq!(visible, vec![false, true, false]);

        let steps: Vec<_> = rx.by_ref().collect().await;
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].id, "T0000002");
        assert!(matches!(
            steps[0].outcome,
            RefreshOutcome::Updated(ref k) if k == &key("T0000002")
        ));
    }
}
