//! Call surface (the Parla call window) through the Slack provider.

use std::sync::Arc;

use domain::{CallSurface, ChatProvider, ConversationId, ProviderError};
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackProvider, WsOptions};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn json(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
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

async fn provider(server: &MockServer) -> SlackProvider {
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
            r#"{"ok":true,"self":{"id":"U0000001","name":"alice"},"team":{"id":"T0000001","name":"Example","domain":"example","url":"https://app.slack.com/"},"channels":[{"id":"C0000005","name":"general","is_channel":true}],"ims":[]}"#,
        ),
        (
            "client.counts",
            r#"{"ok":true,"channels":[],"mpims":[],"ims":[]}"#,
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
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = std::time::Duration::from_secs(5);
    cfg.retry_base_delay = std::time::Duration::from_millis(10);
    SlackProvider::start_with(
        creds(server),
        cfg,
        WsOptions::default(),
        &tokio::runtime::Handle::current(),
    )
}

#[tokio::test]
async fn call_surface_returns_huddle_url_cookie_suffixes_and_paths() {
    let server = MockServer::start().await;
    let p = provider(&server).await;
    let c = ConversationId(Arc::from("C0000005"));
    let s: CallSurface = p.call_surface(&c).await.unwrap();
    assert_eq!(&*s.url, "https://app.slack.com/huddle/T0000001/C0000005");
    // cookie metadata only; value compared by equality, never printed.
    assert_eq!(s.cookies.len(), 1);
    let ck = &s.cookies[0];
    assert_eq!(&*ck.name, "d");
    assert_eq!(&*ck.domain, ".slack.com");
    assert_eq!(&*ck.path, "/");
    assert!(ck.secure);
    assert!(ck.http_only);
    assert_eq!(ck.value.expose(), "xoxd-secretcookie%2Fvalue");
    assert_eq!(
        &*s.allowed_host_suffixes,
        [Arc::from("slack.com")].as_slice()
    );
    assert_eq!(
        &*s.signed_out_paths,
        [Arc::from("/signin"), Arc::from("/workspace-signin")].as_slice()
    );
}

#[tokio::test]
async fn capabilities_call_surface_is_true() {
    let server = MockServer::start().await;
    let p = provider(&server).await;
    assert!(p.capabilities().call_surface);
}

#[tokio::test]
async fn call_surface_malformed_conversation_id_is_error() {
    let server = MockServer::start().await;
    let p = provider(&server).await;
    let err = p
        .call_surface(&ConversationId(Arc::from("bad/id")))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Other(_)));
}
