//! Signs in several workspaces one after another in one webview that already holds the
//! session cookie. Driven by navigation events only: no polling, no timers.

use futures::{FutureExt, Stream, StreamExt};
use url::Url;

use super::capture::{CaptureFailure, UrlKind, WebviewDriver, classify_url, cookie_from_jar};
use slack_api::validate::is_team_id;

use crate::{DiscoveredWorkspace, READ_TOKENS_SCRIPT, parse_local_config};

#[derive(Debug)]
pub enum TeamOutcome {
    /// The workspace's token was read.
    Captured(Box<DiscoveredWorkspace>),
    /// The page left the web client (SSO, two-factor, signed out): show the webview and
    /// let the user finish, then call [`capture_team`] with [`Start::Resume`].
    NeedsUser,
    Failed(CaptureFailure),
}

#[derive(Debug)]
pub struct TeamResult {
    pub team_id: String,
    pub outcome: TeamOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Navigate to the workspace's web client first.
    Navigate,
    /// The user is finishing sign-in in the webview: only wait for the client.
    Resume,
}

/// The team id in `https://app.slack.com/client/<team>[/...]`.
fn client_team(url: &Url) -> Option<String> {
    (classify_url(url) == UrlKind::ClientReady)
        .then(|| url.path_segments()?.nth(1).map(str::to_string))
        .flatten()
}

async fn read_team(driver: &dyn WebviewDriver, team_id: &str) -> Option<TeamOutcome> {
    let raw = driver.eval(READ_TOKENS_SCRIPT).await.ok().flatten()?;
    let found = parse_local_config(&raw).ok()?;
    let ws = found.into_iter().find(|w| w.workspace.id.0 == team_id)?;
    match driver.cookies().ok().and_then(|c| cookie_from_jar(&c)) {
        Some(_) => Some(TeamOutcome::Captured(Box::new(ws))),
        None => Some(TeamOutcome::Failed(CaptureFailure::NoCookie)),
    }
}

/// Captures one workspace. Events for other workspaces' clients are ignored (late events
/// from the previous page); any other page means the user is needed. A config that does
/// not hold the token yet is retried on the next navigation event.
pub async fn capture_team<S: Stream<Item = Url> + Unpin>(
    driver: &dyn WebviewDriver,
    urls: &mut S,
    team_id: &str,
    start: Start,
) -> TeamOutcome {
    if !is_team_id(team_id) {
        return TeamOutcome::Failed(CaptureFailure::NoTeams);
    }
    let Ok(target) = Url::parse(&format!("https://app.slack.com/client/{team_id}")) else {
        return TeamOutcome::Failed(CaptureFailure::NoTeams);
    };
    let mut via_link = false;
    if start == Start::Navigate {
        // Events queued by the previous page must not be taken for this one.
        while urls.next().now_or_never().flatten().is_some() {}
        match driver.open_team(team_id) {
            Ok(link) => via_link = link,
            Err(_) => return TeamOutcome::Failed(CaptureFailure::Closed),
        }
    }
    let mut client_loaded = false;
    let mut last = CaptureFailure::Closed;
    while let Some(url) = urls.next().await {
        let kind = classify_url(&url);
        if via_link {
            // A login link passes through the workspace's own pages, and may end on
            // the "open the desktop app" step: once there, the session cookie exists,
            // so open the web client. Back on the sign-in page means the link failed.
            match kind {
                UrlKind::SigningIn => continue,
                UrlKind::DesktopHandoff if !client_loaded => {
                    client_loaded = true;
                    if driver.load(&target).is_err() {
                        return TeamOutcome::Failed(CaptureFailure::Closed);
                    }
                    continue;
                }
                _ => {}
            }
        }
        match kind {
            // The user finished signing in and Slack ends on "open the desktop app",
            // which is never loaded: open the team's client, once.
            UrlKind::DesktopHandoff if start == Start::Resume && !client_loaded => {
                client_loaded = true;
                if driver.load(&target).is_err() {
                    return TeamOutcome::Failed(CaptureFailure::Closed);
                }
            }
            UrlKind::DisallowedUserAgent => {
                return TeamOutcome::Failed(CaptureFailure::DisallowedUserAgent);
            }
            UrlKind::ClientReady if client_team(&url).as_deref() == Some(team_id) => {
                match read_team(driver, team_id).await {
                    Some(done) => return done,
                    None => last = CaptureFailure::NoTeams,
                }
            }
            UrlKind::ClientReady => {}
            // The user is finishing SSO or two-factor: these pages are expected until the
            // team's client shows up (or the window closes).
            UrlKind::SigningIn | UrlKind::SessionPage | UrlKind::DesktopHandoff
                if start == Start::Resume => {}
            UrlKind::SigningIn | UrlKind::SessionPage | UrlKind::DesktopHandoff => {
                return TeamOutcome::NeedsUser;
            }
        }
    }
    TeamOutcome::Failed(last)
}

/// Captures `team_ids` in order. `progress` sees each outcome; for [`TeamOutcome::NeedsUser`]
/// it returns whether to wait for the user (then it sees the final outcome too) or skip.
/// While waiting, an item on `skip` abandons that team (it stays `NeedsUser`).
pub async fn capture_teams<S: Stream<Item = Url> + Unpin, K: Stream<Item = ()> + Unpin>(
    driver: &dyn WebviewDriver,
    urls: &mut S,
    skip: &mut K,
    team_ids: &[String],
    mut progress: impl FnMut(&str, &TeamOutcome) -> bool,
) -> Vec<TeamResult> {
    let mut out = Vec::new();
    for id in team_ids {
        let mut outcome = capture_team(driver, urls, id, Start::Navigate).await;
        // One call per outcome: a skipped team is reported once, as NeedsUser.
        if matches!(outcome, TeamOutcome::NeedsUser) && progress(id, &outcome) {
            // A click made before this wait began is not for this team.
            while skip.next().now_or_never().flatten().is_some() {}
            let resume = capture_team(driver, urls, id, Start::Resume).fuse();
            futures::pin_mut!(resume);
            let mut skipped = skip.next().fuse();
            outcome = loop {
                futures::select! {
                    done = resume => break done,
                    s = skipped => if s.is_some() { break TeamOutcome::NeedsUser },
                }
            };
            if !matches!(outcome, TeamOutcome::NeedsUser) {
                progress(id, &outcome);
            }
        } else if !matches!(outcome, TeamOutcome::NeedsUser) {
            progress(id, &outcome);
        }
        let closed = matches!(outcome, TeamOutcome::Failed(CaptureFailure::Closed));
        out.push(TeamResult {
            team_id: id.clone(),
            outcome,
        });
        if closed {
            break;
        }
    }
    out
}
