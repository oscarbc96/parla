//! Non-idempotent writes are never resent after a 5xx: the write may already have happened.

use std::time::Duration;

use slack_api::{ClientConfig, FileHosts, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

/// A server that answers 503 to `api_path` and expects exactly one request.
async fn failing(api_path: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(api_path))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn post_message_is_sent_once() {
    let s = failing("/api/chat.postMessage").await;
    assert!(
        client(&s)
            .post_message("C1", "hi", None, "id")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn post_reply_is_sent_once() {
    let s = failing("/api/chat.postMessage").await;
    assert!(
        client(&s)
            .post_reply("C1", "1.2", "hi", false, "id")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn upload_complete_is_sent_once() {
    let api = MockServer::start().await;
    let files = MockServer::start().await;
    let body = format!(
        r#"{{"ok":true,"file_id":"F1","upload_url":"{}/up"}}"#,
        files.uri()
    );
    Mock::given(method("POST"))
        .and(path("/api/files.getUploadURLExternal"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/up"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&files)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/files.completeUploadExternal"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&api)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.png");
    std::fs::write(&p, b"x").unwrap();
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&api.uri()).unwrap());
    cfg.retry_base_delay = Duration::from_millis(10);
    cfg.file_hosts = FileHosts::default().with_test_authority(&files.address().to_string());
    let c = WebClient::new(
        cfg,
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap();
    assert!(
        c.upload_files("C1", None, &[(&p, "a.png", None)], None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn pins_add_is_sent_once() {
    let s = failing("/api/pins.add").await;
    assert!(client(&s).pins_add("C1", "1.2").await.is_err());
}

#[tokio::test]
async fn saved_add_is_sent_once() {
    let s = failing("/api/saved.add").await;
    assert!(client(&s).saved_add("C1", "1.2").await.is_err());
}

#[tokio::test]
async fn reminders_complete_is_sent_once() {
    let s = failing("/api/reminders.complete").await;
    assert!(client(&s).reminders_complete("Rm1").await.is_err());
}

#[tokio::test]
async fn reminders_delete_is_sent_once() {
    let s = failing("/api/reminders.delete").await;
    assert!(client(&s).reminders_delete("Rm1").await.is_err());
}

#[tokio::test]
async fn delete_scheduled_is_sent_once() {
    let s = failing("/api/chat.deleteScheduledMessage").await;
    assert!(client(&s).delete_scheduled("C1", "Q1").await.is_err());
}

#[tokio::test]
async fn bookmarks_remove_is_sent_once() {
    let s = failing("/api/bookmarks.remove").await;
    assert!(client(&s).bookmarks_remove("C1", "Bk1").await.is_err());
}
