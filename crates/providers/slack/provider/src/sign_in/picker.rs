//! Mapping from `signin.findWorkspaces` to the neutral workspace picker, and
//! the session that adds the picked workspaces one after another in the
//! sign-in webview.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use components::sign_in::{Candidate, SignInError};
use components::workspace_picker::{
    AddState, AddStep, RefreshOutcome, RefreshStep, WorkspaceAdder, WorkspaceChoice,
};
use domain::ImageRef;
use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Task, Window};
use slack_api::{SessionCookie, SigninWorkspace};
use slack_auth::webview::{
    AdderWebview, CaptureFailure, Start, TeamOutcome, WebviewDriver, capture_team, capture_teams,
};
use slack_auth::{AccountIndex, DiscoveredWorkspace};
use url::Url;
use zeroize::Zeroizing;

use super::finish::complete_capture;
use super::{fresh_header, parse_cookie, text};
use crate::factory::CredentialBackend;

/// Ids already in Parla, so the picker shows them disabled.
pub(crate) fn added_ids(accounts_path: &Path) -> HashSet<String> {
    AccountIndex::load(accounts_path)
        .map(|index| {
            index
                .accounts()
                .iter()
                .filter(|a| a.workspace.provider.0 == "slack")
                .map(|a| a.workspace.id.0.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The picker rows for the workspaces the session can open.
pub(crate) fn choices(
    found: Vec<SigninWorkspace>,
    added: &HashSet<String>,
    refreshed: &HashSet<String>,
) -> Vec<WorkspaceChoice> {
    found
        .into_iter()
        .map(|w| {
            let is_added = added.contains(&w.id);
            let updated = is_added && refreshed.contains(&w.id);
            WorkspaceChoice {
                id: w.id,
                name: w.name,
                domain: w.domain,
                icon: w.icon.map(|u| ImageRef::Web(Arc::from(u.as_str()))),
                sso: w.sso_required,
                two_factor: w.two_factor_required,
                signed_out: w.is_logged_out,
                added: is_added,
                updated,
                updating: false,
                update_error: false,
                needs_sign_in: false,
            }
        })
        .collect()
}

/// Rows and the added workspaces to refresh in the background.
#[derive(Debug, PartialEq)]
pub struct PreparedPicker {
    pub rows: Vec<WorkspaceChoice>,
    pub refresh: Vec<String>,
}

/// What to do with the workspaces a sign-in found.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Captured workspaces to save now: the ones already in Parla (their
    /// credentials are refreshed) and, without a picker, the one new one.
    pub save: Vec<String>,
    /// Whether to open the picker for workspaces not in Parla yet.
    pub pick: bool,
}

/// Decides from the listed rows and the ids the capture already holds. Only
/// workspaces not yet in Parla need the picker, and not when the single one
/// is the workspace just captured.
pub(crate) fn plan(rows: &[WorkspaceChoice], captured: &[String]) -> Plan {
    let new: Vec<&WorkspaceChoice> = rows.iter().filter(|r| !r.added).collect();
    let has = |id: &str| captured.iter().any(|c| c == id);
    let pick = !(new.is_empty() || new.len() == 1 && has(&new[0].id));
    let mut save: Vec<String> = rows
        .iter()
        .filter(|r| has(&r.id) && (r.added || !pick))
        .map(|r| r.id.clone())
        .collect();
    if save.is_empty() && !pick {
        // Nothing listed matches: save what was captured, as before.
        save = captured.to_vec();
    }
    Plan { save, pick }
}

/// What to do with a listed session, given any workspace already captured from
/// the client page.
#[derive(Debug, PartialEq)]
pub(crate) enum ListedAction {
    /// Listing failed or empty: keep the webview and the capture loop.
    KeepWaiting,
    /// No new workspaces to choose: refresh every added workspace hidden,
    /// save, close, and return the refreshed keys.
    RefreshAllAdded {
        rows: Vec<WorkspaceChoice>,
        refresh: Vec<String>,
    },
    /// A single new workspace was not captured from the client page: capture it
    /// and save it directly, with no picker.
    CaptureDirect { team_id: String },
    /// The only new workspace is the one on the client page: save the captured
    /// workspaces directly.
    SaveDirect {
        rows: Vec<WorkspaceChoice>,
        captured: Vec<String>,
        plan: Plan,
    },
    /// Open the picker. Added workspaces refresh in the background.
    Pick(PreparedPicker),
}

/// Decides what to do for a session whose workspaces are listed. `captured`
/// holds the ids read from the current client page, if any.
pub(crate) fn listed_action(
    listed: Option<Vec<SigninWorkspace>>,
    added: &HashSet<String>,
    captured: &[String],
) -> ListedAction {
    let Some(listed) = listed.filter(|l| !l.is_empty()) else {
        return ListedAction::KeepWaiting;
    };
    let mut rows = choices(listed, added, &HashSet::new());
    let no_new_rows = rows.iter().all(|r| r.added);
    if no_new_rows {
        let refresh: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        for row in &mut rows {
            row.updating = true;
            row.updated = false;
        }
        return ListedAction::RefreshAllAdded { rows, refresh };
    }
    let new: Vec<&WorkspaceChoice> = rows.iter().filter(|r| !r.added).collect();
    if new.len() == 1 && rows.len() == 1 && captured.is_empty() {
        return ListedAction::CaptureDirect {
            team_id: new[0].id.clone(),
        };
    }
    let p = plan(&rows, captured);
    if !p.pick {
        return ListedAction::SaveDirect {
            rows,
            captured: captured.to_vec(),
            plan: p,
        };
    }
    let refresh: Vec<String> = rows
        .iter()
        .filter(|r| r.added)
        .map(|r| r.id.clone())
        .collect();
    for row in &mut rows {
        if row.added {
            row.updating = true;
            row.updated = false;
        }
    }
    ListedAction::Pick(PreparedPicker { rows, refresh })
}

/// Prepares the picker for a session that can list its workspaces. Added rows
/// are marked `Updating...` and refreshed in the picker session. `None` keeps
/// the webview and the capture loop.
pub fn prepare_picker(
    listed: Option<Vec<SigninWorkspace>>,
    added: &HashSet<String>,
) -> Option<PreparedPicker> {
    match listed_action(listed, added, &[]) {
        ListedAction::Pick(picker) => Some(picker),
        _ => None,
    }
}

/// What the sign-in window does when the session cookie is ready.
#[cfg(test)]
#[derive(Debug, PartialEq)]
pub(crate) enum SessionPlan {
    /// Open the native picker with these rows and refresh these added workspaces.
    Pick(PreparedPicker),
    /// Keep the webview and the capture loop.
    KeepWaiting,
}

/// Decides what to do as soon as the session exists, before any client page was
/// captured. A list with workspaces to choose opens the native picker; a failed
/// or empty lookup, or a list with no new workspaces to choose, keeps the
/// webview so the capture can continue or finish as before.
#[cfg(test)]
pub(crate) fn session_plan(
    listed: Option<Vec<SigninWorkspace>>,
    added: &HashSet<String>,
) -> SessionPlan {
    match listed_action(listed, added, &[]) {
        ListedAction::Pick(picker) => SessionPlan::Pick(picker),
        _ => SessionPlan::KeepWaiting,
    }
}

/// Adds picked workspaces from the session cookie held by the sign-in webview.
pub struct SlackAdder<W: AdderWebview> {
    rt: tokio::runtime::Handle,
    backend: Arc<dyn CredentialBackend>,
    accounts_path: PathBuf,
    webview: Rc<W>,
    urls: Rc<futures::lock::Mutex<Option<UnboundedReceiver<Url>>>>,
    cookie: Zeroizing<String>,
    skip: RefCell<Option<UnboundedSender<()>>>,
    refresh_task: RefCell<Option<Task<()>>>,
}

impl<W: AdderWebview> SlackAdder<W> {
    pub fn new(
        rt: tokio::runtime::Handle,
        backend: Arc<dyn CredentialBackend>,
        accounts_path: PathBuf,
        webview: Rc<W>,
        urls: UnboundedReceiver<Url>,
        cookie: Zeroizing<String>,
    ) -> Self {
        Self {
            rt,
            backend,
            accounts_path,
            webview,
            urls: Rc::new(futures::lock::Mutex::new(Some(urls))),
            cookie,
            skip: RefCell::new(None),
            refresh_task: RefCell::new(None),
        }
    }
}

fn report_state(state: AddState, id: String, progress: &UnboundedSender<AddStep>) {
    let _ = progress.unbounded_send(AddStep { id, state });
}

fn report_refresh(outcome: RefreshOutcome, id: String, progress: &UnboundedSender<RefreshStep>) {
    let _ = progress.unbounded_send(RefreshStep { id, outcome });
}

/// Refreshes added workspaces one by one in the hidden webview, reporting each
/// outcome as it finishes. A workspace that needs the user is reported as
/// `NeedsUser` and skipped so the other rows can continue.
#[allow(clippy::too_many_arguments)]
pub async fn refresh_teams(
    driver: &dyn WebviewDriver,
    urls: &mut UnboundedReceiver<Url>,
    rt: &tokio::runtime::Handle,
    backend: &Arc<dyn CredentialBackend>,
    accounts_path: &Path,
    header: &str,
    ids: &[String],
    progress: &UnboundedSender<RefreshStep>,
) {
    for id in ids {
        let outcome = capture_team(driver, urls, id, Start::Navigate).await;
        match outcome {
            TeamOutcome::Captured(workspace) => {
                match save(
                    rt,
                    backend.clone(),
                    accounts_path,
                    &fresh_header(driver, header),
                    *workspace,
                )
                .await
                {
                    Ok(added) => {
                        if let Some(candidate) = added.into_iter().find(|c| c.ready) {
                            report_refresh(
                                RefreshOutcome::Updated(candidate.key),
                                id.clone(),
                                progress,
                            );
                        } else {
                            report_refresh(RefreshOutcome::Failed, id.clone(), progress);
                        }
                    }
                    Err(_) => report_refresh(RefreshOutcome::Failed, id.clone(), progress),
                }
                tokio::task::yield_now().await;
            }
            TeamOutcome::NeedsUser => {
                report_refresh(RefreshOutcome::NeedsUser, id.clone(), progress);
                tokio::task::yield_now().await;
            }
            TeamOutcome::Failed(CaptureFailure::Closed) => {
                report_refresh(RefreshOutcome::Failed, id.clone(), progress);
                break;
            }
            TeamOutcome::Failed(_) => {
                report_refresh(RefreshOutcome::Failed, id.clone(), progress);
                tokio::task::yield_now().await;
            }
        }
    }
}

impl<W: AdderWebview> SlackAdder<W> {
    /// Refreshes added workspaces in the hidden webview, holding the session
    /// for the whole run.
    pub async fn run_refresh(&self, ids: Vec<String>, progress: UnboundedSender<RefreshStep>) {
        refresh_session(
            self.urls.clone(),
            self.webview.clone(),
            self.rt.clone(),
            self.backend.clone(),
            self.accounts_path.clone(),
            self.cookie.as_str().to_string(),
            ids,
            progress,
        )
        .await;
    }

    /// Adds workspaces in the hidden webview, holding the session for the
    /// whole run.
    pub async fn run_add(
        &self,
        ids: Vec<String>,
        progress: UnboundedSender<AddStep>,
    ) -> Result<Vec<Candidate>, SignInError> {
        let (skip_tx, skip_rx) = futures::channel::mpsc::unbounded();
        *self.skip.borrow_mut() = Some(skip_tx);
        add_session(
            self.urls.clone(),
            self.webview.clone(),
            self.rt.clone(),
            self.backend.clone(),
            self.accounts_path.clone(),
            self.cookie.as_str().to_string(),
            ids,
            skip_rx,
            progress,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn refresh_session<W: AdderWebview>(
    urls: Rc<futures::lock::Mutex<Option<UnboundedReceiver<Url>>>>,
    webview: Rc<W>,
    rt: tokio::runtime::Handle,
    backend: Arc<dyn CredentialBackend>,
    accounts_path: PathBuf,
    header: String,
    ids: Vec<String>,
    progress: UnboundedSender<RefreshStep>,
) {
    if ids.is_empty() {
        return;
    }
    let mut guard = urls.lock().await;
    let Some(urls_ref) = guard.as_mut() else {
        for id in ids {
            report_refresh(RefreshOutcome::Failed, id, &progress);
        }
        return;
    };
    let _ = webview.set_visible(false);
    let driver = webview.driver();
    refresh_teams(
        &driver,
        urls_ref,
        &rt,
        &backend,
        &accounts_path,
        &header,
        &ids,
        &progress,
    )
    .await;
    let _ = webview.set_visible(false);
}

#[allow(clippy::too_many_arguments)]
async fn add_session<W: AdderWebview>(
    urls: Rc<futures::lock::Mutex<Option<UnboundedReceiver<Url>>>>,
    webview: Rc<W>,
    rt: tokio::runtime::Handle,
    backend: Arc<dyn CredentialBackend>,
    accounts_path: PathBuf,
    header: String,
    ids: Vec<String>,
    mut skip_rx: futures::channel::mpsc::UnboundedReceiver<()>,
    progress: UnboundedSender<AddStep>,
) -> Result<Vec<Candidate>, SignInError> {
    let mut guard = urls.lock().await;
    let Some(urls_ref) = guard.as_mut() else {
        return Err(SignInError::Failed(text("signin-error-interrupted")));
    };
    let _ = webview.set_visible(false);
    let order = ids.clone();
    if let Some(first) = ids.first() {
        report_state(AddState::SigningIn, first.clone(), &progress);
    }
    let driver = webview.driver();
    let results = capture_teams(&driver, urls_ref, &mut skip_rx, &ids, |id, outcome| {
        let needs_user = matches!(outcome, TeamOutcome::NeedsUser);
        let state = match outcome {
            TeamOutcome::NeedsUser => {
                let _ = webview.set_visible(true);
                AddState::NeedsUser
            }
            TeamOutcome::Captured(_) => AddState::SigningIn,
            TeamOutcome::Failed(_) => AddState::Failed,
        };
        report_state(state, id.to_string(), &progress);
        if !needs_user {
            let _ = webview.set_visible(false);
            if let Some(pos) = order.iter().position(|i| i == id)
                && let Some(next) = order.get(pos + 1)
            {
                report_state(AddState::SigningIn, next.clone(), &progress);
            }
        }
        needs_user
    })
    .await;
    let _ = webview.set_visible(false);

    let mut ready = Vec::new();
    for result in results {
        let TeamOutcome::Captured(workspace) = result.outcome else {
            continue;
        };
        match save(
            &rt,
            backend.clone(),
            &accounts_path,
            &fresh_header(&driver, &header),
            *workspace,
        )
        .await
        {
            Ok(added) => {
                let state = if added.is_empty() {
                    AddState::Failed
                } else {
                    AddState::Done
                };
                report_state(state, result.team_id, &progress);
                ready.extend(added);
            }
            Err(_) => report_state(AddState::Failed, result.team_id, &progress),
        }
    }
    webview.sign_in_done();
    Ok(ready)
}

impl<W: AdderWebview> WorkspaceAdder for SlackAdder<W> {
    fn skip(&self) {
        if let Some(tx) = self.skip.borrow().as_ref() {
            let _ = tx.unbounded_send(());
        }
    }

    fn refresh(
        &self,
        ids: Vec<String>,
        progress: UnboundedSender<RefreshStep>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let urls = self.urls.clone();
        let wv = self.webview.clone();
        let rt = self.rt.clone();
        let backend = self.backend.clone();
        let path = self.accounts_path.clone();
        let header = self.cookie.as_str().to_string();
        let task = window.spawn(cx, async move |_| {
            refresh_session(urls, wv, rt, backend, path, header, ids, progress).await;
        });
        *self.refresh_task.borrow_mut() = Some(task);
    }

    fn add(
        &self,
        ids: Vec<String>,
        progress: UnboundedSender<AddStep>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<Candidate>, SignInError>> {
        let (skip_tx, skip_rx) = futures::channel::mpsc::unbounded();
        *self.skip.borrow_mut() = Some(skip_tx);
        let urls = self.urls.clone();
        let wv = self.webview.clone();
        let rt = self.rt.clone();
        let backend = self.backend.clone();
        let path = self.accounts_path.clone();
        let header = self.cookie.as_str().to_string();
        window.spawn(cx, async move |_| {
            add_session(urls, wv, rt, backend, path, header, ids, skip_rx, progress).await
        })
    }
}

/// Saves one captured workspace exactly like a single-workspace sign-in.
async fn save(
    rt: &tokio::runtime::Handle,
    backend: Arc<dyn CredentialBackend>,
    accounts_path: &Path,
    header: &str,
    workspace: DiscoveredWorkspace,
) -> Result<Vec<Candidate>, SignInError> {
    let cookie: SessionCookie = parse_cookie(header)?;
    let path = accounts_path.to_path_buf();
    rt.spawn(async move { complete_capture(&*backend, &path, cookie, vec![workspace]).await })
        .await
        .unwrap_or_else(|_| Err(SignInError::Failed(text("signin-error-interrupted"))))
        .map(|found: Vec<Candidate>| found.into_iter().filter(|c| c.ready).collect())
}

/// Handles `SessionReady`, before any client page was captured. It returns the
/// picker rows immediately with added workspaces marked for background refresh;
/// the refresh runs in the picker session. `None` keeps the webview and the
/// capture loop.
pub fn early_session(
    accounts_path: &Path,
    listed: Option<Vec<SigninWorkspace>>,
) -> Option<PreparedPicker> {
    prepare_picker(listed, &added_ids(accounts_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(id: &str, domain: &str, sso: bool, two_factor: bool, out: bool) -> SigninWorkspace {
        SigninWorkspace {
            id: id.into(),
            name: format!("{id} Workspace"),
            domain: domain.into(),
            icon: None,
            sso_required: sso,
            two_factor_required: two_factor,
            is_logged_out: out,
        }
    }

    #[test]
    fn maps_every_workspace_and_marks_the_added_ones() {
        let added: HashSet<String> = ["T2".to_string()].into_iter().collect();
        let rows = choices(
            vec![
                ws("T1", "one", false, false, false),
                ws("T2", "two", false, false, false),
                ws("T3", "three", true, true, true),
            ],
            &added,
            &HashSet::new(),
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "T1");
        assert!(!rows[0].added);
        assert!(rows[1].added);
        assert!(rows[2].sso);
        assert!(rows[2].two_factor);
        assert!(rows[2].signed_out);
        assert_eq!(rows[0].domain, "one");
    }

    fn row(id: &str, added: bool) -> WorkspaceChoice {
        choices(
            vec![ws(id, id, false, false, false)],
            &if added {
                [id.to_string()].into()
            } else {
                HashSet::new()
            },
            &HashSet::new(),
        )
        .remove(0)
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn added_workspaces_are_refreshed_and_only_new_ones_open_the_picker() {
        let rows = vec![row("T1", true), row("T2", false), row("T3", false)];
        let p = plan(&rows, &ids(&["T1"]));
        assert_eq!(
            p,
            Plan {
                save: ids(&["T1"]),
                pick: true
            }
        );
    }

    #[test]
    fn nothing_new_refreshes_without_a_picker() {
        let rows = vec![row("T1", true), row("T2", true)];
        let p = plan(&rows, &ids(&["T2"]));
        assert_eq!(
            p,
            Plan {
                save: ids(&["T2"]),
                pick: false
            }
        );
    }

    #[test]
    fn the_one_new_workspace_just_captured_is_saved_directly() {
        let rows = vec![row("T1", true), row("T2", false)];
        let p = plan(&rows, &ids(&["T2"]));
        assert_eq!(
            p,
            Plan {
                save: ids(&["T2"]),
                pick: false
            }
        );
        let p = plan(&rows, &ids(&["T1", "T2"]));
        assert_eq!(
            p,
            Plan {
                save: ids(&["T1", "T2"]),
                pick: false
            }
        );
    }

    #[test]
    fn one_new_workspace_that_was_not_captured_still_opens_the_picker() {
        let rows = vec![row("T1", true), row("T2", false)];
        let p = plan(&rows, &ids(&["T1"]));
        assert_eq!(
            p,
            Plan {
                save: ids(&["T1"]),
                pick: true
            }
        );
    }

    #[test]
    fn a_capture_unrelated_to_the_list_is_saved_whole() {
        let rows = vec![row("T1", true)];
        let p = plan(&rows, &ids(&["TX"]));
        assert_eq!(
            p,
            Plan {
                save: ids(&["TX"]),
                pick: false
            }
        );
    }

    #[test]
    fn an_empty_index_marks_nothing_as_added() {
        let dir = tempfile::tempdir().unwrap();
        assert!(added_ids(&dir.path().join("accounts.json")).is_empty());
    }

    #[test]
    fn session_ready_with_three_workspaces_opens_the_picker() {
        let listed = vec![
            ws("T1", "one", false, false, false),
            ws("T2", "two", false, false, false),
            ws("T3", "three", false, false, false),
        ];
        let SessionPlan::Pick(picker) = session_plan(Some(listed), &HashSet::new()) else {
            panic!("the picker opens without a capture");
        };
        assert_eq!(picker.rows.len(), 3);
        assert!(picker.rows.iter().all(|r| !r.added));
        assert!(picker.refresh.is_empty());
    }

    #[test]
    fn session_ready_single_new_workspace_captures_directly() {
        let listed = vec![ws("T1", "one", false, false, false)];
        assert_eq!(
            listed_action(Some(listed), &HashSet::new(), &[]),
            ListedAction::CaptureDirect {
                team_id: "T1".to_string()
            }
        );
    }

    #[test]
    fn session_ready_lookup_failure_keeps_the_webview() {
        assert_eq!(
            session_plan(None, &HashSet::new()),
            SessionPlan::KeepWaiting
        );
        assert_eq!(
            session_plan(Some(Vec::new()), &HashSet::new()),
            SessionPlan::KeepWaiting
        );
    }

    #[test]
    fn session_ready_all_added_refreshes_hidden_without_a_picker() {
        let listed = vec![ws("T1", "one", false, false, false)];
        let added: HashSet<String> = ["T1".to_string()].into();
        let ListedAction::RefreshAllAdded { rows, refresh } =
            listed_action(Some(listed), &added, &[])
        else {
            panic!("all-added session refreshes hidden");
        };
        assert_eq!(rows.len(), 1);
        assert!(rows[0].added);
        assert!(rows[0].updating);
        assert_eq!(refresh, vec!["T1".to_string()]);
    }

    #[test]
    fn one_new_workspace_on_the_client_page_is_saved_without_a_picker() {
        let listed = vec![ws("T1", "one", false, false, false)];
        let ListedAction::SaveDirect {
            rows,
            captured,
            plan: p,
        } = listed_action(Some(listed), &HashSet::new(), &["T1".to_string()])
        else {
            panic!("the captured workspace is saved directly");
        };
        assert_eq!(rows.len(), 1);
        assert!(!p.pick);
        assert_eq!(p.save, vec!["T1".to_string()]);
        assert_eq!(captured, vec!["T1".to_string()]);
    }

    #[test]
    fn one_new_workspace_not_on_the_client_page_is_captured_directly() {
        let listed = vec![ws("T1", "one", false, false, false)];
        assert_eq!(
            listed_action(Some(listed), &HashSet::new(), &[]),
            ListedAction::CaptureDirect {
                team_id: "T1".to_string()
            }
        );
    }
}
