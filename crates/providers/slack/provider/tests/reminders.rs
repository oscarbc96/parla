//! Reminders through the provider (synthetic fixtures; endpoints unverified, see
//! docs/providers/slack/web-client/reminders.md).

use std::sync::Arc;
use std::time::Duration;

use domain::{ChatProvider, ProviderError, ReminderId, UserId};
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackProvider, WsOptions};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use slack_realtime::fake_server::FakeServer;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/reminders/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn json(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

struct Field(&'static str, &'static str);

impl Match for Field {
    fn matches(&self, r: &Request) -> bool {
        String::from_utf8_lossy(&r.body)
            .contains(&format!("name=\"{}\"\r\n\r\n{}\r\n", self.0, self.1))
    }
}

fn creds(server: &MockServer) -> WorkspaceCredentials {
    WorkspaceCredentials {
        workspace: WorkspaceKey {
            provider: ProviderId("slack".into()),
            id: WorkspaceId("T0000001".into()),
        },
        user_id: "U0000001".into(),
        url: Url::parse(&server.uri()).unwrap(),
        token: XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        cookie: SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    }
}

async fn provider(server: &MockServer) -> (SlackProvider, FakeServer) {
    for (p, body) in [
        (
            "client.shouldReload",
            r#"{"ok":true,"should_reload":false,"recommended_version":"1794525555.123"}"#,
        ),
        (
            "auth.test",
            r#"{"ok":true,"url":"https://example.slack.com/","team":"Example","user":"alice","team_id":"T0000001","user_id":"U0000001"}"#,
        ),
        (
            "client.userBoot",
            r#"{"ok":true,"self":{"id":"U0000001","name":"alice"},"team":{"id":"T0000001","name":"Example","domain":"example","url":"https://app.slack.com/"},"channels":[],"ims":[]}"#,
        ),
        (
            "users.list",
            r#"{"ok":true,"members":[],"response_metadata":{"next_cursor":""}}"#,
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/{p}")))
            .respond_with(json(body.to_string()))
            .mount(server)
            .await;
    }
    let ws = FakeServer::start(vec![]).await;
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    cfg.retry_base_delay = Duration::from_millis(10);
    let p = SlackProvider::start_with(
        creds(server),
        cfg,
        WsOptions {
            base: Some(ws.url()),
            conn: None,
        },
        &tokio::runtime::Handle::current(),
    );
    (p, ws)
}

#[tokio::test]
async fn list_and_add_map_to_domain() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.list"))
        .respond_with(json(fixture("list.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .and(Field("time", "in 20 minutes"))
        .and(Field("user", "U0000002"))
        .respond_with(json(fixture("add_recurring.json")))
        .expect(1)
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    assert!(
        !p.capabilities().reminders,
        "off until a capture shows the session may use the routes"
    );

    let all = p.reminders().await.unwrap();
    assert_eq!(all.len(), 4, "the entry without an id is dropped");
    assert_eq!(&*all[0].id.0, "Rm0000001");
    assert_eq!(all[0].due, Some(1_800_000_000));
    assert!(!all[0].completed);
    assert!(all[1].recurring && all[1].due.is_none());
    assert!(all[2].completed);

    let r = p
        .add_reminder(
            "standup",
            "in 20 minutes",
            Some(&UserId(Arc::from("U0000002"))),
        )
        .await
        .unwrap();
    assert_eq!(&*r.id.0, "Rm0000002");
    assert_eq!(&*r.for_user.0, "U0000002");
}

#[tokio::test]
async fn complete_and_delete_send_the_id_and_gone_is_not_allowed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.complete"))
        .and(Field("reminder", "Rm0000001"))
        .respond_with(json(fixture("complete.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.delete"))
        .respond_with(json(fixture("not_found.json")))
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    p.complete_reminder(&ReminderId(Arc::from("Rm0000001")))
        .await
        .unwrap();
    assert!(matches!(
        p.delete_reminder(&ReminderId(Arc::from("Rm0000009"))).await,
        Err(ProviderError::NotAllowed(e)) if e == "not_found"
    ));
}

#[tokio::test]
async fn unparsable_time_is_a_structured_error_with_its_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/reminders.add"))
        .respond_with(json(fixture("cannot_parse.json")))
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    match p.add_reminder("x", "soonish", None).await {
        Err(ProviderError::Other(m)) => assert!(m.contains("cannot_parse"), "{m}"),
        other => panic!("{other:?}"),
    }
}
