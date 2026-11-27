//! Opening a team without a webview: follow its one-time login
//! link with plain HTTPS requests to get the `d` session cookie, then read the
//! team token from its boot page. Hop shapes are unverified until a live probe
//! run (`examples/native_probe.rs`) confirms them. Nothing here
//! logs a body, link, cookie or token.

use std::time::Duration;

use reqwest::header::{COOKIE, LOCATION, SET_COOKIE, USER_AGENT};
use slack_api::{DEFAULT_USER_AGENT, Secret, SessionCookie, XoxcToken};
use url::Url;
use zeroize::Zeroizing;

use crate::is_workspace_url;

/// Redirect hops followed at most, after the first request.
pub const MAX_HOPS: usize = 5;

/// Why a team could not be opened natively. Messages never hold a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NativeError {
    /// The team wants the user (SSO, two-factor, password, a sign-in page): open
    /// it in the browser window instead.
    #[error("the workspace needs you to sign in")]
    NeedsUser,
    #[error("the service could not be reached")]
    Offline,
    /// A hop left the provider's hosts, looped, or answered with an error.
    #[error("the service refused the sign-in link")]
    Refused,
    /// The link or the page was not in the expected shape.
    #[error("unexpected answer from the service")]
    Malformed,
}

/// Path words of pages that need the user (auth.md, "Native email-code sign-in").
const NEEDS_USER: &[&str] = &["sso", "saml", "two_factor", "2fa", "password"];

fn needs_user(url: &Url) -> bool {
    url.path_segments().is_some_and(|mut segs| {
        segs.any(|s| {
            let s = s.to_ascii_lowercase();
            NEEDS_USER
                .iter()
                .any(|w| s == *w || s.starts_with(&format!("{w}_")))
        })
    })
}

fn client() -> Result<reqwest::Client, NativeError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| NativeError::Malformed)
}

fn transport(e: reqwest::Error) -> NativeError {
    if e.is_connect() || e.is_timeout() {
        NativeError::Offline
    } else {
        NativeError::Refused
    }
}

/// Where a cookie may be sent: the exact host that set it (no `Domain`), or a
/// domain and its subdomains.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Scope {
    Host(String),
    Domain(String),
}

impl Scope {
    fn matches(&self, url: &Url) -> bool {
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };
        match self {
            Scope::Host(h) => host == *h,
            Scope::Domain(d) => {
                host == *d
                    || host
                        .strip_suffix(d.as_str())
                        .is_some_and(|h| h.ends_with('.'))
            }
        }
    }
}

/// A session cookie and where it may be sent.
struct Scoped {
    value: Zeroizing<String>,
    scope: Scope,
}

/// The value and scope of a `name=` cookie in one `Set-Cookie` line received
/// from `from`. A `Domain` that is not a provider host, or that does not cover
/// `from`, is ignored as a browser would.
fn session_cookie(line: &str, name: &str, from: &Url) -> Option<Scoped> {
    let mut parts = line.split(';');
    let (k, v) = parts.next()?.trim().split_once('=')?;
    if k != name || v.is_empty() {
        return None;
    }
    let domain = parts
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| k.eq_ignore_ascii_case("domain"))
        .map(|(_, d)| d.trim().trim_start_matches('.').to_ascii_lowercase());
    let scope = match domain {
        None => Scope::Host(from.host_str()?.to_ascii_lowercase()),
        Some(d) => {
            let ok = Url::parse(&format!("https://{d}/")).is_ok_and(|u| is_workspace_url(&u));
            let scope = Scope::Domain(d);
            if !ok || !scope.matches(from) {
                return None;
            }
            scope
        }
    };
    Some(Scoped {
        value: Zeroizing::new(v.to_string()),
        scope,
    })
}

