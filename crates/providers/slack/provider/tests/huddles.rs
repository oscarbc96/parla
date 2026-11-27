//! Huddles through the provider (synthetic fixtures; the decline call and the active-state
//! source are unverified, see docs/providers/slack/web-client/huddles.md).

use std::sync::Arc;
use std::time::Duration;

use domain::{ChatProvider, ConversationId, HuddleId, ProviderError, ProviderEvent};
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackProvider, WsOptions, map_events_as};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use slack_realtime::event::{SlackEvent, parse_frame};
use slack_realtime::fake_server::FakeServer;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

fn api_fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/huddles/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn frame(name: &str) -> SlackEvent {
    let text = std::fs::read_to_string(format!(
        "{}/../realtime/tests/fixtures/huddles/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    parse_frame(&text).unwrap()
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
            r#"{"ok":true,"self":{"id":"U0000001","name":"alice"},"team":{"id":"T0000001","name":"Example","domain":"example","url":"https://app.slack.com/"},"channels":[{"id":"C0000005","name":"general","is_channel":true},{"id":"C0000009","name":"old","is_channel":true,"is_archived":true}],"ims":[{"id":"D0000012","is_im":true,"user":"U0000002"}]}"#,
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
    let ws = FakeServer::start(vec![]).await;
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    cfg.retry_base_delay = Duration::from_millis(10);
    cfg.edge_base = Url::parse(&format!("{}/cache/", server.uri())).unwrap();
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
async fn running_huddles_load_once_from_the_edge_list() {
    let server = MockServer::start().await;
    let ex: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/../api/tests/fixtures/huddles-active/0205-post.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    Mock::given(method("POST"))
        .and(path("/cache/T0000001/huddles/list"))
        .and(wiremock::matchers::body_json(serde_json::json!({
            "token": "xoxc-1-2-3-abcdef",
        })))
        .respond_with(json(ex["response"]["body"]["json"].to_string()))
        .expect(1)
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    assert!(p.capabilities().huddles);
    let all = p.huddles().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(&*all[0].id.0, "R0000014");
    assert_eq!(&*all[0].conversation.0, "C0000010");
    assert!(!all[0].is_dm);
    assert_eq!(all[0].started, 1_794_995_685);
}

#[tokio::test]
async fn decline_params_match_the_plan_and_a_failure_is_structured() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/huddles.decline"))
        .and(Field("huddle_id", "R0000031"))
        .and(Field("channel_id", "D0000007"))
        .respond_with(json(api_fixture("decline.json")))
        .expect(1)
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    p.decline_huddle(
        &ConversationId(Arc::from("D0000007")),
        &HuddleId(Arc::from("R0000031")),
    )
    .await
    .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/huddles.decline"))
        .respond_with(json(r#"{"ok":false,"error":"invalid_auth"}"#.into()))
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    let e = p
        .decline_huddle(
            &ConversationId(Arc::from("D0000007")),
            &HuddleId(Arc::from("R0000031")),
        )
        .await
        .unwrap_err();
    assert!(matches!(e, ProviderError::SessionExpired), "{e:?}");
}

#[tokio::test]
async fn decline_is_not_resent_after_a_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/huddles.decline"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    let e = p
        .decline_huddle(
            &ConversationId(Arc::from("D0000007")),
            &HuddleId(Arc::from("R0000031")),
        )
        .await
        .unwrap_err();
    assert!(matches!(e, ProviderError::Network(_)), "{e:?}");
}

#[tokio::test]
async fn huddle_url_matches_link_shape() {
    let server = MockServer::start().await;
    let (p, _ws) = provider(&server).await;
    let c = ConversationId(Arc::from("C0000005"));
    assert_eq!(
        p.huddle_url(&c).as_deref(),
        Some("https://app.slack.com/huddle/T0000001/C0000005")
    );
    assert_eq!(
        p.app_url(&c).as_deref(),
        Some("https://app.slack.com/client/T0000001/C0000005")
    );
    assert!(
        p.huddle_url(&ConversationId(Arc::from("bad/id"))).is_none(),
        "a malformed id builds no URL"
    );
}

#[test]
fn map_final_update_is_ended() {
    let ev = map_events_as(frame("room_update_final.json"), Some("U0000001"));
    match ev.as_slice() {
        [
            ProviderEvent::HuddleEnded {
                conversation,
                id,
                ended,
            },
        ] => {
            assert_eq!(&*conversation.0, "C0000005");
            assert_eq!(&*id.0, "R0000021");
            assert_eq!(*ended, 1_790_000_090);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn join_and_leave_update_the_huddle() {
    let ev = map_events_as(frame("room_leave.json"), None);
    match ev.as_slice() {
        [ProviderEvent::HuddleUpdated(h)] => {
            assert_eq!(&*h.id.0, "R0000021");
            assert_eq!(h.participants.len(), 1);
            assert!(h.created_by.is_none());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn ring_only_for_self_invitee() {
    let ev = map_events_as(frame("room_join_ring.json"), Some("U0000001"));
    assert!(matches!(ev[0], ProviderEvent::HuddleUpdated(_)), "{ev:?}");
    match &ev[1] {
        ProviderEvent::HuddleRing {
            conversation,
            huddle,
            from,
        } => {
            assert_eq!(&*conversation.0, "D0000007");
            assert_eq!(&*huddle.0, "R0000020");
            assert_eq!(&*from.0, "U0000002");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(ev.len(), 2);
    for me in [Some("U0000009"), None] {
        let ev = map_events_as(frame("room_join_ring.json"), me);
        assert!(
            ev.iter()
                .all(|e| !matches!(e, ProviderEvent::HuddleRing { .. })),
            "{me:?}"
        );
    }
}

#[test]
fn user_state_and_invite_cancel_map() {
    assert!(matches!(
        map_events_as(frame("user_in_huddle.json"), None).as_slice(),
        [ProviderEvent::InHuddle { active: true, .. }]
    ));
    assert!(matches!(
        map_events_as(frame("user_left_huddle.json"), None).as_slice(),
        [ProviderEvent::InHuddle { active: false, .. }]
    ));
    match map_events_as(frame("invite_cancel.json"), None).as_slice() {
        [ProviderEvent::HuddleRingCancelled { conversation }] => {
            assert_eq!(&*conversation.0, "D0000007");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn user_huddle_changed_only_updates_presence() {
    for name in ["user_in_huddle.json", "user_left_huddle.json"] {
        let evs = map_events_as(frame(name), Some("U0000001"));
        assert!(
            matches!(evs.as_slice(), [ProviderEvent::InHuddle { .. }]),
            "{name}: {evs:?}"
        );
    }
}

#[test]
fn updates_and_leaves_never_ring() {
    for name in ["room_update_final.json", "room_leave.json"] {
        let evs = map_events_as(frame(name), Some("U0000001"));
        assert!(
            !evs.iter()
                .any(|e| matches!(e, ProviderEvent::HuddleRing { .. })),
            "{name}: {evs:?}"
        );
    }
}

fn live_frame(name: &str) -> SlackEvent {
    let text = std::fs::read_to_string(format!(
        "{}/../realtime/tests/fixtures/huddles-live/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    parse_frame(&v["data"]["json"].to_string()).unwrap()
}

/// 2026-11-18 capture: a DM huddle appears and rings the invitee, updates, and ends live.
#[test]
fn live_huddle_appears_rings_updates_and_ends() {
    let ev = map_events_as(live_frame("ws-7087-sh_room_join.json"), Some("U0000014"));
    match ev.as_slice() {
        [
            ProviderEvent::HuddleUpdated(h),
            ProviderEvent::HuddleRing { from, .. },
        ] => {
            assert_eq!(&*h.id.0, "R0000016");
            assert_eq!(&*h.conversation.0, "D0000012");
            assert!(h.is_dm);
            assert_eq!(&*from.0, "U0000013");
        }
        other => panic!("{other:?}"),
    }
    let ev = map_events_as(live_frame("ws-7080-sh_room_update.json"), None);
    assert!(matches!(ev.as_slice(), [ProviderEvent::HuddleUpdated(_)]));
    for f in [
        "ws-7069-sh_room_update.json",
        "ws-7072-ended_huddle_update.json",
    ] {
        let ev = map_events_as(live_frame(f), Some("U0000014"));
        match ev.as_slice() {
            [
                ProviderEvent::HuddleEnded {
                    conversation,
                    id,
                    ended,
                },
            ] => {
                assert_eq!(&*conversation.0, "C0000010");
                assert_eq!(&*id.0, "R0000014");
                assert_eq!(*ended, 1_794_996_137);
            }
            other => panic!("{f}: {other:?}"),
        }
    }
}
