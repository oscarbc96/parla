use std::time::Duration;

use slack_api::{ApiError, ClientConfig, Message, Priority, SessionCookie, WebClient, XoxcToken};
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

fn has(req: &Request, name: &str, value: &str) -> bool {
    String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\"\r\n\r\n{value}"))
}

fn lacks(req: &Request, name: &str) -> bool {
    !String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\""))
}

#[tokio::test]
async fn replies_send_web_client_params_and_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.replies"))
        .and(|r: &Request| {
            has(r, "channel", "C0000001")
                && has(r, "ts", "1794500969.628009")
                && has(r, "limit", "4")
                && has(r, "inclusive", "true")
                && has(r, "cursor", "abc")
                && lacks(r, "oldest")
        })
        .respond_with(json(fixture("threads/replies_p1.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.replies"))
        .and(|r: &Request| has(r, "oldest", "1794506750.365639") && has(r, "inclusive", "false"))
        .respond_with(json(fixture("threads/replies_last.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let page = c
        .conversations_replies(
            "C0000001",
            "1794500969.628009",
            None,
            4,
            Some("abc"),
            Priority::Visible,
        )
        .await
        .unwrap();
    assert!(page.has_more);
    assert_eq!(page.messages.len(), 5);
    let root = &page.messages[0];
    assert_eq!(root.reply_count, Some(9));
    assert_eq!(root.latest_reply.as_deref(), Some("1794508024.488979"));
    assert_eq!(root.reply_users.as_deref().map(<[_]>::len), Some(2));
    assert_eq!(root.reply_users_count, Some(2));
    let last = c
        .conversations_replies(
            "C0000001",
            "1794500969.628009",
            Some("1794506750.365639"),
            4,
            None,
            Priority::Visible,
        )
        .await
        .unwrap();
    assert!(!last.has_more);
}

#[tokio::test]
async fn history_between_sends_oldest_latest_exclusive() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .and(|r: &Request| {
            has(r, "oldest", "1700000000.000100")
                && has(r, "latest", "1700000009.000000")
                && has(r, "inclusive", "false")
        })
        .respond_with(json(fixture("conversations/history_last.json")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .conversations_history(
            "C0000001",
            Some("1700000009.000000"),
            Some("1700000000.000100"),
            50,
            Priority::Visible,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn reply_broadcast_flag_only_when_asked() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(|r: &Request| {
            has(r, "reply_broadcast", "true")
                && has(r, "thread_ts", "1700000000.000100")
                && has(r, "client_msg_id", "id-1")
        })
        .respond_with(json(fixture("threads/post_reply_broadcast.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(|r: &Request| lacks(r, "reply_broadcast") && has(r, "thread_ts", "1700000000.000100"))
        .respond_with(json(fixture("chat/post_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let p = c
        .post_reply("C0000001", "1700000000.000100", "x", true, "id-1")
        .await
        .unwrap();
    assert_eq!(p.message.subtype.as_deref(), Some("thread_broadcast"));
    c.post_reply("C0000001", "1700000000.000100", "x", false, "id-2")
        .await
        .unwrap();
}

#[tokio::test]
async fn already_reacted_and_no_reaction_are_ok() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reactions.add"))
        .and(|r: &Request| {
            has(r, "channel", "C0000001")
                && has(r, "timestamp", "1700000000.000100")
                && has(r, "name", "tada")
        })
        .respond_with(json(fixture("reactions/already_reacted.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/reactions.remove"))
        .respond_with(json(fixture("reactions/no_reaction.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    c.add_reaction("C0000001", "1700000000.000100", "tada")
        .await
        .unwrap();
    c.remove_reaction("C0000001", "1700000000.000100", "tada")
        .await
        .unwrap();
}

#[tokio::test]
async fn other_reaction_errors_surface() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reactions.add"))
        .respond_with(json(r#"{"ok":false,"error":"invalid_name"}"#.into()))
        .mount(&server)
        .await;
    let e = client(&server)
        .add_reaction("C0000001", "1.2", "x")
        .await
        .unwrap_err();
    assert!(matches!(e, ApiError::Slack { .. }));
}

#[tokio::test]
async fn add_reaction_ok_fixture() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reactions.add"))
        .respond_with(json(fixture("reactions/add_ok.json")))
        .mount(&server)
        .await;
    client(&server)
        .add_reaction("C0000001", "1.2", "tada")
        .await
        .unwrap();
}

#[test]
fn reactions_typed_from_raw() {
    let m: Message = serde_json::from_str(
        r#"{"ts":"1.2","reactions":[{"name":"tada","count":2,"users":["U1","U2"]},{"name":"x","count":1,"users":["U1"]}]}"#,
    )
    .unwrap();
    let r = m.reactions_typed();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].name, "tada");
    assert_eq!(r[0].count, 2);
    assert_eq!(r[0].users, ["U1", "U2"]);
    let bad: Message = serde_json::from_str(r#"{"ts":"1.2","reactions":"nope"}"#).unwrap();
    assert!(bad.reactions_typed().is_empty());
    assert!(Message::default().reactions_typed().is_empty());
}

#[tokio::test]
async fn emoji_list_keeps_aliases() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/emoji.list"))
        .respond_with(json(fixture("emoji/list.json")))
        .expect(1)
        .mount(&server)
        .await;
    let m = client(&server).emoji_list().await.unwrap();
    assert_eq!(m["partying"], "alias:party_example");
    assert!(m["party_example"].starts_with("https://"));
    assert_eq!(m.len(), 3);
}

#[tokio::test]
async fn mark_thread_params_match_capture() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.mark"))
        .and(|r: &Request| {
            has(r, "channel", "C0000001")
                && has(r, "thread_ts", "1700000000.000100")
                && has(r, "ts", "1700000005.000000")
        })
        .respond_with(json(fixture("subscriptions/thread_mark_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .mark_thread("C0000001", "1700000000.000100", "1700000005.000000")
        .await
        .unwrap();
}

// Recorded live from the web client on 2026-11-24: following a thread reads its
// subscription, then adds it with the read cursor; unfollowing removes it.
#[tokio::test]
async fn thread_subscription_reads_last_read() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.get"))
        .and(|r: &Request| {
            has(r, "channel", "C0000001") && has(r, "thread_ts", "1700000000.000100")
        })
        .respond_with(json(fixture("subscriptions/thread_get_followed.json")))
        .expect(1)
        .mount(&server)
        .await;
    let last = client(&server)
        .thread_subscription("C0000001", "1700000000.000100")
        .await
        .unwrap();
    assert_eq!(last.as_deref(), Some("1700000005.000000"));
}

#[tokio::test]
async fn thread_subscription_none_when_not_followed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.get"))
        .respond_with(json(fixture("subscriptions/thread_get_none.json")))
        .mount(&server)
        .await;
    let last = client(&server)
        .thread_subscription("C0000001", "1700000000.000100")
        .await
        .unwrap();
    assert_eq!(last, None);
}

#[tokio::test]
async fn follow_and_unfollow_params_match_capture() {
    let server = MockServer::start().await;
    for (m, f) in [
        ("add", "subscriptions/thread_add_ok.json"),
        ("remove", "subscriptions/thread_remove_ok.json"),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/subscriptions.thread.{m}")))
            .and(|r: &Request| {
                has(r, "channel", "C0000001")
                    && has(r, "thread_ts", "1700000000.000100")
                    && has(r, "last_read", "1700000005.000000")
            })
            .respond_with(json(fixture(f)))
            .expect(1)
            .mount(&server)
            .await;
    }
    let c = client(&server);
    c.follow_thread("C0000001", "1700000000.000100", "1700000005.000000")
        .await
        .unwrap();
    c.unfollow_thread("C0000001", "1700000000.000100", "1700000005.000000")
        .await
        .unwrap();
}

#[tokio::test]
async fn thread_subscription_and_follow_report_api_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.get"))
        .respond_with(json(fixture("subscriptions/thread_get_error.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.add"))
        .respond_with(json(fixture("subscriptions/thread_add_error.json")))
        .mount(&server)
        .await;
    let c = client(&server);
    assert!(
        c.thread_subscription("C0000001", "1700000000.000100")
            .await
            .is_err()
    );
    assert!(
        c.follow_thread("C0000001", "1700000000.000100", "1700000000.000100")
            .await
            .is_err()
    );
}
