//! Opening a team from its one-time login link without a webview, against wiremock.
//! Hop shapes follow the synthetic fixtures in
//! `crates/providers/slack/api/tests/fixtures/signin-email-code/` (0012, 0013).

use std::sync::{Arc, Mutex};

use slack_api::Secret;
use slack_auth::native::{NativeError, boot_token, open_team};
use url::Url;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const D: &str = "xoxd-abc%2Fdef";

fn link(server: &MockServer, p: &str) -> Secret {
    Secret::new(format!("{}{p}", server.uri()))
}

fn base(server: &MockServer) -> Url {
    Url::parse(&server.uri()).unwrap()
}

fn redirect(to: &str) -> ResponseTemplate {
    ResponseTemplate::new(302).append_header("location", to)
}

#[tokio::test]
async fn follows_the_link_and_collects_the_session_cookie() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/t/workspace1/login/abc123"))
        .respond_with(
            redirect("/ssb/redirect")
                .append_header("set-cookie", format!("d={D}; Path=/"))
                .append_header("set-cookie", "d-s=1700000000; Path=/"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ssb/redirect"))
        .and(header("cookie", format!("d={D}; d-s=1700000000").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
        .mount(&server)
        .await;
    let cookie = open_team(&link(&server, "/t/workspace1/login/abc123"), &base(&server))
        .await
        .unwrap();
    assert_eq!(cookie.d_value(), D);
    assert_eq!(cookie.d_s_value(), Some("1700000000"));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_host_only_cookie_is_not_sent_to_another_host() {
    let server = MockServer::start().await;
    let port = Url::parse(&server.uri()).unwrap().port().unwrap();
    // `127.0.0.1` sets a host-only cookie, then sends the client to `localhost`.
    Mock::given(method("GET"))
        .and(path("/t/workspace1/login/abc123"))
        .respond_with(
            redirect(&format!("http://localhost:{port}/ssb/redirect"))
                .append_header("set-cookie", format!("d={D}; Path=/")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ssb/redirect"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
        .mount(&server)
        .await;
    let cookie = open_team(&link(&server, "/t/workspace1/login/abc123"), &base(&server))
        .await
        .unwrap();
    assert_eq!(cookie.d_value(), D, "still collected");
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 2);
    assert!(
        reqs[1].headers.get("cookie").is_none(),
        "the cookie stays with the host that set it"
    );
}

#[tokio::test]
async fn a_cookie_for_a_domain_the_hop_is_not_on_is_ignored() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/t/workspace1/login/abc123"))
        .respond_with(
            redirect("/ssb/redirect")
                .append_header("set-cookie", format!("d={D}; Domain=.slack.com; Path=/")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ssb/redirect"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
        .mount(&server)
        .await;
    let err = open_team(&link(&server, "/t/workspace1/login/abc123"), &base(&server))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        NativeError::NeedsUser,
        "no session cookie for this host"
    );
    let reqs = server.received_requests().await.unwrap();
    assert!(reqs[1].headers.get("cookie").is_none());
}

#[tokio::test]
async fn stops_after_five_hops() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loop"))
        .respond_with(redirect("/loop"))
        .mount(&server)
        .await;
    let err = open_team(&link(&server, "/loop"), &base(&server))
        .await
        .unwrap_err();
    assert_eq!(err, NativeError::Refused);
    assert!(server.received_requests().await.unwrap().len() <= 6);
}

#[tokio::test]
async fn never_leaves_the_provider_hosts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/t/workspace1/login/abc123"))
        .respond_with(redirect("https://evil.example/steal"))
        .mount(&server)
        .await;
    let err = open_team(&link(&server, "/t/workspace1/login/abc123"), &base(&server))
        .await
        .unwrap_err();
    assert_eq!(err, NativeError::Refused);
    // A link that is not on a provider host is not requested at all.
    let err = open_team(
        &Secret::new("https://evil.example/t/x/login/y".into()),
        &base(&server),
    )
    .await
    .unwrap_err();
    assert_eq!(err, NativeError::Refused);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn sso_two_factor_and_password_pages_need_the_user() {
    for to in [
        "/sso/saml/start",
        "/signin/two_factor",
        "/signin/password",
        "/2fa",
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/t/workspace1/login/abc123"))
            .respond_with(redirect(to))
            .mount(&server)
            .await;
        let err = open_team(&link(&server, "/t/workspace1/login/abc123"), &base(&server))
            .await
            .unwrap_err();
        assert_eq!(err, NativeError::NeedsUser, "{to}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "{to}");
    }
}

#[tokio::test]
async fn a_page_without_a_session_cookie_needs_the_user() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/t/workspace1/login/abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>sign in</html>"))
        .mount(&server)
        .await;
    let err = open_team(&link(&server, "/t/workspace1/login/abc123"), &base(&server))
        .await
        .unwrap_err();
    assert_eq!(err, NativeError::NeedsUser);
}

#[tokio::test]
async fn an_unreachable_host_is_offline() {
    // A port nothing listens on (wiremock pools its servers, so not a dropped one).
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let b = Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    let l = Secret::new(format!("{b}t/workspace1/login/abc123"));
    assert_eq!(open_team(&l, &b).await.unwrap_err(), NativeError::Offline);
}

fn cookie() -> slack_api::SessionCookie {
    slack_api::SessionCookie::parse(D).unwrap()
}

#[tokio::test]
async fn boot_token_reads_the_token_from_the_boot_page() {
    let server = MockServer::start().await;
    let page = r#"<html><script>var boot_data = {"version_ts":1,"api_token":"xoxc-1-2-3-abc","x":"y"};</script></html>"#;
    Mock::given(method("GET"))
        .and(path("/ssb/redirect"))
        .and(header("cookie", format!("d={D}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string(page))
        .mount(&server)
        .await;
    let team = Url::parse(&format!("{}/", server.uri())).unwrap();
    let token = boot_token(&team, &cookie()).await.unwrap();
    assert_eq!(token.expose(), "xoxc-1-2-3-abc");
}

#[tokio::test]
async fn boot_token_without_a_token_is_malformed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ssb/redirect"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"api_token":"nope"}"#))
        .mount(&server)
        .await;
    let team = Url::parse(&format!("{}/", server.uri())).unwrap();
    assert_eq!(
        boot_token(&team, &cookie()).await.unwrap_err(),
        NativeError::Malformed
    );
}

#[tokio::test]
async fn boot_token_redirected_to_sign_in_needs_the_user() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ssb/redirect"))
        .respond_with(redirect("/signin"))
        .mount(&server)
        .await;
    let team = Url::parse(&format!("{}/", server.uri())).unwrap();
    assert_eq!(
        boot_token(&team, &cookie()).await.unwrap_err(),
        NativeError::NeedsUser
    );
}

/// Collects every tracing field the calls emit.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<String>>);

struct Fields<'a>(&'a mut String);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={v:?} ", f.name()));
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, a: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        a.record(&mut Fields(&mut self.0.lock().unwrap()));
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, v: &tracing::span::Record<'_>) {
        v.record(&mut Fields(&mut self.0.lock().unwrap()));
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, e: &tracing::Event<'_>) {
        e.record(&mut Fields(&mut self.0.lock().unwrap()));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn neither_call_logs_a_body_link_or_cookie() {
    let cap = Capture::default();
    let _guard = tracing::subscriber::set_default(cap.clone());
    tracing::info!(probe = "capture works");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/t/workspace1/login/abc123"))
        .respond_with(
            redirect("/ssb/redirect").append_header("set-cookie", format!("d={D}; Path=/")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ssb/redirect"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"api_token":"xoxc-1-2-3-abc"}"#),
        )
        .mount(&server)
        .await;
    let c = open_team(&link(&server, "/t/workspace1/login/abc123"), &base(&server))
        .await
        .unwrap();
    let team = Url::parse(&format!("{}/", server.uri())).unwrap();
    boot_token(&team, &c).await.unwrap();
    let out = cap.0.lock().unwrap().clone();
    assert!(out.contains("capture works"));
    for s in ["abc123", "xoxd-", "xoxc-", "api_token"] {
        assert!(!out.contains(s), "{s} in tracing output");
    }
}
