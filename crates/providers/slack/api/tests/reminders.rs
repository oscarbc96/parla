use std::time::Duration;

use slack_api::{ApiError, ClientConfig, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/reminders/{name}",
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

fn short_client(server: &MockServer) -> WebClient {
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_millis(200);
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
async fn add_sends_text_and_time_verbatim() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .and(|r: &Request| {
            has(r, "text", "stretch") && has(r, "time", "in 20 minutes") && lacks(r, "user")
        })
        .respond_with(json(fixture("add.json")))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .reminders_add("stretch", "in 20 minutes", None)
        .await
        .unwrap();
    assert_eq!(r.id, "Rm0000001");
    assert_eq!(r.text, "stretch");
    assert_eq!(r.time, Some(1_800_000_000));
    assert_eq!(r.complete_ts, None, "0 means not completed");
    assert!(!r.recurring);
    assert_eq!(r.user, "U0000001");
}

#[tokio::test]
async fn add_for_other_user_and_recurring_nulls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .and(|r: &Request| has(r, "user", "U0000002"))
        .respond_with(json(fixture("add_recurring.json")))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .reminders_add("standup", "every weekday", Some("U0000002"))
        .await
        .unwrap();
    assert_eq!(r.id, "Rm0000002");
    assert!(r.recurring);
    assert_eq!(r.time, None);
    assert_eq!(r.complete_ts, None);
    assert_eq!(r.user, "U0000002");
}

#[tokio::test]
async fn add_without_reminder_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .respond_with(json(fixture("add_no_reminder.json")))
        .mount(&server)
        .await;
    assert!(
        client(&server)
            .reminders_add("x", "in 1 minute", None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn add_unparsable_time_keeps_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .respond_with(json(fixture("cannot_parse.json")))
        .mount(&server)
        .await;
    match client(&server).reminders_add("x", "soonish", None).await {
        Err(ApiError::Slack { error, .. }) => assert_eq!(error, "cannot_parse"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn list_maps_due_recurring_and_completed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.list"))
        .respond_with(json(fixture("list.json")))
        .expect(1)
        .mount(&server)
        .await;
    let all = client(&server).reminders_list().await.unwrap();
    let ids: Vec<_> = all.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        ["Rm0000001", "Rm0000002", "Rm0000003", "Rm0000004"],
        "empty id dropped"
    );
    assert_eq!(all[0].time, Some(1_800_000_000));
    assert_eq!(all[0].complete_ts, None);
    assert!(all[1].recurring);
    assert_eq!(all[1].time, None);
    assert_eq!(all[2].complete_ts, Some(1_700_000_100));
    // null fields tolerated
    assert_eq!(all[3].text, "");
    assert_eq!(all[3].user, "U0000009", "falls back to the creator");
    assert!(!all[3].recurring);
}

#[tokio::test]
async fn list_null_array_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.list"))
        .respond_with(json(fixture("list_null.json")))
        .mount(&server)
        .await;
    assert!(client(&server).reminders_list().await.unwrap().is_empty());
}

#[tokio::test]
async fn complete_and_delete_params() {
    let server = MockServer::start().await;
    for (p, f) in [
        ("reminders.complete", "complete.json"),
        ("reminders.delete", "delete.json"),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/{p}")))
            .and(|r: &Request| has(r, "reminder", "Rm0000001"))
            .respond_with(json(fixture(f)))
            .expect(1)
            .mount(&server)
            .await;
    }
    let c = client(&server);
    c.reminders_complete("Rm0000001").await.unwrap();
    c.reminders_delete("Rm0000001").await.unwrap();
}

#[tokio::test]
async fn gone_reminder_keeps_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.delete"))
        .respond_with(json(fixture("not_found.json")))
        .mount(&server)
        .await;
    match client(&server).reminders_delete("Rm9").await {
        Err(ApiError::Slack { error, .. }) => assert_eq!(error, "not_found"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn add_is_not_resent_after_a_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .respond_with(json(fixture("add.json")).set_delay(Duration::from_secs(2)))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        short_client(&server)
            .reminders_add("x", "in 1 minute", None)
            .await
            .is_err()
    );
    // `expect(1)` is verified when the server drops.
}

#[tokio::test]
async fn add_is_not_resent_after_a_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        client(&server)
            .reminders_add("x", "in 1 minute", None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn add_retries_a_rate_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .respond_with(json(fixture("add.json")))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        client(&server)
            .reminders_add("x", "in 1 minute", None)
            .await
            .is_ok()
    );
}
