//! Form client for the web-client API: POST, `token` field first, session cookie.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, COOKIE, LOCATION, ORIGIN, RETRY_AFTER, USER_AGENT};
use serde::de::{DeserializeOwned, IgnoredAny};
use url::Url;

use backon::Retryable;

use crate::limit::Limiter;
use crate::method::{Method, Priority, Tier};
use crate::retry;
use crate::{ApiError, RateLimits, SessionCookie, XoxcToken};

/// The web-session API is the one a browser uses, so requests mimic the web
/// client's Chrome user agent on macOS.
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

const ORIGIN_VALUE: &str = "https://app.slack.com";

/// Where the web client reads cached data (running huddles); one path per team.
pub const EDGE_BASE: &str = "https://edgeapi.slack.com/cache/";

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub api_base: Url,
    /// Edge cache API base (`https://edgeapi.slack.com/cache/`); `<team>/<path>` is appended.
    pub edge_base: Url,
    pub user_agent: String,
    pub timeout: Duration,
    pub limits: RateLimits,
    /// First retry delay; doubles (with jitter) up to 3 retries. A `Retry-After` replaces it.
    pub retry_base_delay: Duration,
    /// Hosts that may receive file requests with credentials.
    pub file_hosts: crate::FileHosts,
}

impl ClientConfig {
    /// `url` is the workspace (or Grid org) host; the API lives at `<origin>/api/`.
    pub fn for_workspace(url: &Url) -> ClientConfig {
        let mut api_base = url.clone();
        api_base.set_path("/api/");
        api_base.set_query(None);
        api_base.set_fragment(None);
        ClientConfig {
            api_base,
            edge_base: Url::parse(EDGE_BASE).expect("static url"),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            timeout: Duration::from_secs(30),
            limits: RateLimits::default(),
            retry_base_delay: Duration::from_secs(1),
            file_hosts: crate::FileHosts::default(),
        }
    }
}

/// Retry policy of [`WebClient::call_ok`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Retry {
    /// Safe to repeat: retried on 429, 5xx and transport errors.
    Idempotent,
    /// Not safe to repeat: retried only on 429.
    Write,
}

#[derive(Clone)]
pub struct WebClient {
    pub(crate) http: reqwest::Client,
    pub(crate) cfg: ClientConfig,
    pub(crate) token: std::sync::Arc<XoxcToken>,
    pub(crate) cookie: std::sync::Arc<SessionCookie>,
    recorder: Option<std::sync::Arc<redact::record::Recorder>>,
    limiter: std::sync::Arc<Limiter>,
}

impl fmt::Debug for WebClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebClient")
            .field("api_base", &self.cfg.api_base.as_str())
            .field("credentials", &"REDACTED")
            .finish()
    }
}

pub(crate) fn transport(e: reqwest::Error) -> ApiError {
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connect failed"
    } else if e.is_redirect() {
        "redirect"
    } else if e.is_decode() || e.is_body() {
        "body"
    } else {
        "request failed"
    };
    ApiError::Transport(kind.to_string())
}

/// A team id: `T` or `E` and upper-case alphanumerics, nothing that is special in a URL.
fn is_team_id(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some('T' | 'E' | 'W'))
        && s.len() > 1
        && c.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn ms_since(t: std::time::Instant) -> u64 {
    u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Where the time of one write call went, summed over its attempts.
#[derive(Default)]
struct WriteTiming {
    limiter_ms: AtomicU64,
    http_ms: AtomicU64,
    attempts: AtomicU64,
}

impl WriteTiming {
    /// One INFO line: `post timing: limiter_ms=.. http_ms=.. retries=.. wait_ms=.. total_ms=.. outcome=..`.
    /// `wait_ms` is the time spent between attempts (the server's 429 hold).
    fn log(&self, started: std::time::Instant, err: Option<&ApiError>) {
        let (limiter, http) = (
            self.limiter_ms.load(Ordering::Relaxed),
            self.http_ms.load(Ordering::Relaxed),
        );
        let total = ms_since(started);
        let attempts = self.attempts.load(Ordering::Relaxed);
        let outcome = match err {
            None => "ok",
            Some(ApiError::RateLimited { .. }) => "rate_limited",
            Some(ApiError::Transport(_)) => "transport",
            Some(ApiError::Http { .. }) => "http",
            Some(_) => "error",
        };
        log::info!(
            "post timing: limiter_ms={limiter} http_ms={http} retries={} wait_ms={} total_ms={total} outcome={outcome}",
            attempts.saturating_sub(1),
            total.saturating_sub(limiter + http),
        );
    }
}

/// The channel a post is rate limited under.
fn post_key<'a>(method: Method, params: &[(&str, &'a str)]) -> Option<&'a str> {
    if method.tier != Tier::PostMessage {
        return None;
    }
    params
        .iter()
        .find(|(k, _)| *k == "channel")
        .map(|(_, v)| *v)
}

