use settings::WorkspaceKey;
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::*;
use url::Url;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OK: &str = r#"{"ok":true,"url":"https://example.invalid/","team":"Example Team","user":"alice","team_id":"T0000001","user_id":"U0000001","is_enterprise_install":false}"#;
const BAD: &str = r#"{"ok":false,"error":"invalid_auth"}"#;

fn creds(url: &Url, token: &str) -> WorkspaceCredentials {
    WorkspaceCredentials {
        workspace: WorkspaceKey::new("slack", "T0000001"),
        user_id: "U0000001".to_string(),
        url: url.clone(),
        token: XoxcToken::parse(token).unwrap(),
        cookie: SessionCookie::parse("xoxd-synthetic").unwrap(),
    }
}

fn cfg(url: &Url) -> ClientConfig {
    let mut c = ClientConfig::for_workspace(url);
    c.timeout = std::time::Duration::from_secs(2);
    c
}

#[tokio::test]
async fn verify_valid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(OK))
        .mount(&server)
        .await;
    let url = Url::parse(&server.uri()).unwrap();
    let st = verify(&creds(&url, "xoxc-1-2-3-aaaa"), cfg(&url)).await;
    assert!(
        matches!(st, SessionStatus::Valid { ref user_id, ref team_name } if user_id == "U0000001" && team_name == "Example Team"),
        "{st:?}"
    );
}

#[tokio::test]
async fn verify_invalid_auth_is_expired() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(BAD))
        .mount(&server)
        .await;
    let url = Url::parse(&server.uri()).unwrap();
    let st = verify(&creds(&url, "xoxc-1-2-3-aaaa"), cfg(&url)).await;
    assert!(matches!(st, SessionStatus::Expired), "{st:?}");
}

#[tokio::test]
async fn verify_network_error_is_unreachable_not_expired() {
    let server = MockServer::start().await;
    let url = Url::parse(&server.uri()).unwrap();
    drop(server);
    let st = verify(&creds(&url, "xoxc-1-2-3-aaaa"), cfg(&url)).await;
    assert!(matches!(st, SessionStatus::Unreachable(_)), "{st:?}");
}

#[tokio::test]
async fn verify_server_error_is_unreachable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let url = Url::parse(&server.uri()).unwrap();
    let st = verify(&creds(&url, "xoxc-1-2-3-aaaa"), cfg(&url)).await;
    assert!(matches!(st, SessionStatus::Unreachable(_)), "{st:?}");
}

#[tokio::test]
async fn verify_discovered_checks_each_team() {
    let server = MockServer::start().await;
    // Token A is valid, token B is expired; both must send the same cookie.
    Mock::given(method("POST"))
        .and(path("/api/auth.test"))
        .and(body_string_contains("xoxc-1-2-3-aaaa"))
        .and(wiremock::matchers::header("cookie", "d=xoxd-synthetic"))
        .respond_with(ResponseTemplate::new(200).set_body_string(OK))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/auth.test"))
        .and(body_string_contains("xoxc-1-2-3-bbbb"))
        .and(wiremock::matchers::header("cookie", "d=xoxd-synthetic"))
        .respond_with(ResponseTemplate::new(200).set_body_string(BAD))
        .expect(1)
        .mount(&server)
        .await;
    let url = Url::parse(&server.uri()).unwrap();
    let ws = |id: &str, tok: &str| DiscoveredWorkspace {
        workspace: WorkspaceKey::new("slack", id),
        name: id.to_string(),
        url: url.clone(),
        token: XoxcToken::parse(tok).unwrap(),
        enterprise_id: None,
    };
    let cookie = SessionCookie::parse("xoxd-synthetic").unwrap();
    let out = verify_discovered(
        &cookie,
        vec![
            ws("T0000001", "xoxc-1-2-3-aaaa"),
            ws("T0000002", "xoxc-1-2-3-bbbb"),
        ],
    )
    .await;
    assert_eq!(out.len(), 2);
    assert!(matches!(out[0].1, SessionStatus::Valid { .. }));
    assert!(matches!(out[1].1, SessionStatus::Expired));
}
