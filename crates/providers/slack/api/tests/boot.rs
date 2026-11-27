use std::time::Duration;

use slack_api::{ClientConfig, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn client(server: &MockServer) -> WebClient {
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    cfg.retry_base_delay = Duration::from_millis(10);
    WebClient::new(
        cfg,
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap()
}

fn json(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

fn body(req: &Request) -> String {
    String::from_utf8_lossy(&req.body).to_string()
}

async fn mount(server: &MockServer, p: &str, fx: &str) {
    Mock::given(method("POST"))
        .and(path(p))
        .respond_with(json(fixture(fx)))
        .mount(server)
        .await;
}

fn has(b: &str, k: &str, v: &str) -> bool {
    b.contains(&format!("name=\"{k}\"\r\n\r\n{v}\r\n"))
}

#[tokio::test]
async fn user_boot_sends_web_client_params() {
    let server = MockServer::start().await;
    mount(&server, "/api/client.userBoot", "boot/user_boot.json").await;
    client(&server).user_boot().await.unwrap();
    let b = body(&server.received_requests().await.unwrap()[0]);
    assert!(has(&b, "min_channel_updated", "0"), "{b}");
    assert!(has(&b, "_x_reason", "initial-data"), "{b}");
}

#[tokio::test]
async fn user_boot_parses_self_team_channels() {
    let server = MockServer::start().await;
    mount(&server, "/api/client.userBoot", "boot/user_boot.json").await;
    let boot = client(&server).user_boot().await.unwrap();
    assert_eq!(boot.self_user.id, "U0000001");
    assert_eq!(boot.team.domain, "example");
    assert_eq!(boot.team.enterprise_id.as_deref(), Some("E0000001"));
    assert_eq!(boot.channels[0].id, "C0000001");
    assert_eq!(boot.ims[0].id, "D0000001");
    assert!(boot.is_europe);
}

#[tokio::test]
async fn counts_sends_web_client_params() {
    let server = MockServer::start().await;
    mount(&server, "/api/client.counts", "boot/counts.json").await;
    client(&server).counts().await.unwrap();
    let b = body(&server.received_requests().await.unwrap()[0]);
    for k in [
        "thread_counts_by_channel",
        "org_wide_aware",
        "include_file_channels",
        "include_all_unreads",
    ] {
        assert!(has(&b, k, "true"), "{k}: {b}");
    }
}

#[tokio::test]
async fn counts_parse_unread_and_mentions() {
    let server = MockServer::start().await;
    mount(&server, "/api/client.counts", "boot/counts.json").await;
    let c = client(&server).counts().await.unwrap();
    assert_eq!(c.channels[0].mention_count, 3);
    assert!(c.channels[0].has_unreads);
    assert_eq!(c.channels[0].last_read, "1775131052.973139");
    assert!(c.mpims.is_empty());
    assert_eq!(c.ims.len(), 1);
}

#[tokio::test]
async fn counts_parse_captured_spike_fixture() {
    let server = MockServer::start().await;
    let v: serde_json::Value = serde_json::from_str(&fixture("spike/client.counts.json")).unwrap();
    mount_raw(
        &server,
        "/api/client.counts",
        v["response"]["body"]["json"].to_string(),
    )
    .await;
    let c = client(&server).counts().await.unwrap();
    assert!(!c.channels.is_empty());
}

async fn mount_raw(server: &MockServer, p: &str, b: String) {
    Mock::given(method("POST"))
        .and(path(p))
        .respond_with(json(b))
        .mount(server)
        .await;
}

#[tokio::test]
async fn boot_missing_optional_sections_ok() {
    let server = MockServer::start().await;
    mount(&server, "/api/client.userBoot", "boot/user_boot_min.json").await;
    let boot = client(&server).user_boot().await.unwrap();
    assert!(boot.ims.is_empty() && boot.channels.is_empty());
    assert!(!boot.is_europe);
    assert_eq!(boot.team.enterprise_id, None);
}

#[tokio::test]
async fn build_version_returns_recommended() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/api/client.shouldReload",
        "boot/should_reload.json",
    )
    .await;
    let v = client(&server).build_version().await.unwrap();
    assert_eq!(v.as_deref(), Some("1794525555.123"));
}

#[tokio::test]
async fn build_version_absent_is_none() {
    let server = MockServer::start().await;
    mount_raw(&server, "/api/client.shouldReload", r#"{"ok":true}"#.into()).await;
    assert_eq!(client(&server).build_version().await.unwrap(), None);
}
