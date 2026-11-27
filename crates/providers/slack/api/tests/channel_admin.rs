//! Channel admin calls replay the 2026-11-24 web-client shapes (hand-written
//! fixtures from the captured shapes; see
//! docs/providers/slack/web-client/conversations.md "Channel admin").

use std::time::Duration;

use slack_api::{ApiError, ClientConfig, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/channel_admin/{name}",
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
    String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\"\r\n\r\n{value}\r\n"))
}

fn has(req: &Request, name: &str) -> bool {
    String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\""))
}

async fn mount(
    server: &MockServer,
    m: &str,
    ok: impl Fn(&Request) -> bool + Send + Sync + 'static,
    body: &str,
) {
    Mock::given(method("POST"))
        .and(path(format!("/api/{m}")))
        .and(move |r: &Request| ok(r))
        .respond_with(json(fixture(body)))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn create_sends_the_name_and_validate_name() {
    let server = MockServer::start().await;
    mount(
        &server,
        "conversations.create",
        |r| {
            field(r, "name", "launch-plan")
                && field(r, "validate_name", "true")
                && !has(r, "is_private")
        },
        "create.json",
    )
    .await;
    let c = client(&server)
        .create_channel("launch-plan", false)
        .await
        .unwrap();
    assert_eq!(c.id, "C0000020");
    assert_eq!(c.name.as_deref(), Some("launch-plan"));
    assert!(!c.is_private);
}

#[tokio::test]
async fn create_private_adds_is_private() {
    let server = MockServer::start().await;
    mount(
        &server,
        "conversations.create",
        |r| field(r, "name", "launch-plan") && field(r, "is_private", "true"),
        "create_private.json",
    )
    .await;
    let c = client(&server)
        .create_channel("launch-plan", true)
        .await
        .unwrap();
    assert!(c.is_private);
}

#[tokio::test]
async fn a_taken_name_is_an_api_error() {
    let server = MockServer::start().await;
    mount(
        &server,
        "conversations.create",
        |_| true,
        "create_name_taken.json",
    )
    .await;
    let e = client(&server)
        .create_channel("general", false)
        .await
        .unwrap_err();
    assert!(
        matches!(e, ApiError::Slack { ref error, .. } if error == "name_taken"),
        "{e:?}"
    );
}

#[tokio::test]
async fn join_sends_request_in_background_and_reads_the_channel() {
    let server = MockServer::start().await;
    mount(
        &server,
        "conversations.join",
        |r| field(r, "channel", "C0000022") && field(r, "request_in_background", "true"),
        "join.json",
    )
    .await;
    let c = client(&server).join_channel("C0000022").await.unwrap();
    assert!(c.is_member);
    assert_eq!(
        c.purpose.map(|p| p.value).as_deref(),
        Some("Show work early")
    );
    assert_eq!(c.num_members, Some(4));
}

#[tokio::test]
async fn leave_archive_and_invite() {
    let server = MockServer::start().await;
    mount(
        &server,
        "conversations.leave",
        |r| field(r, "channel", "C0000020"),
        "leave.json",
    )
    .await;
    mount(
        &server,
        "conversations.archive",
        |r| field(r, "channel", "C0000020"),
        "archive.json",
    )
    .await;
    mount(
        &server,
        "conversations.invite",
        |r| field(r, "channel", "C0000020") && field(r, "users", "U0000002,U0000003"),
        "invite.json",
    )
    .await;
    let c = client(&server);
    c.leave_channel("C0000020").await.unwrap();
    c.archive_channel("C0000020").await.unwrap();
    c.invite_to_channel("C0000020", &["U0000002", "U0000003"])
        .await
        .unwrap();
}

#[tokio::test]
async fn rename_topic_and_purpose() {
    let server = MockServer::start().await;
    mount(
        &server,
        "conversations.rename",
        |r| {
            field(r, "channel", "C0000020")
                && field(r, "name", "launch-day")
                && field(r, "validate", "true")
        },
        "rename.json",
    )
    .await;
    mount(
        &server,
        "conversations.setTopic",
        |r| field(r, "channel", "C0000020") && field(r, "topic", "Ship on Friday"),
        "set_topic.json",
    )
    .await;
    mount(
        &server,
        "conversations.setPurpose",
        |r| {
            field(r, "channel", "C0000020")
                && field(r, "purpose", "Plans for the launch")
                && field(r, "name_tagging", "true")
        },
        "set_purpose.json",
    )
    .await;
    let c = client(&server);
    c.rename_channel("C0000020", "launch-day").await.unwrap();
    c.set_channel_topic("C0000020", "Ship on Friday")
        .await
        .unwrap();
    c.set_channel_purpose("C0000020", "Plans for the launch")
        .await
        .unwrap();
}

#[tokio::test]
async fn browse_sends_the_captured_params_and_starts_on_page_one() {
    let server = MockServer::start().await;
    mount(
        &server,
        "search.modules.channels",
        |r| {
            field(r, "module", "channels")
                && field(r, "query", "")
                && field(r, "page", "1")
                && field(r, "count", "50")
                && field(r, "sort", "recommended")
                && field(r, "channel_type", "all")
                && field(r, "exclude_my_channels", "0")
                && field(r, "search_only_my_channels", "false")
                && field(r, "search_context", "desktop_channel_browser")
                && !has(r, "browse")
                && !has(r, "sort_dir")
        },
        "browse.json",
    )
    .await;
    let page = client(&server).browse_channels("", None, 50).await.unwrap();
    assert_eq!(page.items.len(), 2);
    let first = &page.items[0];
    assert_eq!(first.id, "C0000022");
    assert!(!first.is_member);
    assert_eq!(first.num_members, Some(4));
    assert_eq!(
        first.purpose.as_ref().map(|p| p.value.as_str()),
        Some("Show work early")
    );
    assert_eq!(page.next_cursor.as_deref(), Some("2"));
}

#[tokio::test]
async fn members_follow_the_cursor() {
    let server = MockServer::start().await;
    mount(
        &server,
        "conversations.members",
        |r| field(r, "channel", "C0000020") && !has(r, "cursor"),
        "members_p1.json",
    )
    .await;
    mount(
        &server,
        "conversations.members",
        |r| field(r, "channel", "C0000020") && field(r, "cursor", "next-page"),
        "members_p2.json",
    )
    .await;
    let m = client(&server).channel_members("C0000020").await.unwrap();
    assert_eq!(m, ["U0000001", "U0000002", "U0000003"]);
}

#[tokio::test]
async fn already_starred_and_not_starred_are_success() {
    let server = MockServer::start().await;
    mount(&server, "stars.add", |_| true, "stars_already.json").await;
    mount(&server, "stars.remove", |_| true, "stars_not_starred.json").await;
    let c = client(&server);
    c.star_channel("C0000020", true).await.unwrap();
    c.star_channel("C0000020", false).await.unwrap();
}

#[tokio::test]
async fn stars_add_and_remove_name_the_channel() {
    let server = MockServer::start().await;
    mount(
        &server,
        "stars.add",
        |r| field(r, "channel", "C0000020"),
        "stars_add.json",
    )
    .await;
    mount(
        &server,
        "stars.remove",
        |r| field(r, "channel", "C0000020"),
        "stars_remove.json",
    )
    .await;
    let c = client(&server);
    c.star_channel("C0000020", true).await.unwrap();
    c.star_channel("C0000020", false).await.unwrap();
}
