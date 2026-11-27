//! On-demand member lookup and message-carried bot names (synthetic fixtures;
//! `bots.info` unverified, see docs/providers/slack/web-client/messages.md).

use std::sync::Arc;
use std::time::Duration;

use domain::{ChatProvider, ConversationId, ProviderError, ProviderEvent, UserId};
use futures::StreamExt;
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

fn uid(s: &str) -> UserId {
    UserId(Arc::from(s))
}

#[tokio::test]
async fn lookup_resolves_users_and_bots_and_omits_unknown_ids() {
    let server = MockServer::start().await;
    for (p, field, id, body) in [
        ("users.info", "user", "U0000003", "users/info_carol.json"),
        ("users.info", "user", "U0000004", "users/not_found.json"),
        ("bots.info", "bot", "B0000001", "bots/info.json"),
        ("bots.info", "bot", "B0000009", "bots/not_found.json"),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/{p}")))
            .and(Field(field, id))
            .respond_with(json(fixture(body)))
            .mount(&server)
            .await;
    }
    let (p, _ws) = provider(&server).await;
    assert!(p.capabilities().member_lookup);
    let got = p
        .members(&[
            uid("U0000003"),
            uid("U0000004"),
            uid("B0000001"),
            uid("B0000009"),
        ])
        .await
        .unwrap();
    let names: Vec<(&str, &str, bool)> = got
        .iter()
        .map(|m| (&*m.id.0, &*m.display_name, m.is_bot))
        .collect();
    assert_eq!(
        names,
        vec![
            ("U0000003", "Carol C", false),
            ("B0000001", "Build Notifier", true)
        ]
    );
}

#[tokio::test]
async fn lookup_error_is_returned_only_when_every_id_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/users.info"))
        .respond_with(json(fixture("users/not_found.json")))
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    let r = p.members(&[uid("U0000004")]).await;
    assert!(matches!(
        r,
        Err(ProviderError::NotAllowed(_) | ProviderError::Other(_))
    ));
}

#[tokio::test]
async fn bot_messages_announce_their_own_name_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .respond_with(json(fixture("conversations/history_bot.json")))
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    let mut events = p.events();
    let c = ConversationId(Arc::from("C0000001"));
    for _ in 0..2 {
        p.history(&c, None, 10).await.unwrap();
    }
    let mut named = Vec::new();
    let mut icons = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_millis(300), events.next()).await {
        if let ProviderEvent::MemberUpdated(m) = e
            && m.is_bot
        {
            named.push((m.id.0.to_string(), m.display_name.to_string()));
            icons.push((m.id.0.to_string(), m.avatar_for(36.0).cloned()));
        }
    }
    named.sort();
    assert_eq!(
        named,
        vec![
            ("B0000002".to_string(), "Deploy Bot".to_string()),
            ("B0000003".to_string(), "Legacy Hook".to_string()),
        ],
        "bot_profile.name wins over username, one announcement per bot"
    );
    icons.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        icons,
        vec![
            (
                "B0000002".to_string(),
                Some(domain::ImageRef::Web(
                    "https://files.example.test/bot-72.png".into()
                ))
            ),
            ("B0000003".to_string(), None),
        ],
        "a bot's icon comes from bot_profile.icons"
    );
}