/// Decodes a web-API reply: the `ok`/`error` envelope (with the `ratelimited` and
/// `invalid_auth` mappings), then the typed body. Shared by the token and cookie calls.
pub(crate) fn decode_response<R: DeserializeOwned>(
    method: &'static str,
    bytes: &[u8],
    retry_after: Duration,
) -> Result<R, ApiError> {
    let decode = |message: String| ApiError::Decode {
        method,
        pointer: String::new(),
        message,
    };
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| decode(e.to_string()))?;
    if value.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let error = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown_error")
            .to_string();
        if error == "ratelimited" {
            return Err(ApiError::RateLimited { retry_after });
        }
        if error == "invalid_auth" {
            return Err(ApiError::InvalidAuth);
        }
        let needed = value
            .get("needed")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        return Err(ApiError::Slack {
            method,
            error,
            needed,
        });
    }
    serde_path_to_error::deserialize(value).map_err(|e| ApiError::Decode {
        method,
        pointer: pointer_of(&e.path().to_string()),
        message: e.inner().to_string(),
    })
}

fn pointer_of(path: &str) -> String {
    if path == "." || path.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for seg in path.replace('[', ".").replace(']', "").split('.') {
        if !seg.is_empty() {
            out.push('/');
            out.push_str(seg);
        }
    }
    out
}

