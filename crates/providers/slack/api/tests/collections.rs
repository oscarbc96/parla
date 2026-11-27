//! Pins, bookmarks, saved items, the Threads inbox and single-message fetch
//! (synthetic fixtures; the endpoints are unverified, see
//! docs/providers/slack/web-client/messages.md).

use std::time::Duration;

use slack_api::{ApiError, ClientConfig, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(dir: &str, name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{dir}/{name}",
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
    String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\"\r\n\r\n{value}\r\n"))
}

fn lacks(req: &Request, name: &str) -> bool {
    !String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\""))
}

#[tokio::test]
async fn pins_list_add_remove() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/pins.list"))
        .and(|r: &Request| has(r, "channel", "C0000001"))
        .respond_with(json(fixture("pins", "list.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/pins.add"))
        .and(|r: &Request| {
            has(r, "channel", "C0000001") && has(r, "timestamp", "1800000000.000100")
        })
        .respond_with(json(fixture("pins", "ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/pins.remove"))
        .and(|r: &Request| {
            has(r, "channel", "C0000001") && has(r, "timestamp", "1800000000.000100")
        })
        .respond_with(json(fixture("pins", "ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let pins = c.pins_list("C0000001").await.unwrap();
    assert_eq!(pins.len(), 2, "the file pin is skipped");
    assert_eq!(pins[0].ts, "1800000000.000100");
    assert_eq!(pins[0].by.as_deref(), Some("U0000002"));
    assert_eq!(pins[0].at, 1_800_000_100);
    assert_eq!(pins[1].by, None, "no created_by");
    c.pins_add("C0000001", "1800000000.000100").await.unwrap();
    c.pins_remove("C0000001", "1800000000.000100")
        .await
        .unwrap();
}

#[tokio::test]
async fn pin_errors_keep_their_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/pins.add"))
        .respond_with(json(fixture("pins", "already_pinned.json")))
        .mount(&server)
        .await;
    match client(&server).pins_add("C0000001", "1.2").await {
        Err(ApiError::Slack { error, .. }) => assert_eq!(error, "already_pinned"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn bookmarks_list_add_remove() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/bookmarks.list"))
        .and(|r: &Request| has(r, "channel_id", "C0000001"))
        .respond_with(json(fixture("bookmarks", "list.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/bookmarks.add"))
        .and(|r: &Request| {
            has(r, "channel_id", "C0000001")
                && has(r, "title", "Design doc")
                && has(r, "link", "https://example.com/design")
                && has(r, "type", "link")
        })
        .respond_with(json(fixture("bookmarks", "add.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/bookmarks.remove"))
        .and(|r: &Request| has(r, "channel_id", "C0000001") && has(r, "bookmark_id", "Bk0000003"))
        .respond_with(json(fixture("bookmarks", "ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let list = c.bookmarks_list("C0000001").await.unwrap();
    assert_eq!(list.len(), 2, "an entry without an id is skipped");
    assert_eq!(list[0].id, "Bk0000001");
    assert_eq!(list[0].title, "Runbook");
    assert_eq!(list[0].link, "https://example.com/runbook");
    assert_eq!(list[0].emoji.as_deref(), Some(":book:"));
    assert_eq!(list[1].emoji, None);
    let b = c
        .bookmarks_add("C0000001", "Design doc", "https://example.com/design")
        .await
        .unwrap();
    assert_eq!(b.id, "Bk0000003");
    assert_eq!(b.channel, "C0000001");
    c.bookmarks_remove("C0000001", "Bk0000003").await.unwrap();
}

#[tokio::test]
async fn bookmark_add_without_bookmark_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/bookmarks.add"))
        .respond_with(json(fixture("bookmarks", "add_no_bookmark.json")))
        .mount(&server)
        .await;
    assert!(
        client(&server)
            .bookmarks_add("C0000001", "x", "https://example.com")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn saved_list_pages_and_toggles() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/saved.list"))
        .and(|r: &Request| lacks(r, "cursor"))
        .respond_with(json(fixture("saved", "list_p1.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/saved.list"))
        .and(|r: &Request| has(r, "cursor", "c2"))
        .respond_with(json(fixture("saved", "list_p2.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/saved.add"))
        .and(|r: &Request| {
            has(r, "item_type", "message")
                && has(r, "item_id", "C0000001")
                && has(r, "ts", "1800000000.000100")
        })
        .respond_with(json(fixture("saved", "ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/saved.delete"))
        .and(|r: &Request| {
            has(r, "item_type", "message")
                && has(r, "item_id", "C0000001")
                && has(r, "ts", "1800000000.000100")
        })
        .respond_with(json(fixture("saved", "ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let p1 = c.saved_list(None).await.unwrap();
    assert_eq!(p1.next.as_deref(), Some("c2"));
    assert_eq!(p1.items.len(), 2, "the file item is skipped");
    assert_eq!(p1.items[0].channel, "C0000001");
    assert_eq!(p1.items[0].ts, "1800000000.000100");
    assert_eq!(p1.items[0].saved_at, 1_800_000_200);
    assert_eq!(p1.items[0].due, None, "0 means no due date");
    assert!(!p1.items[0].done);
    assert_eq!(p1.items[1].due, Some(1_800_009_999));
    assert!(p1.items[1].done);
    let p2 = c.saved_list(Some("c2")).await.unwrap();
    assert_eq!(p2.next, None, "an empty cursor is the last page");
    assert_eq!(p2.items.len(), 1);
    assert_eq!(p2.items[0].due, None, "null due");
    c.saved_add("C0000001", "1800000000.000100").await.unwrap();
    c.saved_remove("C0000001", "1800000000.000100")
        .await
        .unwrap();
}

#[tokio::test]
async fn inbox_maps_unread_and_mentions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.getView"))
        .and(|r: &Request| lacks(r, "cursor"))
        .respond_with(json(fixture("inbox", "list.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.getView"))
        .and(|r: &Request| has(r, "cursor", "t2"))
        .respond_with(json(fixture("inbox", "last.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let p = c.inbox_list(None).await.unwrap();
    assert_eq!(p.next.as_deref(), Some("t2"));
    assert_eq!(p.threads.len(), 2, "a root without a channel is skipped");
    let t = &p.threads[0];
    assert_eq!(t.channel, "C0000001");
    assert_eq!(t.root_ts, "1800000000.000100");
    assert_eq!(t.latest, "1800000600.000100");
    assert_eq!(t.unread_replies.len(), 2);
    assert!(t.unread_replies[0].text.contains("<@U0000001>"));
    let quiet = &p.threads[1];
    assert!(quiet.unread_replies.is_empty());
    assert_eq!(
        quiet.latest, "1800000150.000100",
        "latest_reply of the root"
    );
    let last = c.inbox_list(Some("t2")).await.unwrap();
    assert!(last.threads.is_empty());
    assert_eq!(last.next, None);
}

#[tokio::test]
async fn message_get_root_and_reply() {
    let server = MockServer::start().await;
    // A root: history with latest=ts, inclusive, limit=1 answers it.
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .and(|r: &Request| {
            has(r, "latest", "1800000000.000100")
                && has(r, "inclusive", "true")
                && has(r, "limit", "1")
        })
        .respond_with(json(fixture("messages", "root.json")))
        .expect(1)
        .mount(&server)
        .await;
    // A reply is not in history: the newest older root comes back, then replies answers.
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .and(|r: &Request| has(r, "latest", "1800000050.000100"))
        .respond_with(json(fixture("messages", "older_instead.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.replies"))
        .and(|r: &Request| has(r, "ts", "1800000050.000100") && has(r, "limit", "1"))
        .respond_with(json(fixture("messages", "reply.json")))
        .expect(1)
        .mount(&server)
        .await;
    // Gone: history is empty and replies has no such thread.
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .and(|r: &Request| has(r, "latest", "1700000000.000100"))
        .respond_with(json(fixture("messages", "empty.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.replies"))
        .and(|r: &Request| has(r, "ts", "1700000000.000100"))
        .respond_with(json(fixture("messages", "not_found.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let root = c
        .message_get("C0000001", "1800000000.000100")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(root.text, "an old message");
    let reply = c
        .message_get("C0000001", "1800000050.000100")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.text, "a reply");
    assert_eq!(reply.thread_ts.as_deref(), Some("1800000000.000100"));
    assert!(
        c.message_get("C0000001", "1700000000.000100")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn message_get_maps_channel_not_found_from_history() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .respond_with(json(
            r#"{"ok":false,"error":"channel_not_found"}"#.to_string(),
        ))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        client(&server)
            .message_get("C0000001", "1700000000.000100")
            .await
            .unwrap()
            .is_none()
    );
}
