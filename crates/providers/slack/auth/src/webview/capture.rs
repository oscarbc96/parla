//! Sign-in capture state machine and cookie filtering. No webview types here, so it
//! is testable with a fake driver. Nothing is logged; `RawCookie` redacts its value.

use futures::future::LocalBoxFuture;
use futures::{Stream, StreamExt};
use slack_api::SessionCookie;
use url::Url;

use super::policy::{Handoff, handoff};
use crate::AuthError;
use slack_api::validate::{is_slack_host, is_team_id};

/// A cookie as read from the webview store.
#[derive(Clone)]
pub struct RawCookie {
    pub name: String,
    pub domain: String,
    value: String,
}

impl RawCookie {
    pub fn new(name: &str, domain: &str, value: &str) -> RawCookie {
        RawCookie {
            name: name.into(),
            domain: domain.into(),
            value: value.into(),
        }
    }
}

impl std::fmt::Debug for RawCookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RawCookie({}, {}, <redacted>)", self.name, self.domain)
    }
}

/// What the capture needs from a webview. The wry implementation is not `Send`, so
/// futures are local and `cookies` is synchronous (safe on the main thread).
pub trait WebviewDriver {
    /// Runs a script; `None` when it returned null.
    fn eval(&self, script: &'static str) -> LocalBoxFuture<'_, Result<Option<String>, AuthError>>;
    /// All cookies in the store (not URL-filtered: `cookies_for_url` needs an exact domain).
    fn cookies(&self) -> Result<Vec<RawCookie>, AuthError>;
    /// Navigates to `url` (an allowed web-client URL).
    fn load(&self, url: &Url) -> Result<(), AuthError>;
    /// Loads a handoff page as is, when no web-client URL could be found for it, so the
    /// user can still use Slack's own "open in the browser" link.
    fn show_handoff_page(&self, url: &Url) -> Result<(), AuthError>;
    /// The workspaces Slack's own chooser listed after the email code, before any
    /// session cookie exists. `None` until the chooser has been seen.
    fn chooser_workspaces(&self) -> Option<Vec<slack_api::SigninWorkspace>> {
        None
    }
    /// Opens `team_id`'s web client. A team from the chooser is opened through its
    /// one-time login link, which creates the session cookie; then this returns
    /// `true`. Otherwise it loads the client URL and returns `false`.
    fn open_team(&self, team_id: &str) -> Result<bool, AuthError> {
        let target = Url::parse(&format!("https://app.slack.com/client/{team_id}"))
            .map_err(|_| AuthError::Webview("bad team id".into()))?;
        self.load(&target).map(|()| false)
    }
}

/// The webview session that adds or refreshes workspaces: a driver, visibility,
/// and a way to end sign-in. Implemented by the real login webview and by tests.
pub trait AdderWebview: 'static {
    type Driver<'a>: WebviewDriver
    where
        Self: 'a;

    fn driver(&self) -> Self::Driver<'_>;
    fn set_visible(&self, visible: bool) -> Result<(), AuthError>;
    fn sign_in_done(&self);
}

/// Finds a workspace's team id from its subdomain (`signin.findWorkspaces`, which needs
/// the session cookie and the network, so the provider supplies it).
pub trait TeamLookup {
    fn team_id(&self, domain: &str, cookie: &SessionCookie) -> LocalBoxFuture<'_, Option<String>>;
}

/// No lookup: a handoff whose URL has no team id is shown as is.
pub struct NoLookup;

impl TeamLookup for NoLookup {
    fn team_id(&self, _: &str, _: &SessionCookie) -> LocalBoxFuture<'_, Option<String>> {
        Box::pin(async { None })
    }
}

