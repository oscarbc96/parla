use std::time::Duration;

use slack_api::{ClientConfig, Priority, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/huddles/{name}",
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

#[tokio::test]
async fn get_parses_one_room_and_tolerates_none() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/huddles.get"))
        .respond_with(json(fixture("get.json")))
        .mount(&server)
        .await;
    let r = client(&server)
        .huddles_get("R0000030")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.id, "R0000030");
    assert!(r.is_active());
    assert_eq!(
        r.huddle_link.as_deref(),
        Some("https://app.example.com/huddle/T0000001/C0000005")
    );

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/huddles.get"))
        .respond_with(json(r#"{"ok":true,"huddle":null}"#.into()))
        .mount(&server)
        .await;
    assert!(client(&server).huddles_get("R1").await.unwrap().is_none());
}

#[tokio::test]
async fn history_drops_idless_rows_and_signals_a_full_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/huddles.history"))
        .respond_with(json(fixture("history.json")))
        .mount(&server)
        .await;
    let h = client(&server)
        .huddles_history(Priority::Visible)
        .await
        .unwrap();
    assert_eq!(h.rooms.len(), 4);
    assert!(!h.truncated);

    let rows: Vec<String> = (0..30)
        .map(|i| format!(r#"{{"id":"R{i:07}","channels":["C1"]}}"#))
        .collect();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/huddles.history"))
        .respond_with(json(format!(
            r#"{{"ok":true,"huddles":[{}]}}"#,
            rows.join(",")
        )))
        .mount(&server)
        .await;
    let h = client(&server)
        .huddles_history(Priority::Visible)
        .await
        .unwrap();
    assert!(h.truncated, "a full page may hide older rows");
}

fn active_exchange(name: &str) -> serde_json::Value {
    let raw = std::fs::read_to_string(format!(
        "{}/tests/fixtures/huddles-active/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn edge_client(server: &MockServer) -> WebClient {
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    cfg.retry_base_delay = Duration::from_millis(10);
    cfg.edge_base = Url::parse(&format!("{}/cache/", server.uri())).unwrap();
    WebClient::new(
        cfg,
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap()
}

/// Replays the 2026-11-18 `huddles/list` call the web client makes on page load: a body with
/// only the token, no channel ids, and every running huddle in the response.
#[tokio::test]
async fn list_replays_the_edge_call_and_parses_every_running_huddle() {
    let ex = active_exchange("0205-post.json");
    assert_eq!(
        ex["request"]["body"]["json"].as_object().unwrap().len(),
        1,
        "the web client sends only the token"
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/cache/T0000002/huddles/list"))
        .and(wiremock::matchers::query_param("_x_app_name", "client"))
        .and(wiremock::matchers::header(
            "content-type",
            "text/plain;charset=UTF-8",
        ))
        .and(wiremock::matchers::body_json(serde_json::json!({
            "token": "xoxc-1-2-3-abcdef",
        })))
        .respond_with(json(ex["response"]["body"]["json"].to_string()))
        .expect(1)
        .mount(&server)
        .await;
    let rooms = edge_client(&server)
        .huddles_list("T0000002", Priority::Visible)
        .await
        .unwrap();
    assert_eq!(rooms.len(), 1);
    let r = &rooms[0];
    assert_eq!(r.id, "R0000014");
    assert_eq!(r.channels, ["C0000010"]);
    assert_eq!(r.created_by.as_deref(), Some("U0000013"));
    assert_eq!(r.date_start, 1_794_995_685);
    assert!(r.is_active() && !r.is_dm_call);
}

/// The per-channel `huddles/info` response has the same shape; a DM huddle parses as one.
#[tokio::test]
async fn info_entry_shape_parses_a_ringing_dm_huddle() {
    let ex = active_exchange("0002-post.json");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/cache/T0000002/huddles/list"))
        .respond_with(json(ex["response"]["body"]["json"].to_string()))
        .mount(&server)
        .await;
    let rooms = edge_client(&server)
        .huddles_list("T0000002", Priority::Visible)
        .await
        .unwrap();
    let r = &rooms[0];
    assert_eq!(r.id, "R0000011");
    assert_eq!(r.channels, ["D0000012"]);
    assert!(r.is_dm_call && r.is_active());
    assert_eq!(r.pending_invitees, ["U0000013"]);
}

#[tokio::test]
async fn edge_call_rejects_a_team_that_would_leave_the_edge_base() {
    let server = MockServer::start().await;
    let c = edge_client(&server);
    for team in ["//evil.example", "../x", "T1/../../x", "t1", "T1?x=1", ""] {
        let r = c.huddles_list(team, Priority::Visible).await;
        assert!(
            matches!(r, Err(slack_api::ApiError::Config(_))),
            "{team:?}: {r:?}"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
