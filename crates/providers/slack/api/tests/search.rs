use std::time::Duration;

use slack_api::{ClientConfig, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/search/{name}",
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
async fn messages_search_params_match_capture() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/search.modules.messages"))
        .and(|r: &Request| {
            has(r, "module", "messages")
                && has(r, "query", "back in:<#C0000001>")
                && has(r, "count", "20")
                && has(r, "highlight", "true")
                && has(r, "sort", "timestamp")
                && has(r, "sort_dir", "desc")
                && lacks(r, "cursor")
        })
        .respond_with(json(fixture("messages_p1.json")))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .search_messages("back in:<#C0000001>", None, 20)
        .await
        .unwrap();
    assert_eq!(r.items.len(), 3);
    assert_eq!(r.items[0].channel, "C0000001");
    assert_eq!(r.items[0].message.ts, "1794500969.628009");
    assert_eq!(r.items[2].channel, "D0000001");
    assert_eq!(r.next_cursor.as_deref(), Some("cur-2"));
}

#[tokio::test]
async fn paging_follows_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/search.modules.messages"))
        .and(|r: &Request| has(r, "cursor", "cur-2"))
        .respond_with(json(fixture("messages_last.json")))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .search_messages("back", Some("cur-2"), 20)
        .await
        .unwrap();
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.next_cursor, None, "an empty cursor means done");
}

#[tokio::test]
async fn files_people_channels_parse() {
    let server = MockServer::start().await;
    for (m, f) in [
        ("files", "files.json"),
        ("people", "people.json"),
        ("channels", "channels.json"),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/search.modules.{m}")))
            .and(move |r: &Request| has(r, "module", m) && has(r, "query", "q"))
            .respond_with(json(fixture(f)))
            .expect(1)
            .mount(&server)
            .await;
    }
    let c = client(&server);
    let files = c.search_files("q", None, 10).await.unwrap();
    assert_eq!(files.items.len(), 2);
    assert_eq!(files.items[0].file.id, "F0000001");
    assert_eq!(files.items[0].channel.as_deref(), Some("C0000001"));
    assert_eq!(files.items[0].user.as_deref(), Some("U0000002"));
    assert_eq!(files.items[1].channel, None);
    let people = c.search_people("q", 10).await.unwrap();
    assert_eq!(people[0].id, "U0000002");
    assert_eq!(people[1].profile.real_name, "Robo");
    let chans = c.search_channels("q", 10).await.unwrap();
    assert_eq!(chans.len(), 2);
    assert!(chans[1].is_private);
}

#[tokio::test]
async fn hits_without_channel_or_ts_are_dropped_and_count_is_clamped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/search.modules.messages"))
        .and(|r: &Request| has(r, "count", "100"))
        .respond_with(json(
            r#"{"ok":true,"items":[{"channel":{"id":""},"messages":[{"ts":"1.1","text":"a"}]},{"channel":{"id":"C0000001"},"messages":[{"ts":"","text":"b"},{"ts":"1794300000.000300","text":"c"}]}]}"#.into(),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let r = client(&server)
        .search_messages("x", None, 5000)
        .await
        .unwrap();
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.items[0].message.text, "c");
}

/// The web client pages by number: `pagination` carries `page` and
/// `page_count`, not a cursor, and the next page is asked for with `page`.
#[tokio::test]
async fn numbered_pages_become_the_next_cursor() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/search.modules.messages"))
        .and(|r: &Request| lacks(r, "page") && lacks(r, "cursor"))
        .respond_with(json(fixture("messages_paged.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let r = c.search_messages("back", None, 20).await.unwrap();
    assert_eq!(r.items.len(), 1);
    assert_eq!(r.next_cursor.as_deref(), Some("2"), "page 1 of 3");
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/api/search.modules.messages"))
        .and(|r: &Request| has(r, "page", "3") && lacks(r, "cursor"))
        .respond_with(json(
            fixture("messages_paged.json").replace("\"page\":1", "\"page\":3"),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let r = c.search_messages("back", Some("3"), 20).await.unwrap();
    assert_eq!(r.next_cursor, None, "page 3 of 3 is the last");
}
