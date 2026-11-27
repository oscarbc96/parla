use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use slack_api::{
    ApiError, AuthTest, ClientConfig, Priority, RateLimits, SessionCookie, WebClient, XoxcToken,
    method,
};
use url::Url;
use wiremock::matchers::{method as http_method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn client(server: &MockServer) -> WebClient {
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    cfg.retry_base_delay = Duration::from_millis(10);
    cfg.limits = RateLimits::default();
    WebClient::new(
        cfg,
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap()
}

fn ok_body() -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/auth/auth_test_ok.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Responds with the queued templates in order, then repeats the last one.
struct Seq(Vec<ResponseTemplate>, Arc<AtomicUsize>);

impl Respond for Seq {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let i = self.1.fetch_add(1, Ordering::SeqCst);
        self.0[i.min(self.0.len() - 1)].clone()
    }
}

async fn mount(server: &MockServer, seq: Vec<ResponseTemplate>) -> Arc<AtomicUsize> {
    let n = Arc::new(AtomicUsize::new(0));
    Mock::given(http_method("POST"))
        .and(path("/api/auth.test"))
        .respond_with(Seq(seq, n.clone()))
        .mount(server)
        .await;
    n
}

fn ok() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(ok_body(), "application/json")
}

async fn call(c: &WebClient) -> Result<AuthTest, ApiError> {
    c.call(method::AUTH_TEST, &[], Priority::User).await
}

#[tokio::test]
async fn retry_after_is_honoured() {
    let server = MockServer::start().await;
    let n = mount(
        &server,
        vec![
            ResponseTemplate::new(429).insert_header("Retry-After", "1"),
            ok(),
        ],
    )
    .await;
    let start = Instant::now();
    call(&client(&server)).await.unwrap();
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(n.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn ratelimited_body_is_retried() {
    let server = MockServer::start().await;
    let limited = ResponseTemplate::new(200)
        .insert_header("Retry-After", "1")
        .set_body_raw(r#"{"ok":false,"error":"ratelimited"}"#, "application/json");
    let n = mount(&server, vec![limited, ok()]).await;
    let start = Instant::now();
    call(&client(&server)).await.unwrap();
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(n.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn invalid_auth_not_retried() {
    let server = MockServer::start().await;
    let body = r#"{"ok":false,"error":"invalid_auth"}"#;
    let n = mount(
        &server,
        vec![ResponseTemplate::new(200).set_body_raw(body, "application/json")],
    )
    .await;
    assert_eq!(
        call(&client(&server)).await.unwrap_err(),
        ApiError::InvalidAuth
    );
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn other_slack_errors_not_retried() {
    let server = MockServer::start().await;
    let body = r#"{"ok":false,"error":"channel_not_found"}"#;
    let n = mount(
        &server,
        vec![ResponseTemplate::new(200).set_body_raw(body, "application/json")],
    )
    .await;
    assert!(matches!(
        call(&client(&server)).await,
        Err(ApiError::Slack { .. })
    ));
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn gives_up_after_three_retries() {
    let server = MockServer::start().await;
    let n = mount(&server, vec![ResponseTemplate::new(503)]).await;
    assert_eq!(
        call(&client(&server)).await.unwrap_err(),
        ApiError::Http { status: 503 }
    );
    assert_eq!(n.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn client_errors_not_retried() {
    let server = MockServer::start().await;
    let n = mount(&server, vec![ResponseTemplate::new(404)]).await;
    assert_eq!(
        call(&client(&server)).await.unwrap_err(),
        ApiError::Http { status: 404 }
    );
    assert_eq!(n.load(Ordering::SeqCst), 1);
}
