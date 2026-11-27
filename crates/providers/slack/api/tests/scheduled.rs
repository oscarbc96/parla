use std::time::Duration;

use slack_api::{ApiError, ClientConfig, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/scheduled/{name}",
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

fn has(req: &Request, name: &str, value: &str) -> bool {
    String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\"\r\n\r\n{value}"))
}

fn lacks(req: &Request, name: &str) -> bool {
    !String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\""))
}

#[tokio::test]
async fn schedule_params_match_documented_form() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduleMessage"))
        .and(|r: &Request| {
            has(r, "channel", "C0000001")
                && has(r, "text", "hello later")
                && has(r, "post_at", "1800000000")
                && lacks(r, "thread_ts")
        })
        .respond_with(json(fixture("schedule_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .schedule_message("C0000001", "hello later", 1_800_000_000, None)
        .await
        .unwrap();
    assert_eq!(r.id, "Q0000001");
    assert_eq!(r.channel_id, "C0000001");
    assert_eq!(r.post_at, 1_800_000_000);
    assert_eq!(r.text, "hello later");
    assert_eq!(r.thread_ts, None);
}

#[tokio::test]
async fn thread_schedule_sends_thread_ts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduleMessage"))
        .and(|r: &Request| has(r, "thread_ts", "1700000000.000100"))
        .respond_with(json(fixture("schedule_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .schedule_message(
            "C0000001",
            "hello later",
            1_800_000_000,
            Some("1700000000.000100"),
        )
        .await
        .unwrap();
    assert_eq!(r.thread_ts.as_deref(), Some("1700000000.000100"));
}

#[tokio::test]
async fn list_follows_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduledMessages.list"))
        .and(|r: &Request| lacks(r, "cursor"))
        .respond_with(json(fixture("list_p1.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduledMessages.list"))
        .and(|r: &Request| has(r, "cursor", "cur2"))
        .respond_with(json(fixture("list_p2.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let (p1, next) = c.scheduled_list(None).await.unwrap();
    assert_eq!(p1.len(), 2);
    assert_eq!(p1[1].thread_ts.as_deref(), Some("1700000000.000100"));
    assert_eq!(next.as_deref(), Some("cur2"));
    let (p2, next) = c.scheduled_list(next.as_deref()).await.unwrap();
    assert_eq!(p2[0].id, "Q0000003");
    assert_eq!(next, None);
}

#[tokio::test]
async fn too_far_keeps_error_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduleMessage"))
        .respond_with(json(fixture("schedule_too_far.json")))
        .mount(&server)
        .await;
    let e = client(&server)
        .schedule_message("C0000001", "x", 9_999_999_999, None)
        .await
        .unwrap_err();
    assert!(
        matches!(e, ApiError::Slack { ref error, .. } if error == "time_too_far"),
        "{e:?}"
    );
}

#[tokio::test]
async fn in_past_keeps_error_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduleMessage"))
        .respond_with(json(fixture("schedule_in_past.json")))
        .mount(&server)
        .await;
    let e = client(&server)
        .schedule_message("C0000001", "x", 1, None)
        .await
        .unwrap_err();
    assert!(
        matches!(e, ApiError::Slack { ref error, .. } if error == "time_in_past"),
        "{e:?}"
    );
}

#[tokio::test]
async fn delete_sends_channel_and_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.deleteScheduledMessage"))
        .and(|r: &Request| {
            has(r, "channel", "C0000001") && has(r, "scheduled_message_id", "Q0000001")
        })
        .respond_with(json(fixture("delete_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .delete_scheduled("C0000001", "Q0000001")
        .await
        .unwrap();
}

#[tokio::test]
async fn null_text_and_metadata_are_tolerated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduledMessages.list"))
        .respond_with(json(fixture("list_nulls.json")))
        .mount(&server)
        .await;
    let (items, next) = client(&server).scheduled_list(None).await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].text, "");
    assert_eq!(next, None);
}

#[tokio::test]
async fn delete_of_gone_item_keeps_error_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.deleteScheduledMessage"))
        .respond_with(json(fixture("delete_gone.json")))
        .mount(&server)
        .await;
    let e = client(&server)
        .delete_scheduled("C0000001", "Q0000001")
        .await
        .unwrap_err();
    assert!(
        matches!(e, ApiError::Slack { ref error, .. } if error == "invalid_scheduled_message_id"),
        "{e:?}"
    );
}

#[tokio::test]
async fn schedule_is_not_resent_after_a_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduleMessage"))
        .respond_with(json(fixture("schedule_ok.json")).set_delay(Duration::from_secs(2)))
        .expect(1)
        .mount(&server)
        .await;
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_millis(200);
    cfg.retry_base_delay = Duration::from_millis(10);
    let c = WebClient::new(
        cfg,
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap();
    assert!(
        c.schedule_message("C0000001", "hi", 1_800_000_000, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn schedule_retries_a_rate_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduleMessage"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduleMessage"))
        .respond_with(json(fixture("schedule_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        client(&server)
            .schedule_message("C0000001", "hi", 1_800_000_000, None)
            .await
            .is_ok()
    );
}
