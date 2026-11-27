//! User groups for `@` completion (synthetic `usergroups.list` fixture; the web
//! client's call is unverified, see docs/providers/slack/web-client/people.md).

use std::time::Duration;

use domain::{ChatProvider, UserGroup};
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
        "{}/../api/tests/fixtures/{name}",
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
async fn user_groups_map_handle_name_count_and_my_membership() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/usergroups.list"))
        .and(Field("include_users", "true"))
        .respond_with(json(fixture("usergroups/list.json")))
        .expect(1)
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    assert!(p.capabilities().user_groups);
    let got = p.user_groups().await.unwrap();
    assert_eq!(
        got,
        vec![
            UserGroup {
                id: "S0000001".into(),
                handle: "eng".into(),
                name: "Engineering".into(),
                member_count: 3,
                is_member: true,
            },
            UserGroup {
                id: "S0000002".into(),
                handle: "design".into(),
                name: "Design crew".into(),
                member_count: 2,
                is_member: false,
            },
        ]
    );
}
