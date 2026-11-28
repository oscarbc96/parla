//! `parla:plugin/http`: outbound HTTPS to granted hosts.
//!
//! Every rule is enforced here, before the backend is called: https only, the
//! grant's host, URL and header sizes, no host-set headers, a 1 MB request
//! body, a clamped timeout, at most 4 in flight and 120 a minute per
//! instance, redirects only to granted hosts (at most 5), no `Set-Cookie`.

use std::sync::Arc;

use domain::Capabilities;
use url::Url;

use super::parla::plugin::http::{Host, Method, Request, Response};
use super::parla::plugin::types;
use crate::backend::{ApiError, HostBackend, HttpMethod, HttpRequest};
use crate::caps::{self, CallContext, Denied, Grant, HostCall, Origin};
use crate::context::HttpLimiter;
use crate::linker::PluginState;
use crate::manifest::PluginId;

/// Longest request URL.
pub const MAX_URL_BYTES: usize = 8 * 1024;
/// Most request headers.
pub const MAX_HEADERS: usize = 32;
/// Longest one header, name and value together.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;
/// Largest request body.
pub const MAX_REQUEST_BODY: usize = 1 << 20;
/// Longest timeout a plugin may ask for.
pub const MAX_TIMEOUT_MS: u32 = 30_000;
/// Largest response body, whatever the memory cap.
pub const MAX_RESPONSE_BODY: usize = 4 << 20;
/// Redirects the host follows.
pub const MAX_REDIRECTS: usize = 5;

/// The fixed part of one `fetch`, owned so no store borrow spans an await.
pub(crate) struct FetchContext {
    pub plugin: PluginId,
    pub grant: Arc<Grant>,
    pub backend: Arc<dyn HostBackend>,
    /// `min(4 MB, a quarter of the guest's memory cap)`.
    pub max_body: usize,
}

impl Host for PluginState {
    async fn fetch(&mut self, request: Request) -> Result<Response, types::Error> {
        let ctx = FetchContext {
            plugin: self.plugin_id().clone(),
            grant: self.grant().clone(),
            backend: self.context().backend.clone(),
            max_body: (self.cap_bytes() / 4).min(MAX_RESPONSE_BODY),
        };
        let limiter = self.http().clone();
        fetch(ctx, limiter, request).await.map_err(Into::into)
    }
}

pub(crate) async fn fetch(
    ctx: FetchContext,
    limiter: Arc<HttpLimiter>,
    request: Request,
) -> Result<Response, ApiError> {
    // One in-flight slot for the whole chain.
    let _permit = limiter.permit()?;
    let mut method = method(request.method);
    let mut url = check_url(&ctx.grant, &request.url)?;
    let headers = check_headers(request.headers)?;
    let mut body = request.body;
    if body.as_ref().is_some_and(|b| b.len() > MAX_REQUEST_BODY) {
        return Err(ApiError::Limit("request body".to_owned()));
    }
    let timeout_ms = clamp_timeout(request.timeout_ms);
    let mut redirects = 0;

    loop {
        limiter.count()?;
        let response = ctx
            .backend
            .http_fetch(
                &ctx.plugin,
                HttpRequest {
                    method,
                    url: url.to_string(),
                    headers: headers.clone(),
                    body: body.clone(),
                    timeout_ms,
                },
                ctx.max_body,
            )
            .await?;
        if response.body.len() > ctx.max_body {
            return Err(ApiError::Limit("response body".to_owned()));
        }
        // Cookies are never kept or handed on.
        let headers: Vec<(String, String)> = response
            .headers
            .into_iter()
            .filter(|(name, _)| !name.eq_ignore_ascii_case("set-cookie"))
            .collect();
        let Some(next) = redirect_target(response.status, &headers, &url) else {
            return Ok(Response {
                status: response.status,
                headers,
                body: response.body,
            });
        };
        if redirects == MAX_REDIRECTS {
            return Err(ApiError::Limit("http redirects".to_owned()));
        }
        redirects += 1;
        url = check_url(&ctx.grant, &next)?;
        if response.status == 303 {
            method = HttpMethod::Get;
            body = None;
        }
    }
}

/// `raw` as an https URL on a granted host.
fn check_url(grant: &Grant, raw: &str) -> Result<Url, ApiError> {
    if raw.len() > MAX_URL_BYTES {
        return Err(ApiError::Invalid("url is too long".to_owned()));
    }
    let url = Url::parse(raw).map_err(|_| ApiError::Invalid("url".to_owned()))?;
    if url.scheme() != "https" {
        return Err(ApiError::Invalid("only https URLs are allowed".to_owned()));
    }
    let provider = Capabilities::default();
    let cx = CallContext {
        provider: &provider,
        conversation: None,
        message: None,
        origin: Origin::Other,
        views: &[],
        workspaces: None,
    };
    caps::check(grant, &HostCall::Http { url: url.clone() }, &cx).map_err(denied)?;
    Ok(url)
}

