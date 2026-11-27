//! Sign-in calls on the bare `slack.com` host, cookie authenticated or not
//! authenticated at all (no token): `signin.findWorkspaces` for a browser session,
//! and the email-code flow (send a code, confirm it, list the workspaces for the
//! confirmed email). One attempt per call, no retry: the caller decides what next.

use std::fmt;
use std::time::Duration;

use reqwest::header::{COOKIE, ORIGIN, REFERER, RETRY_AFTER, SET_COOKIE, USER_AGENT};
use serde::Deserialize;
use url::Url;
use zeroize::Zeroizing;

use crate::client::{DEFAULT_USER_AGENT, decode_response, transport};
use crate::retry::parse_retry_after;
use crate::team::Icon;
use crate::{ApiError, Secret, SessionCookie};

/// One workspace the session can open, as the sign-in picker needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigninWorkspace {
    pub id: String,
    pub name: String,
    /// Subdomain: `<domain>.slack.com`.
    pub domain: String,
    pub icon: Option<String>,
    pub sso_required: bool,
    pub two_factor_required: bool,
    pub is_logged_out: bool,
}

/// One workspace a confirmed email can open, with its one-time login link when the
/// service gave one on a provider host. The link is a secret: never logged.
#[derive(Debug)]
pub struct EmailWorkspace {
    pub team: SigninWorkspace,
    pub login_link: Option<Secret>,
}

/// What sending a code did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendCodeOutcome {
    /// A code is on its way; `resend_after` when the service says how long to wait.
    Sent { resend_after: Option<Duration> },
    /// The address signs in through SSO: no code is sent.
    NeedsSso,
    /// The address signs in with a password: no code is sent.
    NeedsPassword,
}

/// The email-confirmation cookies set by a confirmed code, as a `Cookie:` header.
/// Zeroized on drop and never printed.
pub struct EmailConfirmation {
    header: Zeroizing<String>,
    count: usize,
}

impl EmailConfirmation {
    /// How many confirmation cookies were kept.
    pub fn cookie_count(&self) -> usize {
        self.count
    }
}

impl fmt::Debug for EmailConfirmation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EmailConfirmation(REDACTED)")
    }
}

/// Cookie names the confirmation is made of (auth.md, "Native email-code sign-in").
const CONFIRMATION_COOKIES: &[&str] = &["ec", "pec"];

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    current_teams: Vec<Account>,
}

#[derive(Deserialize)]
struct Account {
    #[serde(default)]
    teams: Vec<Team>,
}

#[derive(Deserialize)]
struct Team {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    domain: String,
    #[serde(default)]
    icon: Option<Icon>,
    #[serde(default)]
    sso_required: bool,
    #[serde(default)]
    two_factor_required: bool,
    #[serde(default)]
    is_logged_out: bool,
    #[serde(default, deserialize_with = "zeroizing")]
    magic_login_url: Option<Zeroizing<String>>,
}

/// Reads a secret string straight into a zeroizing buffer.
fn zeroizing<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Zeroizing<String>>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.map(Zeroizing::new))
}

/// Every team with an id, once each, in the order listed.
fn teams(env: Envelope) -> Vec<(SigninWorkspace, Option<Zeroizing<String>>)> {
    let mut out: Vec<(SigninWorkspace, Option<Zeroizing<String>>)> = Vec::new();
    for t in env.current_teams.into_iter().flat_map(|a| a.teams) {
        // A team without an id cannot be opened: skip it rather than fail the list.
        let Some(id) = t.id.filter(|i| !i.is_empty()) else {
            continue;
        };
        if out.iter().any(|(w, _)| w.id == id) {
            continue;
        }
        out.push((
            SigninWorkspace {
                id,
                name: t.name,
                domain: t.domain,
                icon: t.icon.and_then(Icon::best),
                sso_required: t.sso_required,
                two_factor_required: t.two_factor_required,
                is_logged_out: t.is_logged_out,
            },
            t.magic_login_url,
        ));
    }
    out
}

/// True for `slack.com`, `slack-gov.com` and their subdomains.
fn is_provider_host(host: &str) -> bool {
    let host = host.trim_start_matches('.').to_ascii_lowercase();
    ["slack.com", "slack-gov.com"]
        .iter()
        .any(|d| host == *d || host.strip_suffix(d).is_some_and(|h| h.ends_with('.')))
}

