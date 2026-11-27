use std::time::Duration;

use slack_api::{
    ApiError, AuthTest, ClientConfig, Priority, SessionCookie, WebClient, XoxcToken, method,
};
use url::Url;
use wiremock::matchers::{method as http_method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TOKEN: &str = "xoxc-1-2-3-abcdef";
const COOKIE: &str = "xoxd-secretcookie%2Fvalue";

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/auth/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn config(server: &MockServer) -> ClientConfig {
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    cfg.retry_base_delay = Duration::from_millis(10);
    cfg
}

fn client(cfg: ClientConfig) -> WebClient {
    WebClient::new(
        cfg,
        XoxcToken::parse(TOKEN).unwrap(),
        SessionCookie::parse(COOKIE).unwrap(),
    )
    .unwrap()
}

fn body_has(req: &Request, needle: &str) -> bool {
    String::from_utf8_lossy(&req.body).contains(needle)
}

#[tokio::test]
async fn sends_form_token_cookie_origin_ua() {
    let server = MockServer::start().await;
    Mock::given(http_method("POST"))
        .and(path("/api/auth.test"))
        .and(wiremock::matchers::header(
            "cookie",
            format!("d={COOKIE}").as_str(),
        ))
        .and(wiremock::matchers::header(
            "origin",
            "https://app.slack.com",
        ))
        .and(|req: &Request| {
            req.headers.get("user-agent").and_then(|v| v.to_str().ok())
                == Some(slack_api::DEFAULT_USER_AGENT)
        })
        .and(|req: &Request| {
            let ct = req
                .headers
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            ct.starts_with("multipart/form-data")
                && body_has(req, "name=\"token\"")
                && body_has(req, TOKEN)
        })
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("auth_test_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    client(config(&server)).auth_test().await.unwrap();
}

#[tokio::test]
async fn auth_test_parses() {
    let server = MockServer::start().await;
    Mock::given(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("auth_test_ok.json")))
        .mount(&server)
        .await;
    let got: AuthTest = client(config(&server)).auth_test().await.unwrap();
    assert_eq!(got.team_id, "T0000001");
    assert_eq!(got.enterprise_id, None);
    assert!(!got.is_enterprise_install);
}

#[tokio::test]
async fn enterprise_auth_test_parses() {
    let server = MockServer::start().await;
    Mock::given(path("/api/auth.test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(fixture("enterprise_auth_test_ok.json")),
        )
        .mount(&server)
        .await;
    let got = client(config(&server)).auth_test().await.unwrap();
    assert_eq!(got.enterprise_id.as_deref(), Some("E0000001"));
    assert!(got.is_enterprise_install);
}

#[tokio::test]
async fn invalid_auth_maps_to_expired() {
    let server = MockServer::start().await;
    Mock::given(path("/api/auth.test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(fixture("auth_test_invalid_auth.json")),
        )
        .mount(&server)
        .await;
    let err = client(config(&server)).auth_test().await.unwrap_err();
    assert!(matches!(err, ApiError::InvalidAuth));
    assert!(err.is_auth());
}

#[tokio::test]
async fn redirect_to_sign_in_maps_to_invalid_auth() {
    let server = MockServer::start().await;
    Mock::given(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/signin"))
        .mount(&server)
        .await;
    let err = client(config(&server)).auth_test().await.unwrap_err();
    assert!(matches!(err, ApiError::InvalidAuth));
}

#[tokio::test]
async fn rate_limit_reports_retry_after() {
    let server = MockServer::start().await;
    Mock::given(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .mount(&server)
        .await;
    let err = client(config(&server)).auth_test().await.unwrap_err();
    assert!(matches!(
        err,
        ApiError::RateLimited { retry_after } if retry_after == Duration::ZERO
    ));
}

#[tokio::test]
async fn http_error_status() {
    let server = MockServer::start().await;
    Mock::given(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let err = client(config(&server)).auth_test().await.unwrap_err();
    assert!(matches!(err, ApiError::Http { status: 500 }));
}

#[tokio::test]
async fn slack_error_keeps_needed() {
    let server = MockServer::start().await;
    Mock::given(path("/api/auth.test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                r#"{"ok":false,"error":"missing_scope","needed":"channels:read"}"#,
            ),
        )
        .mount(&server)
        .await;
    let err = client(config(&server)).auth_test().await.unwrap_err();
    match err {
        ApiError::Slack {
            method,
            error,
            needed,
        } => {
            assert_eq!(method, "auth.test");
            assert_eq!(error, "missing_scope");
            assert_eq!(needed.as_deref(), Some("channels:read"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn decode_error_has_pointer() {
    let server = MockServer::start().await;
    Mock::given(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"ok":true,"url":"https://example-team.slack.com/","team":"T","user":"u","team_id":5,"user_id":"U1"}"#,
        ))
        .mount(&server)
        .await;
    let err = client(config(&server)).auth_test().await.unwrap_err();
    match err {
        ApiError::Decode {
            method, pointer, ..
        } => {
            assert_eq!(method, "auth.test");
            assert_eq!(pointer, "/team_id");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn generic_call_uses_method_name() {
    let server = MockServer::start().await;
    Mock::given(path("/api/auth.test"))
        .and(|req: &Request| body_has(req, "name=\"a\""))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
        .expect(1)
        .mount(&server)
        .await;
    let v: serde_json::Value = client(config(&server))
        .call(method::AUTH_TEST, &[("a", "b")], Priority::User)
        .await
        .unwrap();
    assert_eq!(v["ok"], true);
}

#[test]
fn secrets_never_formatted() {
    let cfg = ClientConfig::for_workspace(&Url::parse("https://example-team.slack.com/").unwrap());
    let c = client(cfg.clone());
    let all = format!(
        "{c:?} {cfg:?} {:?} {:?}",
        ApiError::InvalidAuth,
        ApiError::Transport("connect".into())
    );
    assert!(!all.contains("xoxc"), "{all}");
    assert!(!all.contains("xoxd"), "{all}");
    assert!(!all.contains("secretcookie"), "{all}");
}

#[tokio::test]
async fn errors_do_not_echo_request() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut cfg =
        ClientConfig::for_workspace(&Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap());
    cfg.retry_base_delay = Duration::from_millis(10);
    let err = client(cfg).auth_test().await.unwrap_err();
    assert!(matches!(err, ApiError::Transport(_)));
    let text = format!("{err} {err:?}");
    assert!(!text.contains("xoxc") && !text.contains("xoxd") && !text.contains("secretcookie"));
    assert!(!text.contains(&port.to_string()), "{text}");
}