/// The `Cookie:` header for `url`: only the cookies whose scope covers it.
fn cookie_header(url: &Url, d: &Option<Scoped>, d_s: &Option<Scoped>) -> Option<Zeroizing<String>> {
    let mut out = Zeroizing::new(String::new());
    for (name, c) in [("d", d), ("d-s", d_s)] {
        let Some(c) = c.as_ref().filter(|c| c.scope.matches(url)) else {
            continue;
        };
        if !out.is_empty() {
            out.push_str("; ");
        }
        out.push_str(name);
        out.push('=');
        out.push_str(&c.value);
    }
    (!out.is_empty()).then_some(out)
}

/// Follows the one-time login `link` (resolved against `base`) with redirects off,
/// at most [`MAX_HOPS`] hops, only on provider hosts, and returns the `d` (and
/// `d-s`) cookie it set. Each hop carries only the cookies scoped to its host.
/// One request per hop, no retry.
pub async fn open_team(link: &Secret, base: &Url) -> Result<SessionCookie, NativeError> {
    let mut url = base
        .join(link.expose())
        .map_err(|_| NativeError::Malformed)?;
    let http = client()?;
    let mut d: Option<Scoped> = None;
    let mut d_s: Option<Scoped> = None;
    for _ in 0..=MAX_HOPS {
        if !is_workspace_url(&url) {
            return Err(NativeError::Refused);
        }
        if needs_user(&url) {
            return Err(NativeError::NeedsUser);
        }
        let mut req = http.get(url.clone()).header(USER_AGENT, DEFAULT_USER_AGENT);
        if let Some(header) = cookie_header(&url, &d, &d_s) {
            req = req.header(COOKIE, header.as_str());
        }
        let resp = req.send().await.map_err(transport)?;
        for line in resp.headers().get_all(SET_COOKIE) {
            let Ok(line) = line.to_str() else { continue };
            if let Some(c) = session_cookie(line, "d", &url) {
                d = Some(c);
            }
            if let Some(c) = session_cookie(line, "d-s", &url) {
                d_s = Some(c);
            }
        }
        let status = resp.status();
        if status.is_redirection() {
            let next = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(NativeError::Malformed)?;
            url = url.join(next).map_err(|_| NativeError::Malformed)?;
            continue;
        }
        if !status.is_success() {
            return Err(NativeError::Refused);
        }
        let Some(d) = d else {
            return Err(NativeError::NeedsUser);
        };
        let line = Zeroizing::new(match &d_s {
            Some(ds) => format!("d={}; d-s={}", d.value.as_str(), ds.value.as_str()),
            None => format!("d={}", d.value.as_str()),
        });
        return SessionCookie::parse(&line).map_err(|_| NativeError::Malformed);
    }
    Err(NativeError::Refused)
}

const TOKEN_KEY: &str = "\"api_token\":\"";

/// Reads the team token from the boot page `<team_url>/ssb/redirect`, sent with
/// only the session cookie. A redirect means the session is not good for this team.
pub async fn boot_token(team_url: &Url, cookie: &SessionCookie) -> Result<Secret, NativeError> {
    if !is_workspace_url(team_url) {
        return Err(NativeError::Refused);
    }
    let url = team_url
        .join("ssb/redirect")
        .map_err(|_| NativeError::Malformed)?;
    let header = Zeroizing::new(cookie.header_value());
    let resp = client()?
        .get(url)
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .header(COOKIE, header.as_str())
        .send()
        .await
        .map_err(transport)?;
    let status = resp.status();
    if status.is_redirection() {
        return Err(NativeError::NeedsUser);
    }
    if !status.is_success() {
        return Err(NativeError::Refused);
    }
    let body = Zeroizing::new(resp.text().await.map_err(transport)?);
    let start = body.find(TOKEN_KEY).ok_or(NativeError::Malformed)? + TOKEN_KEY.len();
    let len = body[start..].find('"').ok_or(NativeError::Malformed)?;
    let token = XoxcToken::parse(&body[start..start + len]).map_err(|_| NativeError::Malformed)?;
    Ok(Secret::new(token.expose().to_string()))
}