impl WebClient {
    pub fn new(
        cfg: ClientConfig,
        token: XoxcToken,
        cookie: SessionCookie,
    ) -> Result<WebClient, ApiError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(cfg.timeout)
            .build()
            .map_err(transport)?;
        let limiter = std::sync::Arc::new(Limiter::new(cfg.limits));
        Ok(WebClient {
            http,
            cfg,
            token: std::sync::Arc::new(token),
            cookie: std::sync::Arc::new(cookie),
            recorder: None,
            limiter,
        })
    }

    /// The session cookie this client authenticates with.
    pub fn session_cookie(&self) -> std::sync::Arc<SessionCookie> {
        self.cookie.clone()
    }

    /// Hand one exchange to the recorder; a success status waits for its body.
    fn record(
        &self,
        tap: &Option<(String, Vec<(String, String)>)>,
        status: u16,
        body: &str,
        skip: bool,
    ) {
        if skip {
            return;
        }
        if let (Some(rec), Some((url, form))) = (&self.recorder, tap) {
            let headers = [("origin".to_string(), ORIGIN_VALUE.to_string())];
            rec.record_http("POST", url, &headers, form, status, body);
        }
    }

    /// Record every exchange, redacted, through `recorder` (debug tooling).
    pub fn with_recorder(mut self, recorder: std::sync::Arc<redact::record::Recorder>) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Rate limited by tier and priority, retried on transient failures.
    pub async fn call<R: DeserializeOwned>(
        &self,
        method: Method,
        params: &[(&str, &str)],
        prio: Priority,
    ) -> Result<R, ApiError> {
        let key = post_key(method, params);
        (|| async {
            self.limiter
                .acquire(method.name, method.tier, key, prio)
                .await;
            self.send(method, params)
                .await
                .inspect_err(|e| self.note_rate_limit(method, key, e))
        })
        .retry(retry::backoff(self.cfg.retry_base_delay))
        .when(retry::is_retryable)
        .adjust(retry::delay_for)
        .sleep(tokio::time::sleep)
        .await
    }

    /// Rate limited like [`call`](Self::call) but never retried: for best
    /// effort background calls that must not hold up or repeat.
    pub async fn call_once<R: DeserializeOwned>(
        &self,
        method: Method,
        params: &[(&str, &str)],
        prio: Priority,
    ) -> Result<R, ApiError> {
        self.limiter
            .acquire(method.name, method.tier, None, prio)
            .await;
        self.send(method, params)
            .await
            .inspect_err(|e| self.note_rate_limit(method, None, e))
    }

    /// For non-idempotent writes: rate limited, and retried only when the server answered
    /// 429 (so nothing was done). A timeout, transport failure or 5xx after the request went
    /// out is never resent, since the write may already have happened.
    pub async fn call_write<R: DeserializeOwned>(
        &self,
        method: Method,
        params: &[(&str, &str)],
        prio: Priority,
    ) -> Result<R, ApiError> {
        let key = post_key(method, params);
        let timing = WriteTiming::default();
        let started = std::time::Instant::now();
        let out = (|| async {
            let t = std::time::Instant::now();
            self.limiter
                .acquire(method.name, method.tier, key, prio)
                .await;
            timing.limiter_ms.fetch_add(ms_since(t), Ordering::Relaxed);
            let t = std::time::Instant::now();
            let r = self
                .send(method, params)
                .await
                .inspect_err(|e| self.note_rate_limit(method, key, e));
            timing.http_ms.fetch_add(ms_since(t), Ordering::Relaxed);
            timing.attempts.fetch_add(1, Ordering::Relaxed);
            r
        })
        .retry(retry::backoff(self.cfg.retry_base_delay))
        .when(|e| matches!(e, ApiError::RateLimited { .. }))
        .adjust(retry::delay_for)
        .sleep(tokio::time::sleep)
        .await;
        // Message posts only: phases and milliseconds, never text or ids.
        if method.tier == Tier::PostMessage {
            timing.log(started, out.as_ref().err());
        }
        out
    }

    /// A call whose reply carries nothing but `ok`. `retry` picks the policy explicitly:
    /// [`Retry::Idempotent`] for [`call`](Self::call), [`Retry::Write`] for
    /// [`call_write`](Self::call_write).
    pub(crate) async fn call_ok(
        &self,
        method: Method,
        params: &[(&str, &str)],
        prio: Priority,
        retry: Retry,
    ) -> Result<(), ApiError> {
        match retry {
            Retry::Idempotent => self.call::<IgnoredAny>(method, params, prio).await,
            Retry::Write => self.call_write::<IgnoredAny>(method, params, prio).await,
        }
        .map(|_| ())
    }

    /// A 429 holds the method that got it (server limits are per method).
    fn note_rate_limit(&self, method: Method, key: Option<&str>, e: &ApiError) {
        if let ApiError::RateLimited { retry_after } = e {
            self.limiter.penalize(method.name, key, *retry_after);
        }
    }

    /// POST to the edge cache API like the web client: a JSON body (sent as `text/plain`)
    /// carrying the `token`, at `<edge_base><team>/<path>`. Rate limited and retried like
    /// [`call`](Self::call).
    pub async fn edge_call<R: DeserializeOwned>(
        &self,
        method: Method,
        team: &str,
        path: &str,
        body: serde_json::Value,
        prio: Priority,
    ) -> Result<R, ApiError> {
        (|| async {
            self.limiter
                .acquire(method.name, method.tier, None, prio)
                .await;
            self.send_edge(method, team, path, &body)
                .await
                .inspect_err(|e| self.note_rate_limit(method, None, e))
        })
        .retry(retry::backoff(self.cfg.retry_base_delay))
        .when(retry::is_retryable)
        .adjust(retry::delay_for)
        .sleep(tokio::time::sleep)
        .await
    }

    async fn send_edge<R: DeserializeOwned>(
        &self,
        method: Method,
        team: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<R, ApiError> {
        let bad = || ApiError::Config("bad edge base".to_string());
        // The team goes into the URL path: anything but a team id could change the host.
        if !is_team_id(team) {
            return Err(bad());
        }
        let mut url = self
            .cfg
            .edge_base
            .join(&format!("{team}/{path}"))
            .map_err(|_| bad())?;
        url.query_pairs_mut()
            .append_pair("_x_app_name", "client")
            .append_pair("_x_num_retries", "0");
        let mut payload = body.clone();
        if let Some(o) = payload.as_object_mut() {
            o.insert(
                "token".to_string(),
                serde_json::Value::String(self.token.expose().to_string()),
            );
        }
        let text = serde_json::to_string(&payload).map_err(|e| ApiError::Decode {
            method: method.name,
            pointer: String::new(),
            message: e.to_string(),
        })?;
        let tap = self.recorder.as_ref().map(|_| {
            let mut form = vec![("token".to_string(), self.token.expose().to_string())];
            form.push(("json".to_string(), body.to_string()));
            (url.to_string(), form)
        });
        let resp = self
            .http
            .post(url)
            .header(COOKIE, self.cookie.header_value())
            .header(ORIGIN, ORIGIN_VALUE)
            .header(USER_AGENT, &self.cfg.user_agent)
            .header(CONTENT_TYPE, "text/plain;charset=UTF-8")
            .body(text)
            .send()
            .await
            .map_err(transport)?;
        self.finish(resp, method, &tap).await
    }

    async fn send<R: DeserializeOwned>(
        &self,
        method: Method,
        params: &[(&str, &str)],
    ) -> Result<R, ApiError> {
        let url = self
            .cfg
            .api_base
            .join(method.name)
            .map_err(|_| ApiError::Config("bad api base".to_string()))?;
        let mut fields: Vec<(String, String)> =
            vec![("token".to_string(), self.token.expose().to_string())];
        fields.extend(params.iter().map(|(k, v)| (k.to_string(), v.to_string())));

        let tap = self
            .recorder
            .as_ref()
            .map(|_| (url.to_string(), fields.clone()));
        let req = self
            .http
            .post(url)
            .header(COOKIE, self.cookie.header_value())
            .header(ORIGIN, ORIGIN_VALUE)
            .header(USER_AGENT, &self.cfg.user_agent);
        let mut form = reqwest::multipart::Form::new();
        for (k, v) in fields {
            form = form.text(k, v);
        }
        let req = req.multipart(form);
        let resp = req.send().await.map_err(transport)?;
        self.finish(resp, method, &tap).await
    }

    /// Status, rate-limit and `ok` handling shared by the form and edge calls.
    async fn finish<R: DeserializeOwned>(
        &self,
        resp: reqwest::Response,
        method: Method,
        tap: &Option<(String, Vec<(String, String)>)>,
    ) -> Result<R, ApiError> {
        let status = resp.status();
        self.record(tap, status.as_u16(), "", status.is_success());
        if status.is_redirection() {
            let to_sign_in = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|l| l.contains("sign"));
            return Err(if to_sign_in {
                ApiError::InvalidAuth
            } else {
                ApiError::Http {
                    status: status.as_u16(),
                }
            });
        }
        let retry_after = retry::parse_retry_after(
            resp.headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
        );
        if status.as_u16() == 429 {
            return Err(ApiError::RateLimited { retry_after });
        }
        if !status.is_success() {
            return Err(ApiError::Http {
                status: status.as_u16(),
            });
        }

        let bytes = resp.bytes().await.map_err(transport)?;
        self.record(
            tap,
            status.as_u16(),
            &String::from_utf8_lossy(&bytes),
            false,
        );
        decode_response(method.name, &bytes, retry_after)
    }
}