/// A login link Parla may follow: `https`, no credentials or port, a provider host.
fn provider_link(raw: &str) -> Option<Secret> {
    let url = Url::parse(raw).ok()?;
    let ok = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(is_provider_host);
    ok.then(|| Secret::new(raw.to_string()))
}

/// A form value in a buffer wiped on drop.
fn z(s: &str) -> Zeroizing<String> {
    Zeroizing::new(s.to_string())
}

struct Reply {
    retry_after: Duration,
    /// `Set-Cookie` lines (they can hold confirmation cookies).
    cookies: Vec<Zeroizing<String>>,
    bytes: Zeroizing<Vec<u8>>,
}

/// One multipart POST to `/api/<method>` on the bare host. A 429 is a rate limit;
/// any other non-success status is an HTTP error. Nothing here logs.
async fn post(
    base: &Url,
    method: &'static str,
    form: Vec<(&'static str, Zeroizing<String>)>,
    cookie: Option<&str>,
    referer: &'static str,
) -> Result<Reply, ApiError> {
    let mut url = base
        .join(&format!("/api/{method}"))
        .map_err(|_| ApiError::Config("bad api base".to_string()))?;
    url.query_pairs_mut()
        .append_pair("_x_version_ts", "noversion");
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(transport)?;
    let mut multipart = reqwest::multipart::Form::new();
    for (k, v) in form {
        // reqwest owns its copy of the value; the form's own buffer is wiped on drop.
        multipart = multipart.text(k, v.as_str().to_string());
    }
    let mut req = http
        .post(url)
        .header(ORIGIN, "https://slack.com")
        .header(REFERER, referer)
        .header(USER_AGENT, DEFAULT_USER_AGENT)
        .multipart(multipart);
    if let Some(c) = cookie {
        req = req.header(COOKIE, c);
    }
    let resp = req.send().await.map_err(transport)?;
    let retry_after = parse_retry_after(
        resp.headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
    );
    let status = resp.status();
    if status.as_u16() == 429 {
        return Err(ApiError::RateLimited { retry_after });
    }
    if !status.is_success() {
        return Err(ApiError::Http {
            status: status.as_u16(),
        });
    }
    let cookies = resp
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|v| Zeroizing::new(v.to_string())))
        .collect();
    let bytes = Zeroizing::new(resp.bytes().await.map_err(transport)?.to_vec());
    Ok(Reply {
        retry_after,
        cookies,
        bytes,
    })
}

/// Lists the signed-in workspaces for `cookie`. `base` is `https://slack.com/` (a test
/// server in tests). One attempt, no retry: the caller falls back to the browser flow.
pub async fn find_workspaces(
    base: &Url,
    cookie: &SessionCookie,
) -> Result<Vec<SigninWorkspace>, ApiError> {
    let form = vec![
        ("only_logged_in_teams", z("true")),
        ("ssb_signin", z("false")),
        ("_x_reason", z("get_started_already_signed_into")),
        ("_x_mode", z("online")),
    ];
    let header = Zeroizing::new(cookie.header_value());
    let reply = post(
        base,
        "signin.findWorkspaces",
        form,
        Some(&header),
        "https://slack.com/signin",
    )
    .await?;
    let env: Envelope = decode_response("signin.findWorkspaces", &reply.bytes, reply.retry_after)?;
    Ok(teams(env).into_iter().map(|(w, _)| w).collect())
}

