//! Native call join (`rooms.join`) through the Slack provider.

use std::sync::Arc;

use domain::{ChatProvider, ConversationId, ProviderError};
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

fn fixture() -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/huddles/rooms-join-native.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn chan() -> ConversationId {
    ConversationId(Arc::from("C0000005"))
}

async fn mount_join(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/api/rooms.join"))
        .respond_with(json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn capabilities_native_calls_is_true() {
    let server = MockServer::start().await;
    assert!(provider(&server).await.capabilities().native_calls);
}

#[tokio::test]
async fn maps_every_field_and_sends_the_web_client_params() {
    let server = MockServer::start().await;
    let p = provider(&server).await;
    mount_join(&server, fixture()).await;
    let j = p.native_call_join(&chan()).await.unwrap();
    assert_eq!(&*j.meeting_id, "00000000-0000-4000-8000-000000000002");
    assert_eq!(&*j.attendee_id, "00000000-0000-4000-8000-000000000001");
    assert_eq!(&*j.external_user_id, "U0000002");
    assert_eq!(
        j.join_token.expose(),
        "synthetic-join-token-not-a-credential"
    );
    assert_eq!(
        &*j.signaling_url,
        "wss://signal.chime.example.com/control/00000000-0000-4000-8000-000000000002"
    );
    assert_eq!(&*j.audio_host_url, "audio.chime.example.com:3478");
    assert_eq!(
        j.turn_control_url.as_deref(),
        Some("https://turn.chime.example.com/v2/turn_sessions")
    );

    let reqs = server.received_requests().await.unwrap();
    let req = reqs
        .iter()
        .find(|r| r.url.path() == "/api/rooms.join")
        .unwrap();
    let body = String::from_utf8_lossy(&req.body).to_string();
    for (k, v) in [
        ("channel_id", "C0000005"),
        ("regions", "eu-west-3"),
        ("multidevice", "false"),
        ("_x_reason", "rooms-join"),
        ("_x_mode", "online"),
        ("_x_app_name", "client"),
        ("_x_sonic", "true"),
    ] {
        let part = format!("name=\"{k}\"\r\n\r\n{v}\r\n");
        assert!(body.contains(&part), "missing param {k}");
    }
}

#[tokio::test]
async fn missing_required_fields_are_errors_not_panics() {
    for drop in [
        "/call/free_willy/attendee/JoinToken",
        "/call/free_willy/attendee/AttendeeId",
        "/call/free_willy/attendee/ExternalUserId",
        "/call/free_willy/meeting/MeetingId",
        "/call/free_willy/meeting/MediaPlacement/SignalingUrl",
        "/call/free_willy/meeting/MediaPlacement/AudioHostUrl",
        "/call/free_willy",
        "/call",
    ] {
        let mut v: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
        let (parent, key) = drop.rsplit_once('/').unwrap();
        v.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        let server = MockServer::start().await;
        let p = provider(&server).await;
        mount_join(&server, v.to_string()).await;
        let r = p.native_call_join(&chan()).await;
        assert!(
            matches!(r, Err(ProviderError::Other(_))),
            "{drop} must give an error"
        );
    }
}

#[tokio::test]
async fn missing_turn_url_is_none_and_empty_token_is_an_error() {
    let mut v: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    v.pointer_mut("/call/free_willy/meeting/MediaPlacement")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("TurnControlUrl");
    let server = MockServer::start().await;
    let p = provider(&server).await;
    mount_join(&server, v.to_string()).await;
    assert!(
        p.native_call_join(&chan())
            .await
            .unwrap()
            .turn_control_url
            .is_none()
    );

    let mut v: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    *v.pointer_mut("/call/free_willy/attendee/JoinToken")
        .unwrap() = "".into();
    let server = MockServer::start().await;
    let p = provider(&server).await;
    mount_join(&server, v.to_string()).await;
    assert!(p.native_call_join(&chan()).await.is_err());
}

#[tokio::test]
async fn token_never_reaches_debug_or_errors() {
    let server = MockServer::start().await;
    let p = provider(&server).await;
    mount_join(&server, fixture()).await;
    let j = p.native_call_join(&chan()).await.unwrap();
    assert!(!format!("{j:?}").contains("synthetic-join-token"));

    // A failure that echoes a token-shaped field must not leak it either.
    let mut v: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    v.pointer_mut("/call/free_willy/meeting")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("MeetingId");
    let server = MockServer::start().await;
    let p = provider(&server).await;
    mount_join(&server, v.to_string()).await;
    let e = p.native_call_join(&chan()).await.unwrap_err();
    assert!(!format!("{e:?} {e}").contains("synthetic-join-token"));
}

#[tokio::test]
async fn malformed_conversation_id_and_slack_error() {
    let server = MockServer::start().await;
    let p = provider(&server).await;
    assert!(
        p.native_call_join(&ConversationId(Arc::from("bad/id")))
            .await
            .is_err()
    );
    mount_join(
        &server,
        r#"{"ok":false,"error":"not_allowed_token_type"}"#.into(),
    )
    .await;
    assert!(p.native_call_join(&chan()).await.is_err());
}
