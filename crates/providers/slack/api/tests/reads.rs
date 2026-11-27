use std::time::Duration;

use futures::StreamExt;
use slack_api::{ClientConfig, Priority, SessionCookie, WebClient, XoxcToken};
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

#[tokio::test]
async fn pages_follow_cursor_until_empty() {
    let server = MockServer::start().await;
    let p2 = |req: &Request| body(req).contains("dXNlcjpVMDYxTkZUVDI=");
    Mock::given(method("POST"))
        .and(path("/api/users.conversations"))
        .and(p2)
        .respond_with(json(fixture("conversations/users_conversations_p2.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/users.conversations"))
        .respond_with(json(fixture("conversations/users_conversations_p1.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let pages: Vec<_> = c
        .users_conversations("public_channel,im", Priority::Visible)
        .collect()
        .await;
    assert_eq!(pages.len(), 2);
    let ids: Vec<String> = pages
        .into_iter()
        .flat_map(|p| p.unwrap())
        .map(|c| c.id)
        .collect();
    assert_eq!(ids, ["C0000001", "D0000002", "G0000003"]);
}

#[tokio::test]
async fn history_sends_oldest_and_inclusive() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .and(|req: &Request| {
            let b = body(req);
            b.contains("name=\"oldest\"\r\n\r\n1700000000.000100")
                && b.contains("name=\"channel\"\r\n\r\nC0000001")
                && b.contains("name=\"limit\"\r\n\r\n50")
                && !b.contains("name=\"inclusive\"")
                && !b.contains("name=\"latest\"")
        })
        .respond_with(json(fixture("conversations/history_last.json")))
        .expect(1)
        .mount(&server)
        .await;
    let page = client(&server)
        .conversations_history(
            "C0000001",
            None,
            Some("1700000000.000100"),
            50,
            Priority::Visible,
        )
        .await
        .unwrap();
    assert!(!page.has_more);
    assert_eq!(page.messages.len(), 3);
}

#[tokio::test]
async fn unknown_fields_ignored() {
    let server = MockServer::start().await;
    for (p, f) in [
        ("conversations.info", "conversations/info.json"),
        ("users.info", "users/info.json"),
        (
            "conversations.history",
            "conversations/history_has_more.json",
        ),
        ("users.list", "users/list_p1.json"),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/{p}")))
            .respond_with(json(fixture(f)))
            .mount(&server)
            .await;
    }
    let c = client(&server);
    let conv = c.conversations_info("C0000001").await.unwrap();
    assert_eq!(conv.name.as_deref(), Some("general"));
    assert!(conv.is_channel && conv.is_member);
    let u = c.users_info("U0000001").await.unwrap();
    assert_eq!(u.profile.display_name, "Alice");
    assert_eq!(u.tz.as_deref(), Some("Europe/Madrid"));
    let h = c
        .conversations_history("C0000001", None, None, 10, Priority::Visible)
        .await
        .unwrap();
    assert!(h.has_more);
    assert_eq!(
        h.response_metadata.unwrap().next_cursor.as_deref(),
        Some("bmV4dA==")
    );
    let users: Vec<_> = c.users_list(Priority::Background).collect().await;
    let users = users.into_iter().next().unwrap().unwrap();
    assert_eq!(users.len(), 2);
    assert!(users[1].deleted && users[1].is_bot);
}

#[tokio::test]
async fn message_subtypes_parse() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .respond_with(json(fixture("conversations/history_last.json")))
        .mount(&server)
        .await;
    let mut msgs = client(&server)
        .conversations_history("C0000001", None, None, 10, Priority::Visible)
        .await
        .unwrap()
        .messages;
    let sub: Vec<_> = msgs.iter().map(|m| m.subtype.clone().unwrap()).collect();
    assert_eq!(sub, ["channel_join", "thread_broadcast", "me_message"]);
    assert_eq!(msgs[1].thread_ts.as_deref(), Some("1700000000.000010"));
    assert_eq!(msgs[1].reply_count, Some(3));
    assert_eq!(msgs.remove(2).edited.unwrap().ts, "1700000001.000000");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .respond_with(json(fixture("conversations/history_has_more.json")))
        .mount(&server)
        .await;
    let msgs = client(&server)
        .conversations_history("C0000001", None, None, 10, Priority::Visible)
        .await
        .unwrap()
        .messages;
    assert_eq!(msgs[1].subtype.as_deref(), Some("bot_message"));
    assert_eq!(msgs[1].bot_id.as_deref(), Some("B0000001"));
}

#[tokio::test]
async fn blocks_kept_raw() {
    let server = MockServer::start().await;
    let spike: serde_json::Value =
        serde_json::from_str(&fixture("spike/conversations.history.json")).unwrap();
    let resp = spike["response"]["body"]["json"].to_string();
    let raw_blocks =
        serde_json::to_string(&spike["response"]["body"]["json"]["messages"][0]["blocks"]).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .respond_with(json(resp))
        .mount(&server)
        .await;
    let page = client(&server)
        .conversations_history("C0000007", None, None, 28, Priority::Visible)
        .await
        .unwrap();
    assert_eq!(page.messages[0].blocks.as_ref().unwrap().get(), raw_blocks);
}

#[tokio::test]
async fn pages_stop_on_a_repeated_cursor() {
    let server = MockServer::start().await;
    // Every page, with or without a cursor, points at the same next cursor.
    Mock::given(method("POST"))
        .and(path("/api/users.conversations"))
        .respond_with(json(fixture("conversations/users_conversations_p1.json")))
        .mount(&server)
        .await;
    let c = client(&server);
    let pages: Vec<_> = c
        .users_conversations("public_channel", Priority::Visible)
        .collect()
        .await;
    assert_eq!(
        pages.len(),
        2,
        "first page, then the page the cursor led to"
    );
    assert!(pages.iter().all(|p| p.is_ok()));
}