/// Asks the service to email a sign-in code to `email` (`resend` for a second one).
/// No token, no cookie. SSO-only and password-only addresses are outcomes, not errors.
pub async fn send_code(
    base: &Url,
    email: &str,
    locale: &str,
    resend: bool,
) -> Result<SendCodeOutcome, ApiError> {
    let mut form = vec![("email", z(email)), ("locale", z(locale))];
    if resend {
        form.push(("resend", z("1")));
        form.push(("_x_reason", z("get_started_resend_code")));
    } else {
        form.push(("_x_reason", z("get_started_send_code")));
    }
    form.push(("_x_mode", z("online")));
    let reply = post(
        base,
        "signup.confirmEmail",
        form,
        None,
        "https://slack.com/get-started",
    )
    .await?;
    match decode_response::<serde::de::IgnoredAny>(
        "signup.confirmEmail",
        &reply.bytes,
        reply.retry_after,
    ) {
        Ok(_) => Ok(SendCodeOutcome::Sent { resend_after: None }),
        Err(ApiError::Slack { error, .. }) if error == "sso_required" => {
            Ok(SendCodeOutcome::NeedsSso)
        }
        Err(ApiError::Slack { error, .. }) if error == "password_required" => {
            Ok(SendCodeOutcome::NeedsPassword)
        }
        Err(e) => Err(e),
    }
}

#[derive(Deserialize)]
struct CodeRejected {
    #[serde(default)]
    error: String,
    #[serde(default)]
    attempts_left: Option<u8>,
}

/// Sends the emailed `code` once. On success returns the email-confirmation cookies
/// the reply set (only the known names, only for provider hosts).
pub async fn confirm_code(
    base: &Url,
    email: &str,
    code: &Secret,
) -> Result<EmailConfirmation, ApiError> {
    let form = vec![
        ("email", z(email)),
        ("code", z(code.expose())),
        ("_x_reason", z("get_started_confirm_code")),
        ("_x_mode", z("online")),
    ];
    let reply = post(
        base,
        "signin.confirmCode",
        form,
        None,
        "https://slack.com/get-started",
    )
    .await?;
    if let Ok(r) = serde_json::from_slice::<CodeRejected>(&reply.bytes) {
        match r.error.as_str() {
            "invalid_code" => {
                return Err(ApiError::WrongCode {
                    attempts_left: r.attempts_left,
                });
            }
            "code_expired" => return Err(ApiError::CodeExpired),
            "too_many_attempts" => return Err(ApiError::TooManyAttempts),
            _ => {}
        }
    }
    decode_response::<serde::de::IgnoredAny>(
        "signin.confirmCode",
        &reply.bytes,
        reply.retry_after,
    )?;
    let mut header = Zeroizing::new(String::new());
    let mut count = 0;
    for raw in &reply.cookies {
        let mut parts = raw.split(';');
        let Some((name, value)) = parts.next().and_then(|p| p.trim().split_once('=')) else {
            continue;
        };
        if !CONFIRMATION_COOKIES.contains(&name) || value.is_empty() {
            continue;
        }
        let domain_ok = parts
            .filter_map(|p| p.trim().split_once('='))
            .find(|(k, _)| k.eq_ignore_ascii_case("domain"))
            .is_none_or(|(_, d)| is_provider_host(d.trim()));
        if !domain_ok {
            continue;
        }
        if !header.is_empty() {
            header.push_str("; ");
        }
        header.push_str(name);
        header.push('=');
        header.push_str(value);
        count += 1;
    }
    if count == 0 {
        return Err(ApiError::Decode {
            method: "signin.confirmCode",
            pointer: String::new(),
            message: "no confirmation cookie".to_string(),
        });
    }
    Ok(EmailConfirmation { header, count })
}

/// Lists the workspaces the confirmed `email` can open (the chooser variant of
/// `signin.findWorkspaces`). A team without an id is skipped; a login link that is
/// not `https` on a provider host is dropped and the row kept.
pub async fn find_workspaces_for_email(
    base: &Url,
    conf: &EmailConfirmation,
    email: &str,
) -> Result<Vec<EmailWorkspace>, ApiError> {
    let form = vec![
        ("email", z(email)),
        ("commingled_response", z("false")),
        ("_x_reason", z("get_started_workspaces")),
        ("_x_mode", z("online")),
    ];
    let reply = post(
        base,
        "signin.findWorkspaces",
        form,
        Some(&conf.header),
        "https://app.slack.com/signin",
    )
    .await?;
    let env: Envelope = decode_response("signin.findWorkspaces", &reply.bytes, reply.retry_after)?;
    Ok(teams(env)
        .into_iter()
        .map(|(team, link)| EmailWorkspace {
            team,
            login_link: link.as_deref().and_then(|l| provider_link(l)),
        })
        .collect())
}