fn denied(e: Denied) -> ApiError {
    match e {
        Denied::Missing(need) => ApiError::Denied(need.to_string()),
        Denied::NotHttps => ApiError::Invalid("only https URLs are allowed".to_owned()),
        Denied::InvalidUrl => {
            ApiError::Invalid("the URL may not carry a port, user info or an IP address".to_owned())
        }
        other => ApiError::Denied(other.to_string()),
    }
}

/// Refuses `Host`, `Cookie`, `User-Agent` and `Proxy-*`, which the host sets.
fn check_headers(headers: Vec<(String, String)>) -> Result<Vec<(String, String)>, ApiError> {
    if headers.len() > MAX_HEADERS {
        return Err(ApiError::Invalid("too many headers".to_owned()));
    }
    for (name, value) in &headers {
        if name.len() + value.len() > MAX_HEADER_BYTES {
            return Err(ApiError::Invalid("a header is too long".to_owned()));
        }
        if is_host_set(name) {
            return Err(ApiError::Invalid(format!(
                "the host sets the `{name}` header"
            )));
        }
    }
    Ok(headers)
}

fn is_host_set(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(name.as_str(), "host" | "cookie" | "user-agent") || name.starts_with("proxy-")
}

/// `0` means the maximum; anything above it is clamped.
fn clamp_timeout(ms: u32) -> u32 {
    match ms {
        0 => MAX_TIMEOUT_MS,
        ms => ms.min(MAX_TIMEOUT_MS),
    }
}

fn method(method: Method) -> HttpMethod {
    match method {
        Method::Get => HttpMethod::Get,
        Method::Head => HttpMethod::Head,
        Method::Post => HttpMethod::Post,
        Method::Put => HttpMethod::Put,
        Method::Patch => HttpMethod::Patch,
        Method::Delete => HttpMethod::Delete,
    }
}