#[derive(Debug)]
pub enum CaptureState {
    SigningIn,
    Reading,
    /// The session cookie exists on a Slack page, before the workspaces are listed.
    /// The provider can list them now and falls back to capturing this team if the
    /// listing fails.
    SessionReady(SessionCookie),
    /// Slack's chooser listed the workspaces after the email code, but no session
    /// cookie exists yet: each team is opened through its login link.
    Chooser(Vec<slack_api::SigninWorkspace>),
    Failed(CaptureFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureFailure {
    DisallowedUserAgent,
    NoCookie,
    NoTeams,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlKind {
    SigningIn,
    /// Slack's own workspace chooser on the bare host. A `d` cookie here means
    /// the session exists, so the native picker can open before a client page.
    SessionPage,
    ClientReady,
    DisallowedUserAgent,
    /// Slack's "opening the desktop app" step; see [`crate::webview::handoff`].
    DesktopHandoff,
}

/// The bare-host and app sign-in pages that call `signin.findWorkspaces`:
/// `/signin` (any fragment or query) and `/workspace-signin`.
fn is_session_page(url: &Url) -> bool {
    matches!(url.host_str(), Some("slack.com") | Some("app.slack.com"))
        && matches!(url.path(), "/signin" | "/workspace-signin")
}

pub fn classify_url(url: &Url) -> UrlKind {
    let host = url.host_str().unwrap_or("");
    if host == "accounts.google.com" && url.as_str().contains("disallowed_useragent") {
        return UrlKind::DisallowedUserAgent;
    }
    if super::policy::handoff(url).is_some() {
        return UrlKind::DesktopHandoff;
    }
    if is_session_page(url) {
        return UrlKind::SessionPage;
    }
    if host == "app.slack.com" {
        let mut segs = url.path_segments().into_iter().flatten();
        if segs.next() == Some("client") && segs.next().is_some_and(|t| t.starts_with('T')) {
            return UrlKind::ClientReady;
        }
    }
    UrlKind::SigningIn
}

/// The webview's script callback delivers a JSON string literal, or `null`.
pub fn decode_eval(raw: &str) -> Option<String> {
    serde_json::from_str::<Option<String>>(raw).ok().flatten()
}

fn in_slack_domain(domain: &str) -> bool {
    is_slack_host(domain.trim_start_matches('.'))
}

/// Builds the session cookie from `d` (and `d-s`) on a `slack.com` domain, matched by suffix.
pub fn cookie_from_jar(cookies: &[RawCookie]) -> Option<SessionCookie> {
    let find = |name: &str| {
        cookies
            .iter()
            .find(|c| c.name == name && in_slack_domain(&c.domain))
            .map(|c| c.value.as_str())
    };
    let d = find("d")?;
    let line = match find("d-s") {
        Some(ds) => format!("d={d}; d-s={ds}"),
        None => format!("d={d}"),
    };
    SessionCookie::parse(&line).ok()
}

/// Drives the capture from navigation events. The last failure is reported when the
/// stream ends, else `Closed`. As soon as a usable session cookie exists on a
/// sign-in page, a ready client page, or a desktop handoff, the wait ends early with
/// [`CaptureState::SessionReady`] so the provider can list the workspaces first. A
/// client page without a cookie fails with `NoCookie`.
#[cfg(any(test, feature = "test-support"))]
pub async fn run_capture(
    driver: &dyn WebviewDriver,
    signed_in: impl Stream<Item = Url>,
) -> CaptureState {
    let mut fell_back = std::collections::HashSet::new();
    run_capture_with(driver, signed_in, &NoLookup, &mut fell_back).await
}

/// The web-client URL for a handoff, resolving the team id when the URL only names the
/// workspace. `None` when it cannot be found (no cookie, lookup failed, unknown domain).
async fn handoff_target(
    driver: &dyn WebviewDriver,
    lookup: &dyn TeamLookup,
    url: &Url,
) -> Option<Url> {
    match handoff(url)? {
        Handoff::Client(u) => Some(u),
        Handoff::Lookup { domain } => {
            let cookie = driver.cookies().ok().and_then(|c| cookie_from_jar(&c))?;
            let id = lookup.team_id(&domain, &cookie).await?;
            // The id comes from the network: only a plain team id goes into a URL.
            if !is_team_id(&id) {
                return None;
            }
            Url::parse(&format!("https://app.slack.com/client/{id}")).ok()
        }
    }
}

/// [`run_capture`] that also skips the desktop-app handoff page: it is rewritten to the
/// web client, so sign-in finishes without a click. `fell_back` tracks handoff URLs
/// already handled across calls, so the same handoff page is not retried in a loop.
pub async fn run_capture_with(
    driver: &dyn WebviewDriver,
    signed_in: impl Stream<Item = Url>,
    lookup: &dyn TeamLookup,
    fell_back: &mut std::collections::HashSet<String>,
) -> CaptureState {
    let mut urls = Box::pin(signed_in);
    let mut last = CaptureFailure::Closed;
    while let Some(url) = urls.next().await {
        match classify_url(&url) {
            UrlKind::SigningIn => {}
            UrlKind::SessionPage => {
                // The chooser page carries the session cookie: the provider can
                // list the workspaces without waiting for a client page. Without
                // the cookie it is still the sign-in form, so keep waiting.
                if let Some(cookie) = driver.cookies().ok().and_then(|c| cookie_from_jar(&c)) {
                    return CaptureState::SessionReady(cookie);
                }
                // After the email code the chooser lists the workspaces before any
                // session cookie exists.
                if let Some(list) = driver.chooser_workspaces().filter(|l| !l.is_empty()) {
                    return CaptureState::Chooser(list);
                }
            }
            UrlKind::DesktopHandoff => {
                // Each handoff URL is handled once: a page shown as is comes back as a
                // navigation event, and looking it up again would loop.
                if !fell_back.insert(url.to_string()) {
                    continue;
                }
                let cookie = driver.cookies().ok().and_then(|c| cookie_from_jar(&c));
                let moved = match handoff_target(driver, lookup, &url).await {
                    Some(target) => driver.load(&target),
                    None => driver.show_handoff_page(&url),
                };
                if moved.is_err() {
                    last = CaptureFailure::Closed;
                }
                // The handoff page carries the session cookie: list the workspaces
                // first. The rewrite above keeps the flow moving if the listing fails
                // and the caller falls back to waiting.
                if let Some(cookie) = cookie {
                    return CaptureState::SessionReady(cookie);
                }
            }
            UrlKind::DisallowedUserAgent => {
                return CaptureState::Failed(CaptureFailure::DisallowedUserAgent);
            }
            UrlKind::ClientReady => {
                // The client page carries the session cookie: list the workspaces
                // first. The caller falls back to reading this team if the listing
                // fails.
                if let Some(cookie) = driver.cookies().ok().and_then(|c| cookie_from_jar(&c)) {
                    return CaptureState::SessionReady(cookie);
                }
                // No cookie on the client page: nothing more can be captured.
                return CaptureState::Failed(CaptureFailure::NoCookie);
            }
        }
    }
    CaptureState::Failed(last)
}
