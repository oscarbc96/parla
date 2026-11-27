//! Other people's presence watch, the outbound typing frame and opening a direct
//! message (synthetic fixtures; the `conversations.open` call and `thread_ts` on
//! `user_typing` are unverified, see docs/providers/slack/web-client/).

use std::sync::Arc;
use std::time::Duration;

use domain::{ChatProvider, ConversationId, ConversationKind, MessageTs, UserId};
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackProvider, WsOptions};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use slack_realtime::fake_server::{FakeServer, Script, Step};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

fn api_fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/conversations/{name}",
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
    let ws = FakeServer::start(vec![Script {
        steps: vec![
            Step::Send(r#"{"type":"hello"}"#.into()),
            Step::Sleep(Duration::from_millis(500)),
        ],
        ..Script::default()
    }])
    .await;
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
async fn watching_presence_subscribes_to_exactly_those_users() {
    let server = MockServer::start().await;
    let (p, ws) = provider(&server).await;
    assert!(p.capabilities().presence_watch);
    p.watch_presence(&[uid("U0000002"), uid("U0000003")])
        .await
        .unwrap();
    // Bootstrap lists no direct messages here, so the only frames are ours.
    let frames = ws.wait_received(1).await;
    let sub = frames
        .iter()
        .find(|f| f.contains("\"presence_sub\""))
        .expect("a presence_sub frame");
    let v: serde_json::Value = serde_json::from_str(sub).unwrap();
    assert_eq!(v["ids"], serde_json::json!(["U0000002", "U0000003"]));
}

#[tokio::test]
async fn typing_sends_one_user_typing_frame_with_the_thread_when_set() {
    let server = MockServer::start().await;
    let (p, ws) = provider(&server).await;
    assert!(p.capabilities().typing);
    // The presence frame shows the socket is up.
    p.watch_presence(&[uid("U0000002")]).await.unwrap();
    ws.wait_received(1).await;
    let c = ConversationId(Arc::from("C0000001"));
    p.typing(&c, None);
    p.typing(&c, Some(&MessageTs(Arc::from("1700000000.000100"))));
    let typing = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let got: Vec<String> = ws
                .received()
                .into_iter()
                .filter(|f| f.contains("\"user_typing\""))
                .collect();
            if got.len() >= 2 {
                return got;
            }
            ws.wait_received(ws.received().len() + 1).await;
        }
    })
    .await
    .expect("two user_typing frames");
    let v: Vec<serde_json::Value> = typing
        .iter()
        .map(|f| serde_json::from_str(f).unwrap())
        .collect();
    assert_eq!(v[0]["channel"], "C0000001");
    assert!(v[0].get("thread_ts").is_none());
    assert_eq!(v[1]["channel"], "C0000001");
    assert_eq!(v[1]["thread_ts"], "1700000000.000100");
}

#[tokio::test]
async fn an_empty_watch_set_sends_nothing() {
    let server = MockServer::start().await;
    let (p, ws) = provider(&server).await;
    p.watch_presence(&[]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(ws.received().iter().all(|f| !f.contains("presence_sub")));
}

#[tokio::test]
async fn opening_a_direct_message_maps_the_conversation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.open"))
        .and(Field("users", "U0000002"))
        .and(Field("return_im", "true"))
        .respond_with(json(api_fixture("open.json")))
        .expect(1)
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    assert!(p.capabilities().open_direct);
    let c = p.open_direct(&uid("U0000002")).await.unwrap();
    assert_eq!(&*c.id.0, "D0000009");
    assert_eq!(c.kind, ConversationKind::Direct);
    assert_eq!(c.peer, Some(uid("U0000002")));
    assert!(c.is_open);
}