/// The absolute URL a redirect points at, if this response is one.
fn redirect_target(status: u16, headers: &[(String, String)], base: &Url) -> Option<String> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let location = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| value)?;
    let next = match Url::parse(location) {
        Ok(url) => url,
        Err(_) => base.join(location).ok()?,
    };
    Some(next.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryBackend;
    use crate::manifest::NetworkHost;
    use std::sync::Arc;

    fn ctx(backend: Arc<MemoryBackend>) -> FetchContext {
        let mut grant = Grant::default();
        grant
            .network
            .insert(NetworkHost::parse("api.example.test").unwrap());
        FetchContext {
            plugin: PluginId::parse("test-plugin").unwrap(),
            grant: Arc::new(grant),
            backend,
            max_body: MAX_RESPONSE_BODY,
        }
    }

    fn request(url: &str) -> Request {
        Request {
            method: Method::Get,
            url: url.to_owned(),
            headers: Vec::new(),
            body: None,
            timeout_ms: 1_000,
        }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    fn run(ctx: &FetchContext, request: Request) -> Result<Response, ApiError> {
        let limiter = Arc::new(HttpLimiter::new());
        rt().block_on(fetch(ctx.clone_for_test(), limiter, request))
    }

    impl FetchContext {
        fn clone_for_test(&self) -> FetchContext {
            FetchContext {
                plugin: self.plugin.clone(),
                grant: self.grant.clone(),
                backend: self.backend.clone(),
                max_body: self.max_body,
            }
        }
    }

    #[test]
    fn only_https_is_refused() {
        let backend = Arc::new(MemoryBackend::new());
        let err = run(&ctx(backend.clone()), request("http://api.example.test/x")).unwrap_err();
        assert!(
            matches!(&err, ApiError::Invalid(why) if why.contains("https")),
            "{err:?}"
        );
        assert!(backend.requests().is_empty());
    }

    #[test]
    fn ungranted_host_is_denied() {
        let backend = Arc::new(MemoryBackend::new());
        let err = run(
            &ctx(backend.clone()),
            request("https://evil.example.test/x"),
        )
        .unwrap_err();
        assert_eq!(
            err,
            ApiError::Denied("network: evil.example.test".to_owned())
        );
        assert!(backend.requests().is_empty());
    }

    #[test]
    fn long_url_is_invalid() {
        let backend = Arc::new(MemoryBackend::new());
        let url = format!("https://api.example.test/{}", "x".repeat(MAX_URL_BYTES));
        let err = run(&ctx(backend.clone()), request(&url)).unwrap_err();
        assert!(matches!(err, ApiError::Invalid(_)), "{err:?}");
        assert!(backend.requests().is_empty());
    }

    #[test]
    fn header_limits_are_enforced() {
        let backend = Arc::new(MemoryBackend::new());
        let mut many = request("https://api.example.test/x");
        many.headers = (0..=MAX_HEADERS)
            .map(|i| (format!("x-{i}"), "1".to_owned()))
            .collect();
        let err = run(&ctx(backend.clone()), many).unwrap_err();
        assert!(matches!(err, ApiError::Invalid(_)), "{err:?}");

        let mut long = request("https://api.example.test/x");
        long.headers = vec![("x-long".to_owned(), "v".repeat(MAX_HEADER_BYTES))];
        let err = run(&ctx(backend.clone()), long).unwrap_err();
        assert!(matches!(err, ApiError::Invalid(_)), "{err:?}");
        assert!(backend.requests().is_empty());
    }

    #[test]
    fn host_set_headers_are_refused() {
        for name in ["Host", "cookie", "User-Agent", "Proxy-Authorization"] {
            let backend = Arc::new(MemoryBackend::new());
            let mut req = request("https://api.example.test/x");
            req.headers = vec![(name.to_owned(), "x".to_owned())];
            let err = run(&ctx(backend.clone()), req).unwrap_err();
            assert!(
                matches!(&err, ApiError::Invalid(why) if why.contains(name)),
                "{name}: {err:?}"
            );
            assert!(backend.requests().is_empty(), "{name} reached the backend");
        }
    }

    #[test]
    fn request_body_is_capped() {
        let backend = Arc::new(MemoryBackend::new());
        let mut req = request("https://api.example.test/x");
        req.body = Some(vec![b'x'; MAX_REQUEST_BODY + 1]);
        let err = run(&ctx(backend.clone()), req).unwrap_err();
        assert_eq!(err, ApiError::Limit("request body".to_owned()));
        assert!(backend.requests().is_empty());
    }

    #[test]
    fn timeout_is_clamped() {
        let backend = Arc::new(MemoryBackend::new());
        for (asked, sent) in [(0, MAX_TIMEOUT_MS), (99_999, MAX_TIMEOUT_MS), (500, 500)] {
            let mut req = request("https://api.example.test/x");
            req.timeout_ms = asked;
            run(&ctx(backend.clone()), req).unwrap();
            let seen = backend.requests().pop().unwrap();
            assert_eq!(seen.timeout_ms, sent, "asked {asked}");
        }
    }

    #[test]
    fn response_body_is_capped() {
        let backend = Arc::new(MemoryBackend::new());
        backend.script(200, Vec::new(), vec![b'x'; 11]);
        let mut c = ctx(backend);
        c.max_body = 10;
        let err = run(&c, request("https://api.example.test/x")).unwrap_err();
        assert_eq!(err, ApiError::Limit("response body".to_owned()));
    }

    #[test]
    fn redirects_are_followed_inside_the_grant() {
        let backend = Arc::new(MemoryBackend::new());
        backend.script(
            302,
            vec![("Location".to_owned(), "/next".to_owned())],
            Vec::new(),
        );
        backend.script(200, Vec::new(), b"ok".to_vec());
        let out = run(&ctx(backend.clone()), request("https://api.example.test/x")).unwrap();
        assert_eq!(out.status, 200);
        assert_eq!(out.body, b"ok");
        let requests = backend.requests();
        let urls: Vec<&str> = requests.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://api.example.test/x",
                "https://api.example.test/next"
            ]
        );
    }

    #[test]
    fn too_many_redirects_is_limit() {
        let backend = Arc::new(MemoryBackend::new());
        for _ in 0..=MAX_REDIRECTS {
            backend.script(
                302,
                vec![("Location".to_owned(), "/again".to_owned())],
                Vec::new(),
            );
        }
        let err = run(&ctx(backend.clone()), request("https://api.example.test/x")).unwrap_err();
        assert_eq!(err, ApiError::Limit("http redirects".to_owned()));
        assert_eq!(backend.requests().len(), MAX_REDIRECTS + 1);
    }

    #[test]
    fn redirect_to_an_ungranted_host_is_denied() {
        let backend = Arc::new(MemoryBackend::new());
        backend.script(
            301,
            vec![(
                "Location".to_owned(),
                "https://evil.example.test/x".to_owned(),
            )],
            Vec::new(),
        );
        let err = run(&ctx(backend.clone()), request("https://api.example.test/x")).unwrap_err();
        assert_eq!(
            err,
            ApiError::Denied("network: evil.example.test".to_owned())
        );
        assert_eq!(backend.requests().len(), 1, "the redirect was not fetched");
    }

    #[test]
    fn set_cookie_is_dropped() {
        let backend = Arc::new(MemoryBackend::new());
        backend.script(
            200,
            vec![
                ("Set-Cookie".to_owned(), "a=1".to_owned()),
                ("X-Kept".to_owned(), "yes".to_owned()),
            ],
            Vec::new(),
        );
        let out = run(&ctx(backend), request("https://api.example.test/x")).unwrap();
        assert_eq!(out.headers, vec![("X-Kept".to_owned(), "yes".to_owned())]);
    }

    #[test]
    fn a_hundred_and_twenty_requests_a_minute() {
        let backend = Arc::new(MemoryBackend::new());
        let c = ctx(backend);
        let limiter = Arc::new(HttpLimiter::new());
        rt().block_on(async {
            for _ in 0..crate::context::HTTP_PER_MINUTE {
                fetch(
                    c.clone_for_test(),
                    limiter.clone(),
                    request("https://api.example.test/x"),
                )
                .await
                .unwrap();
            }
            let err = fetch(
                c.clone_for_test(),
                limiter.clone(),
                request("https://api.example.test/x"),
            )
            .await
            .unwrap_err();
            assert_eq!(err, ApiError::Limit("http rate".to_owned()));
        });
    }
}
