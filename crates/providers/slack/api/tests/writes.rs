use std::time::Duration;

use slack_api::{ApiError, ClientConfig, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/chat/{name}",
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

fn field(req: &Request, name: &str, value: &str) -> bool {
    String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\"\r\n\r\n{value}"))
}

#[tokio::test]
async fn post_sends_client_msg_id_and_thread() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(|r: &Request| {
            field(r, "channel", "C0000001")
                && field(r, "text", "hello")
                && field(r, "client_msg_id", "11111111-2222-3333-4444-555555555555")
                && field(r, "thread_ts", "1700000000.000100")
        })
        .respond_with(json(fixture("post_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .post_message(
            "C0000001",
            "hello",
            Some("1700000000.000100"),
            "11111111-2222-3333-4444-555555555555",
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn post_omits_thread_when_none() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(|r: &Request| !String::from_utf8_lossy(&r.body).contains("name=\"thread_ts\""))
        .respond_with(json(fixture("post_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .post_message("C0000001", "hello", None, "id-1")
        .await
        .unwrap();
}

#[tokio::test]
async fn post_returns_ts_and_echo() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .respond_with(json(fixture("post_ok.json")))
        .mount(&server)
        .await;
    let p = client(&server)
        .post_message("C0000001", "hello", None, "id-1")
        .await
        .unwrap();
    assert_eq!(p.channel, "C0000001");
    assert_eq!(p.ts, "1700000100.000200");
    assert_eq!(p.message.text, "hello");
    assert_eq!(
        p.message.client_msg_id.as_deref(),
        Some("11111111-2222-3333-4444-555555555555")
    );
}

#[tokio::test]
async fn edit_error_surfaces() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.update"))
        .respond_with(json(fixture("cant_update_message.json")))
        .mount(&server)
        .await;
    let e = client(&server)
        .update_message("C0000001", "1700000100.000200", "x")
        .await
        .unwrap_err();
    assert!(matches!(e, ApiError::Slack { ref error, .. } if error == "cant_update_message"));
}

#[tokio::test]
async fn edit_sends_ts_and_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.update"))
        .and(|r: &Request| {
            field(r, "channel", "C0000001")
                && field(r, "ts", "1700000100.000200")
                && field(r, "text", "edited")
        })
        .respond_with(json(fixture("update_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .update_message("C0000001", "1700000100.000200", "edited")
        .await
        .unwrap();
}

#[tokio::test]
async fn delete_and_mark_ok() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.delete"))
        .and(|r: &Request| field(r, "channel", "C0000001") && field(r, "ts", "1700000100.000200"))
        .respond_with(json(fixture("delete_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.mark"))
        .and(|r: &Request| field(r, "channel", "C0000001") && field(r, "ts", "1700000100.000200"))
        .respond_with(json(fixture("mark_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    c.delete_message("C0000001", "1700000100.000200")
        .await
        .unwrap();
    c.mark_read("C0000001", "1700000100.000200").await.unwrap();
}
