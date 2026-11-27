//! Channel admin through the provider: the neutral calls reach the right
//! Slack methods and their answers map to neutral conversations (fixtures
//! hand-written from the 2026-11-24 web-client shapes, see
//! docs/providers/slack/web-client/conversations.md "Channel admin").

use std::sync::Arc;
use std::time::Duration;

use domain::{ChatProvider, ConversationId, ConversationKind, ProviderError, UserId};
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackProvider, WsOptions};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use slack_realtime::fake_server::{FakeServer, Script, Step};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/channel_admin/{name}",
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

fn cid(s: &str) -> ConversationId {
    ConversationId(Arc::from(s))
}

async fn expect(server: &MockServer, m: &str, fields: Vec<Field>, body: &str) {
    let mut mock = Mock::given(method("POST")).and(path(format!("/api/{m}")));
    for f in fields {
        mock = mock.and(f);
    }
    mock.respond_with(json(fixture(body)))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_channel_not_held_is_fetched_by_id() {
    let server = MockServer::start().await;
    expect(
        &server,
        "conversations.info",
        vec![Field("channel", "C0000020")],
        "info.json",
    )
    .await;
    let (p, _ws) = provider(&server).await;
    assert!(p.capabilities().conversation_lookup);
    let c = p.conversation(&cid("C0000020")).await.unwrap();
    assert_eq!(&*c.name, "launch-day");
    assert_eq!(c.kind, ConversationKind::Channel);
    assert!(c.is_member);
    assert_eq!(c.member_count, Some(12));
    assert_eq!(&*c.topic, "Weekly notes");
    assert_eq!(&*c.purpose, "Launch planning");
}

#[tokio::test]
async fn creating_a_private_channel_maps_it() {
    let server = MockServer::start().await;
    expect(
        &server,
        "conversations.create",
        vec![Field("name", "launch-plan"), Field("is_private", "true")],
        "create_private.json",
    )
    .await;
    let (p, _ws) = provider(&server).await;
    assert!(p.capabilities().channel_admin);
    let c = p.create_channel("launch-plan", true).await.unwrap();
    assert_eq!(&*c.id.0, "C0000021");
    assert_eq!(c.kind, ConversationKind::PrivateChannel);
    assert!(c.is_member);
}

#[tokio::test]
async fn a_taken_name_reads_as_a_localized_error() {
    let server = MockServer::start().await;
    expect(
        &server,
        "conversations.create",
        vec![],
        "create_name_taken.json",
    )
    .await;
    let (p, _ws) = provider(&server).await;
    match p.create_channel("general", false).await {
        Err(ProviderError::Localized(t)) => assert_eq!(&*t.id, "error-name-taken"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn joining_maps_purpose_topic_and_count() {
    let server = MockServer::start().await;
    expect(
        &server,
        "conversations.join",
        vec![Field("channel", "C0000022")],
        "join.json",
    )
    .await;
    let (p, _ws) = provider(&server).await;
    let c = p.join_conversation(&cid("C0000022")).await.unwrap();
    assert!(c.is_member);
    assert_eq!(&*c.purpose, "Show work early");
    assert_eq!(&*c.topic, "Fridays at noon");
    assert_eq!(c.member_count, Some(4));
}

#[tokio::test]
async fn writes_reach_their_methods() {
    let server = MockServer::start().await;
    expect(
        &server,
        "conversations.leave",
        vec![Field("channel", "C0000020")],
        "leave.json",
    )
    .await;
    expect(
        &server,
        "conversations.archive",
        vec![Field("channel", "C0000020")],
        "archive.json",
    )
    .await;
    expect(
        &server,
        "conversations.rename",
        vec![Field("channel", "C0000020"), Field("name", "launch-day")],
        "rename.json",
    )
    .await;
    expect(
        &server,
        "conversations.setTopic",
        vec![Field("topic", "Ship on Friday")],
        "set_topic.json",
    )
    .await;
    expect(
        &server,
        "conversations.setPurpose",
        vec![Field("purpose", "Plans for the launch")],
        "set_purpose.json",
    )
    .await;
    expect(
        &server,
        "conversations.invite",
        vec![Field("users", "U0000002,U0000003")],
        "invite.json",
    )
    .await;
    expect(
        &server,
        "stars.add",
        vec![Field("channel", "C0000020")],
        "stars_add.json",
    )
    .await;
    let (p, _ws) = provider(&server).await;
    let c = cid("C0000020");
    p.leave_conversation(&c).await.unwrap();
    p.archive_conversation(&c).await.unwrap();
    p.rename_conversation(&c, "launch-day").await.unwrap();
    p.set_topic(&c, "Ship on Friday").await.unwrap();
    p.set_purpose(&c, "Plans for the launch").await.unwrap();
    let report = p
        .invite(&c, &[uid("U0000002"), uid("U0000003")])
        .await
        .unwrap();
    assert_eq!(report.added, vec![uid("U0000002"), uid("U0000003")]);
    assert!(report.failed.is_empty());
    assert!(p.capabilities().conversation_stars);
    p.set_conversation_starred(&c, true).await.unwrap();
}

#[tokio::test]
async fn browsing_pages_by_number() {
    let server = MockServer::start().await;
    expect(
        &server,
        "search.modules.channels",
        vec![
            Field("query", ""),
            Field("page", "1"),
            Field("sort", "recommended"),
        ],
        "browse.json",
    )
    .await;
    let (p, _ws) = provider(&server).await;
    let page = p.browse_channels("", None, 2).await.unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(!page.items[0].is_member);
    assert_eq!(&*page.items[0].purpose, "Show work early");
    assert_eq!(page.items[1].member_count, Some(12));
    assert_eq!(page.next.as_deref(), Some("2"));
}

fn events_of(frame: &str, me: Option<&str>) -> Vec<domain::ProviderEvent> {
    let e = slack_realtime::event::parse_frame(frame).unwrap();
    slack::map_events_as(e, me)
}

/// A live metadata change of `frame` (exactly one event).
fn meta_of(frame: &str, me: Option<&str>) -> domain::ConversationMeta {
    match events_of(frame, me).as_slice() {
        [domain::ProviderEvent::ConversationMetaChanged(m)] => m.clone(),
        other => panic!("{frame}: {other:?}"),
    }
}

#[test]
fn live_renames_archives_and_leaves_change_the_conversation() {
    let m = meta_of(
        r#"{"type":"channel_rename","channel":{"id":"C0000020","name":"launch-day"}}"#,
        None,
    );
    assert_eq!(
        (m.id, m.name.as_deref()),
        (cid("C0000020"), Some("launch-day"))
    );
    let m = meta_of(r#"{"type":"group_archive","channel":"C0000021"}"#, None);
    assert_eq!(m.is_archived, Some(true));
    let m = meta_of(r#"{"type":"channel_unarchive","channel":"C0000020"}"#, None);
    assert_eq!(m.is_archived, Some(false));
    let m = meta_of(r#"{"type":"channel_left","channel":"C0000020"}"#, None);
    assert_eq!(m.is_member, Some(false));
    // The signed-in user's own membership changes the list.
    let m = meta_of(
        r#"{"type":"member_joined_channel","channel":"C0000022","user":"U0000001"}"#,
        Some("U0000001"),
    );
    assert_eq!(m.is_member, Some(true));
}

#[test]
fn another_persons_membership_changes_the_member_list_and_count() {
    // Anyone else's join or leave is a member delta, not ignored (#584).
    match events_of(
        r#"{"type":"member_joined_channel","channel":"C0000022","user":"U0000009"}"#,
        Some("U0000001"),
    )
    .as_slice()
    {
        [
            domain::ProviderEvent::ConversationMemberChanged {
                conversation,
                user,
                joined,
            },
        ] => {
            assert_eq!(conversation, &cid("C0000022"));
            assert_eq!(user, &uid("U0000009"));
            assert!(joined);
        }
        other => panic!("{other:?}"),
    }
    match events_of(
        r#"{"type":"member_left_channel","channel":"C0000022","user":"U0000009"}"#,
        Some("U0000001"),
    )
    .as_slice()
    {
        [domain::ProviderEvent::ConversationMemberChanged { user, joined, .. }] => {
            assert_eq!(user, &uid("U0000009"));
            assert!(!joined);
        }
        other => panic!("{other:?}"),
    }
    // The user leaving is a membership change of the list itself, not a delta.
    let m = meta_of(
        r#"{"type":"member_left_channel","channel":"C0000022","user":"U0000001"}"#,
        Some("U0000001"),
    );
    assert_eq!(m.is_member, Some(false));
}

fn localized(e: &ProviderError) -> &str {
    match e {
        ProviderError::Localized(t) => &t.id,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_batch_with_a_member_already_in_retries_one_by_one() {
    let server = MockServer::start().await;
    expect(
        &server,
        "conversations.invite",
        vec![Field("users", "U0000002,U0000003")],
        "invite_already.json",
    )
    .await;
    expect(
        &server,
        "conversations.invite",
        vec![Field("users", "U0000002")],
        "invite.json",
    )
    .await;
    expect(
        &server,
        "conversations.invite",
        vec![Field("users", "U0000003")],
        "invite_already.json",
    )
    .await;
    let (p, _ws) = provider(&server).await;
    let r = p
        .invite(&cid("C0000020"), &[uid("U0000002"), uid("U0000003")])
        .await
        .unwrap();
    assert_eq!(r.added, vec![uid("U0000002")]);
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].0, uid("U0000003"));
    assert_eq!(localized(&r.failed[0].1), "error-already-in-channel");
}

#[tokio::test]
async fn admin_errors_read_as_clear_text() {
    let server = MockServer::start().await;
    for (m, error) in [
        ("conversations.setTopic", "too_long"),
        ("conversations.setPurpose", "too_long"),
        ("conversations.leave", "not_in_channel"),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/{m}")))
            .respond_with(json(format!(r#"{{"ok":false,"error":"{error}"}}"#)))
            .mount(&server)
            .await;
    }
    for (user, error) in [
        ("U0000001", "cant_invite_self"),
        ("U0000004", "user_is_restricted"),
    ] {
        Mock::given(method("POST"))
            .and(path("/api/conversations.invite"))
            .and(Field("users", user))
            .respond_with(json(format!(r#"{{"ok":false,"error":"{error}"}}"#)))
            .mount(&server)
            .await;
    }
    let (p, _ws) = provider(&server).await;
    let c = cid("C0000020");
    let e = p.set_topic(&c, "x").await.unwrap_err();
    assert_eq!(localized(&e), "error-topic-too-long");
    let e = p.set_purpose(&c, "x").await.unwrap_err();
    assert_eq!(localized(&e), "error-purpose-too-long");
    let e = p.leave_conversation(&c).await.unwrap_err();
    assert_eq!(localized(&e), "error-not-in-channel");
    let r = p.invite(&c, &[uid("U0000001")]).await.unwrap();
    assert_eq!(localized(&r.failed[0].1), "error-cant-invite-self");
    let r = p.invite(&c, &[uid("U0000004")]).await.unwrap();
    assert_eq!(localized(&r.failed[0].1), "error-user-restricted");
}

#[tokio::test]
async fn members_come_from_every_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.members"))
        .and(Field("cursor", "next-page"))
        .respond_with(json(fixture("members_p2.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.members"))
        .respond_with(json(fixture("members_p1.json")))
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    let m = p.conversation_members(&cid("C0000020")).await.unwrap();
    assert_eq!(m, vec![uid("U0000001"), uid("U0000002"), uid("U0000003")]);
}

#[tokio::test]
async fn channel_names_follow_the_workspace_rules() {
    let server = MockServer::start().await;
    let (p, _ws) = provider(&server).await;
    let n = p.check_channel_name("  Launch Plan ");
    assert_eq!((n.name.as_str(), n.problem), ("launch-plan", None));
    assert_eq!(
        p.check_channel_name("   ").problem,
        Some(domain::NameProblem::Empty)
    );
    assert_eq!(
        p.check_channel_name(&"a".repeat(81)).problem,
        Some(domain::NameProblem::TooLong { max: 80 })
    );
    assert_eq!(
        p.check_channel_name("launch!").problem,
        Some(domain::NameProblem::Characters)
    );
}
