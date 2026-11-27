//! Provider tests: wiremock HTTP plus the in-process fake websocket server.

use std::sync::Arc;
use std::time::Duration;

use domain::{ChatProvider, ClientMsgId, ConversationId, ConversationKind, ProviderEvent};
use futures::StreamExt;
use futures::stream::BoxStream;
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackMessageExt, SlackProvider, WsOptions};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use slack_realtime::fake_server::{FakeServer, Script, Step};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

const T0: &str = "1700000000.000001";

fn key() -> WorkspaceKey {
    WorkspaceKey {
        provider: ProviderId("slack".into()),
        id: WorkspaceId("T0000001".into()),
    }
}

fn creds(server: &MockServer) -> WorkspaceCredentials {
    WorkspaceCredentials {
        workspace: key(),
        user_id: "U0000001".into(),
        url: Url::parse(&server.uri()).unwrap(),
        token: XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        cookie: SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    }
}

fn cfg(server: &MockServer) -> ClientConfig {
    let mut c = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    c.timeout = Duration::from_secs(5);
    c.retry_base_delay = Duration::from_millis(10);
    c
}

fn ws(server: Option<&FakeServer>) -> WsOptions {
    WsOptions {
        base: server.map(FakeServer::url),
        conn: Some(Arc::new(|c: &mut slack_realtime::conn::ConnConfig| {
            c.backoff_min = Duration::from_millis(10);
            c.backoff_max = Duration::from_millis(20);
        })),
    }
}

fn json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json")
}

struct Field(&'static str, String);

impl Match for Field {
    fn matches(&self, r: &Request) -> bool {
        let b = String::from_utf8_lossy(&r.body);
        b.contains(&format!("name=\"{}\"\r\n\r\n{}\r\n", self.0, self.1))
    }
}

const BOOT: &str = r#"{"ok":true,"self":{"id":"U0000001","name":"alice"},
"team":{"id":"T0000001","name":"Example","domain":"example","url":"https://app.slack.com/"},
"channels":[{"id":"C1","name":"general","is_channel":true},
{"id":"C2","name":"random","is_channel":true,"is_member":true},
{"id":"C3","name":"secret","is_group":true,"is_private":true,"is_member":true}],
"ims":[{"id":"D1","is_im":true,"user":"U0000002"}]}"#;

const USERS: &str = r#"{"ok":true,"members":[
{"id":"U0000001","name":"alice","profile":{"display_name":"Alice","real_name":"Alice A"}},
{"id":"U0000002","name":"bob","profile":{"display_name":"","real_name":"Bob B"}}],
"response_metadata":{"next_cursor":""}}"#;

fn counts(c1_latest: &str) -> String {
    format!(
        r#"{{"ok":true,"channels":[
{{"id":"C1","last_read":"{T0}","latest":"{c1_latest}","mention_count":2,"has_unreads":true}},
{{"id":"C2","last_read":"{T0}","latest":"{T0}","mention_count":0,"has_unreads":false}},
{{"id":"C3","last_read":"{T0}","latest":"{T0}","mention_count":0,"has_unreads":false}}],
"mpims":[],"ims":[{{"id":"D1","last_read":"0000000000.000000","latest":"0000000000.000000","mention_count":0,"has_unreads":false}}]}}"#
    )
}

async fn mount(server: &MockServer, p: &str, body: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/api/{p}")))
        .respond_with(json(body))
        .mount(server)
        .await;
}

async fn base_mocks(server: &MockServer) {
    mount(
        server,
        "client.shouldReload",
        r#"{"ok":true,"should_reload":false,"recommended_version":"1794525555.123"}"#,
    )
    .await;
    mount(server, "auth.test", r#"{"ok":true,"url":"https://example.slack.com/","team":"Example","user":"alice","team_id":"T0000001","user_id":"U0000001"}"#).await;
    mount(server, "client.userBoot", BOOT).await;
    mount(server, "users.list", USERS).await;
}

fn msg_frame(channel: &str, ts: &str, text: &str) -> Step {
    Step::Send(format!(
        r#"{{"type":"message","channel":"{channel}","user":"U0000002","text":"{text}","ts":"{ts}"}}"#
    ))
}

fn hello() -> Step {
    Step::Send(r#"{"type":"hello"}"#.into())
}

async fn next(s: &mut BoxStream<'static, ProviderEvent>) -> ProviderEvent {
    tokio::time::timeout(Duration::from_secs(10), s.next())
        .await
        .expect("event in time")
        .expect("stream open")
}

/// Events until `n` MessagePosted arrived; returns their timestamps.
async fn posted(s: &mut BoxStream<'static, ProviderEvent>, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    while out.len() < n {
        if let ProviderEvent::MessagePosted(m) = next(s).await {
            out.push(m.id.ts.0.to_string());
        }
    }
    out
}

async fn history_requests(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/api/conversations.history")
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect()
}

// ---- mapping -------------------------------------------------------------

#[test]
fn map_message_keeps_slack_fields_in_ext() {
    let m: slack_api::Message = serde_json::from_str(
        r#"{"type":"message","subtype":"bot_message","bot_id":"B1","ts":"1700000000.000200",
"text":"hi <@U0000002>","thread_ts":"1700000000.000100","reply_count":3,"client_msg_id":"c-1",
"edited":{"user":"U1","ts":"1700000000.000300"},
"blocks":[{"type":"section"}],"files":[{"id":"F1"}],"attachments":[{"text":"a"}],
"reactions":[{"name":"thumbsup","users":["U0000001","U0000002"],"count":2}]}"#,
    )
    .unwrap();
    let d = slack::map_message("C1", m);
    assert_eq!(d.id.conversation, ConversationId("C1".into()));
    assert_eq!(&*d.id.ts.0, "1700000000.000200");
    assert_eq!(
        d.thread_root.as_ref().map(|t| &*t.0),
        Some("1700000000.000100")
    );
    assert_eq!(d.reply_count, 3);
    assert_eq!(d.edited.as_ref().map(|t| &*t.0), Some("1700000000.000300"));
    assert_eq!(d.client_msg_id, Some(ClientMsgId("c-1".into())));
    assert_eq!(d.reactions.len(), 1);
    assert_eq!(d.reactions[0].count, 2);
    assert_eq!(d.mentions.len(), 1);
    let ext = d.ext.get::<SlackMessageExt>().expect("slack ext");
    assert_eq!(ext.subtype.as_deref(), Some("bot_message"));
    assert_eq!(ext.bot_id.as_deref(), Some("B1"));
    assert!(ext.blocks.as_ref().unwrap().get().contains("section"));
    assert!(ext.files.as_ref().unwrap().get().contains("F1"));
    assert!(ext.attachments.is_some());
}

#[test]
fn thread_root_message_is_not_a_reply() {
    let m: slack_api::Message = serde_json::from_str(
        r#"{"type":"message","user":"U1","text":"x","ts":"1700000000.000100","thread_ts":"1700000000.000100","reply_count":1}"#,
    )
    .unwrap();
    assert!(slack::map_message("C1", m).thread_root.is_none());
}

#[test]
fn map_dm_uses_peer_name() {
    let c: slack_api::Conversation =
        serde_json::from_str(r#"{"id":"D1","is_im":true,"user":"U0000002"}"#).unwrap();
    let count = slack_api::ChannelCount {
        id: "D1".into(),
        last_read: "1700000000.000100".into(),
        latest: "1700000000.000200".into(),
        mention_count: 1,
        has_unreads: true,
    };
    let names = |id: &str| (id == "U0000002").then(|| Arc::<str>::from("Bob B"));
    let d = slack::map_conversation(&c, Some(&count), &names);
    assert_eq!(d.kind, ConversationKind::Direct);
    assert_eq!(&*d.name, "Bob B");
    assert_eq!(d.peer.as_ref().map(|p| &*p.0), Some("U0000002"));
    assert!(d.is_member);
    assert!(d.unread);
    assert_eq!(d.mentions, 1);
    assert_eq!(d.latest.as_ref().map(|t| &*t.0), Some("1700000000.000200"));
}

#[test]
fn map_conversation_kinds_and_zero_counts() {
    let none = |_: &str| None;
    let p: slack_api::Conversation = serde_json::from_str(
        r#"{"id":"C3","name":"secret","is_group":true,"is_private":true,"is_member":true}"#,
    )
    .unwrap();
    assert_eq!(
        slack::map_conversation(&p, None, &none).kind,
        ConversationKind::PrivateChannel
    );
    let g: slack_api::Conversation =
        serde_json::from_str(r#"{"id":"G1","name":"mpdm-a--b-1","is_mpim":true}"#).unwrap();
    let g = slack::map_conversation(&g, None, &none);
    assert_eq!(g.kind, ConversationKind::Group);
    assert!(g.is_member);
    let handles: Vec<&str> = g.participants.iter().map(|h| &**h).collect();
    assert_eq!(handles, ["a", "b"]);
    let c: slack_api::Conversation =
        serde_json::from_str(r#"{"id":"C1","name":"general","is_channel":true}"#).unwrap();
    let zero = slack_api::ChannelCount {
        id: "C1".into(),
        last_read: "0000000000.000000".into(),
        latest: "0000000000.000000".into(),
        ..Default::default()
    };
    let c = slack::map_conversation(&c, Some(&zero), &none);
    assert!(c.latest.is_none() && c.last_read.is_none());
    assert_eq!(c.kind, ConversationKind::Channel);
}

#[test]
fn map_user_prefers_display_name() {
    let u: slack_api::User = serde_json::from_str(
        r#"{"id":"U1","name":"alice","is_bot":true,"deleted":true,"profile":{"display_name":"","real_name":"Alice A"}}"#,
    )
    .unwrap();
    let m = slack::map_user(u);
    assert_eq!(&*m.display_name, "Alice A");
    assert_eq!(&*m.real_name, "Alice A");
    assert!(m.is_bot && m.deleted);
    assert_eq!(m.title, None);
    assert_eq!(m.tz, None);
}

#[test]
fn the_built_in_bot_maps_as_a_bot() {
    let u: slack_api::User = serde_json::from_str(
        r#"{"id":"USLACKBOT","name":"slackbot","is_bot":false,"profile":{"real_name":"Slackbot"}}"#,
    )
    .unwrap();
    assert!(slack::map_user(u).is_bot);
}

#[test]
fn map_user_keeps_title_and_time_zone_for_the_directory() {
    let u: slack_api::User = serde_json::from_str(
        r#"{"id":"U1","name":"alice","tz":"Asia/Tokyo","profile":{"display_name":"Alice","real_name":"Alice A","title":"Product designer"}}"#,
    )
    .unwrap();
    let m = slack::map_user(u);
    assert_eq!(m.title.as_deref(), Some("Product designer"));
    assert_eq!(m.tz.as_deref(), Some("Asia/Tokyo"));
    let blank: slack_api::User =
        serde_json::from_str(r#"{"id":"U2","name":"bob","tz":"","profile":{"title":""}}"#).unwrap();
    let m = slack::map_user(blank);
    assert_eq!((m.title, m.tz), (None, None));
}

#[test]
fn map_user_picks_the_row_size_avatar_on_the_right_route() {
    let u: slack_api::User = serde_json::from_str(
        r#"{"id":"U1","name":"a","profile":{"image_48":"https://avatars.slack-edge.com/a-48.png","image_72":"https://avatars.slack-edge.com/a-72.png","image_192":"https://avatars.slack-edge.com/a-192.png"}}"#,
    )
    .unwrap();
    assert_eq!(
        slack::map_user(u).avatar_for(36.0).cloned(),
        Some(domain::ImageRef::Provider(domain::RemoteUrl(
            "https://avatars.slack-edge.com/a-72.png".into()
        )))
    );
    // Only a smaller size exists: use it.
    let small: slack_api::User = serde_json::from_str(
        r#"{"id":"U2","name":"b","profile":{"image_48":"https://avatars.slack-edge.com/b-48.png"}}"#,
    )
    .unwrap();
    assert!(slack::map_user(small).avatar.is_some());
    // The big avatars use the 192 px picture.
    let big: slack_api::User = serde_json::from_str(
        r#"{"id":"U6","name":"f","profile":{"image_72":"https://avatars.slack-edge.com/f-72.png","image_192":"https://avatars.slack-edge.com/f-192.png"}}"#,
    )
    .unwrap();
    let m = slack::map_user(big);
    assert_eq!(
        m.avatar_for(72.0),
        Some(&domain::ImageRef::Provider(domain::RemoteUrl(
            "https://avatars.slack-edge.com/f-192.png".into()
        )))
    );
    // A non-Slack host (a gravatar) is fetched without credentials.
    let web: slack_api::User = serde_json::from_str(
        r#"{"id":"U3","name":"c","profile":{"image_72":"https://secure.gravatar.com/avatar/x.jpg"}}"#,
    )
    .unwrap();
    assert_eq!(
        slack::map_user(web).avatar_for(36.0).cloned(),
        Some(domain::ImageRef::Web(
            "https://secure.gravatar.com/avatar/x.jpg".into()
        ))
    );
    // No picture, or a non-https one: none.
    let none: slack_api::User = serde_json::from_str(
        r#"{"id":"U4","name":"d","profile":{"image_72":"http://avatars.slack-edge.com/d.png"}}"#,
    )
    .unwrap();
    assert_eq!(slack::map_user(none).avatar_for(36.0), None);
    let bare: slack_api::User = serde_json::from_str(r#"{"id":"U5","name":"e"}"#).unwrap();
    assert_eq!(slack::map_user(bare).avatar_for(36.0), None);
}

fn event(text: &str) -> slack_realtime::event::SlackEvent {
    slack_realtime::event::parse_frame(text).unwrap()
}

fn map_events(e: slack_realtime::event::SlackEvent) -> Vec<ProviderEvent> {
    slack::map_events_as(e, None)
}

fn map_event(e: slack_realtime::event::SlackEvent) -> Option<ProviderEvent> {
    map_events(e).into_iter().next()
}

#[test]
fn map_event_message_changed_replaces_the_whole_message() {
    let e = event(
        r#"{"type":"message","subtype":"message_changed","channel":"C1","ts":"1700000009.000000",
"message":{"type":"message","user":"U1","text":"new","ts":"1700000000.000100","edited":{"user":"U1","ts":"1700000009.000000"},
"blocks":[{"type":"rich_text","elements":[{"type":"rich_text_section","elements":[{"type":"text","text":"new"}]}]}],
"reactions":[{"name":"tada","count":1,"users":["U9"]}]}}"#,
    );
    match map_event(e) {
        Some(ProviderEvent::MessageUpdated(m)) => {
            assert_eq!(&*m.id.ts.0, "1700000000.000100");
            assert_eq!(&*m.id.conversation.0, "C1");
            assert_eq!(&*m.text, "new");
            assert_eq!(m.edited.as_ref().map(|t| &*t.0), Some("1700000009.000000"));
            assert_eq!(m.reactions.len(), 1);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn map_event_unfurl_update_is_not_marked_edited() {
    let e = event(
        r#"{"type":"message","subtype":"message_changed","channel":"C1","ts":"1700000009.000000",
"message":{"type":"message","user":"U1","text":"see https://example.test","ts":"1700000000.000100",
"attachments":[{"id":1,"title":"Example","from_url":"https://example.test"}]}}"#,
    );
    match map_event(e) {
        Some(ProviderEvent::MessageUpdated(m)) => {
            assert!(m.edited.is_none(), "an unfurl is not an edit");
            assert_eq!(&*m.text, "see https://example.test");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn map_event_basic_kinds() {
    assert!(matches!(
        map_event(event(
            r#"{"type":"message","channel":"C1","user":"U1","text":"x","ts":"1.000001"}"#
        )),
        Some(ProviderEvent::MessagePosted(_))
    ));
    assert!(matches!(
        map_event(event(
            r#"{"type":"message","subtype":"message_deleted","channel":"C1","deleted_ts":"1.000001","ts":"2.000001"}"#
        )),
        Some(ProviderEvent::MessageDeleted(_))
    ));
    assert!(matches!(
        map_event(event(
            r#"{"type":"channel_marked","channel":"C1","ts":"1.000001"}"#
        )),
        Some(ProviderEvent::ReadMarker { .. })
    ));
    assert!(map_event(event(r#"{"type":"hello"}"#)).is_none());
    assert!(map_event(event(r#"{"type":"brand_new_thing","x":1}"#)).is_none());
    let p = map_events(event(
        r#"{"type":"presence_change","presence":"away","users":["U1","U2"]}"#,
    ));
    assert_eq!(p.len(), 2);
    assert!(matches!(
        p[0],
        ProviderEvent::Presence { active: false, .. }
    ));
}

#[test]
fn map_event_team_rename() {
    let e = event(r#"{"type":"team_rename","name":"New Name"}"#);
    match map_event(e) {
        Some(ProviderEvent::WorkspaceRenamed { name }) => assert_eq!(&*name, "New Name"),
        other => panic!("{other:?}"),
    }
}

// ---- provider ------------------------------------------------------------

#[tokio::test]
async fn bootstrap_combines_boot_counts_users() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    mount(&server, "client.counts", &counts("1700000005.000000")).await;
    let ws_server = FakeServer::start(vec![]).await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    assert_eq!(p.workspace(), &key());
    let b = p.bootstrap().await.unwrap();
    assert_eq!(&*b.self_user.0, "U0000001");
    assert_eq!(&*b.workspace_name, "Example");
    assert_eq!(b.members.len(), 2);
    assert_eq!(b.conversations.len(), 4);
    let c1 = b.conversations.iter().find(|c| &*c.id.0 == "C1").unwrap();
    assert_eq!(c1.mentions, 2);
    assert!(c1.unread);
    // The boot payload omits `is_member`; its channels are the user's own.
    assert!(c1.is_member);
    assert_eq!(c1.latest.as_ref().map(|t| &*t.0), Some("1700000005.000000"));
    let d1 = b.conversations.iter().find(|c| &*c.id.0 == "D1").unwrap();
    assert_eq!(&*d1.name, "Bob B");
    assert!(p.capabilities().threads && p.capabilities().edit && p.capabilities().delete);
}

#[tokio::test]
async fn bootstrap_falls_back_to_users_conversations_when_boot_has_none() {
    let server = MockServer::start().await;
    mount(
        &server,
        "client.shouldReload",
        r#"{"ok":true,"should_reload":false,"recommended_version":"1794525555.123"}"#,
    )
    .await;
    mount(&server, "auth.test", r#"{"ok":true,"url":"https://example.slack.com/","team":"Example","user":"alice","team_id":"T0000001","user_id":"U0000001"}"#).await;
    mount(
        &server,
        "client.userBoot",
        r#"{"ok":true,"self":{"id":"U0000001","name":"alice"},
"team":{"id":"T0000001","name":"Example","domain":"example","url":"https://app.slack.com/"}}"#,
    )
    .await;
    mount(&server, "users.list", USERS).await;
    mount(&server, "client.counts", &counts("1700000005.000000")).await;
    mount(
        &server,
        "users.conversations",
        r#"{"ok":true,"channels":[{"id":"C1","name":"general","is_channel":true},
{"id":"D1","is_im":true,"user":"U0000002"}],"response_metadata":{"next_cursor":""}}"#,
    )
    .await;
    let ws_server = FakeServer::start(vec![]).await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let b = p.bootstrap().await.unwrap();
    let ids: Vec<_> = b.conversations.iter().map(|c| &*c.id.0).collect();
    assert_eq!(ids, vec!["C1", "D1"]);
    // users.conversations omits is_member, but every entry is a membership.
    assert!(b.conversations.iter().all(|c| c.is_member));
}

#[tokio::test]
async fn bootstrap_lists_channels_when_boot_carries_only_dms() {
    let server = MockServer::start().await;
    mount(
        &server,
        "client.shouldReload",
        r#"{"ok":true,"should_reload":false,"recommended_version":"1794525555.123"}"#,
    )
    .await;
    mount(&server, "auth.test", r#"{"ok":true,"url":"https://example.slack.com/","team":"Example","user":"alice","team_id":"T0000001","user_id":"U0000001"}"#).await;
    mount(
        &server,
        "client.userBoot",
        r#"{"ok":true,"self":{"id":"U0000001","name":"alice"},
"team":{"id":"T0000001","name":"Example","domain":"example","url":"https://app.slack.com/"},
"ims":[{"id":"D1","is_im":true,"user":"U0000002"}]}"#,
    )
    .await;
    mount(&server, "users.list", USERS).await;
    mount(&server, "client.counts", &counts("1700000005.000000")).await;
    mount(
        &server,
        "users.conversations",
        r#"{"ok":true,"channels":[{"id":"C1","name":"general","is_channel":true},
{"id":"D1","is_im":true,"user":"U0000002"}],"response_metadata":{"next_cursor":""}}"#,
    )
    .await;
    let ws_server = FakeServer::start(vec![]).await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let b = p.bootstrap().await.unwrap();
    let ids: Vec<_> = b.conversations.iter().map(|c| &*c.id.0).collect();
    assert_eq!(ids, vec!["D1", "C1"]);
    assert!(b.conversations.iter().all(|c| c.is_member));
}

#[tokio::test]
async fn reconnect_does_not_fetch_history_itself() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/client.counts"))
        .respond_with(json(&counts(T0)))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    mount(&server, "client.counts", &counts("1700000004.000000")).await;
    mount(
        &server,
        "conversations.history",
        r#"{"ok":true,"has_more":false,"messages":[
{"type":"message","user":"U0000002","text":"m4","ts":"1700000004.000000"},
{"type":"message","user":"U0000002","text":"m3","ts":"1700000003.000000"},
{"type":"message","user":"U0000002","text":"m2","ts":"1700000002.000000"}]}"#,
    )
    .await;
    let ws_server = FakeServer::start(vec![
        Script {
            steps: vec![
                hello(),
                msg_frame("C1", "1700000001.000000", "m1"),
                msg_frame("C1", "1700000002.000000", "m2"),
                Step::Sleep(Duration::from_millis(200)),
                Step::Drop,
            ],
            ..Script::default()
        },
        Script {
            steps: vec![hello(), msg_frame("C1", "1700000005.000000", "m5")],
            ..Script::default()
        },
    ])
    .await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let mut ev = p.events();
    p.bootstrap().await.unwrap();
    // Catch-up is planned by `domain::sync`, not the provider: it only reports
    // the reconnect and forwards live posts.
    let ts = posted(&mut ev, 3).await;
    assert_eq!(
        ts,
        [
            "1700000001.000000",
            "1700000002.000000",
            "1700000005.000000"
        ]
    );
    assert!(history_requests(&server).await.is_empty());
}

#[tokio::test]
async fn invalid_auth_marks_expired_and_stops() {
    let server = MockServer::start().await;
    mount(
        &server,
        "client.shouldReload",
        r#"{"ok":true,"should_reload":false,"recommended_version":"1794525555.123"}"#,
    )
    .await;
    mount(
        &server,
        "auth.test",
        r#"{"ok":false,"error":"invalid_auth"}"#,
    )
    .await;
    let ws_server = FakeServer::start(vec![]).await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let mut ev = p.events();
    assert!(matches!(next(&mut ev).await, ProviderEvent::SessionExpired));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(ws_server.handshakes().is_empty());
    let n = server.received_requests().await.unwrap().len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        n,
        "no more requests"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), ev.next())
            .await
            .is_err(),
        "expired exactly once"
    );
}

#[tokio::test]
async fn http_invalid_auth_maps_to_session_expired() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    mount(
        &server,
        "conversations.history",
        r#"{"ok":false,"error":"invalid_auth"}"#,
    )
    .await;
    let ws_server = FakeServer::start(vec![]).await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let mut ev = p.events();
    let c = ConversationId("C1".into());
    assert_eq!(
        p.history(&c, None, 20).await.unwrap_err(),
        domain::ProviderError::SessionExpired
    );
    loop {
        if matches!(next(&mut ev).await, ProviderEvent::SessionExpired) {
            break;
        }
    }
}

#[tokio::test]
async fn send_maps_to_post_message() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(Field("channel", "C1".into()))
        .and(Field("text", "hello".into()))
        .and(Field("client_msg_id", "cmid-1".into()))
        .respond_with(json(
            r#"{"ok":true,"channel":"C1","ts":"1700000007.000100","message":{"type":"message","user":"U0000001","text":"hello","ts":"1700000007.000100"}}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;
    let ws_server = FakeServer::start(vec![]).await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let m = p
        .send(
            &ConversationId("C1".into()),
            "hello",
            None,
            ClientMsgId("cmid-1".into()),
        )
        .await
        .unwrap();
    assert_eq!(&*m.id.ts.0, "1700000007.000100");
    assert_eq!(&*m.id.conversation.0, "C1");
    assert_eq!(m.client_msg_id, Some(ClientMsgId("cmid-1".into())));
    assert_eq!(&*m.text, "hello");
}

#[tokio::test]
async fn edit_delete_mark_and_foreign_fetch_refused() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    for (m, f) in [
        ("chat.update", "ts"),
        ("chat.delete", "ts"),
        ("conversations.mark", "ts"),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/{m}")))
            .and(Field(f, "1700000007.000100".into()))
            .respond_with(json(r#"{"ok":true}"#))
            .expect(1)
            .mount(&server)
            .await;
    }
    let ws_server = FakeServer::start(vec![]).await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let id = domain::MessageId {
        conversation: ConversationId("C1".into()),
        ts: domain::MessageTs("1700000007.000100".into()),
    };
    p.edit(&id, "new").await.unwrap();
    p.delete(&id).await.unwrap();
    p.mark_read(&id.conversation, &id.ts).await.unwrap();
    let e = p
        .fetch(
            &domain::RemoteUrl("https://files.example.test/x".into()),
            10,
        )
        .await
        .unwrap_err();
    assert!(matches!(e, domain::ProviderError::Localized(_)), "{e:?}");
    assert!(
        p.web_url(&id)
            .unwrap()
            .ends_with("/archives/C1/p1700000007000100")
    );
    assert!(
        p.conversation_web_url(&id.conversation)
            .unwrap()
            .ends_with("/archives/C1")
    );
}

// ---- factory -------------------------------------------------------------

use domain::ProviderFactory;
use slack_auth::{Account, AccountIndex, CredentialStore, MemoryStore};

async fn factory_for(
    server: &MockServer,
    dir: &std::path::Path,
    with_creds: bool,
    fake: &FakeServer,
) -> impl ProviderFactory {
    let path = dir.join("accounts.json");
    let mut idx = AccountIndex::load(&path).unwrap();
    idx.upsert(Account {
        workspace: key(),
        name: "Example".into(),
        url: Url::parse(&server.uri()).unwrap(),
        user_id: "U0000001".into(),
        enterprise_id: None,
        enterprise_name: None,
        icon_url: None,
    });
    idx.save().unwrap();
    let store = MemoryStore::default();
    if with_creds {
        store.save(&creds(server)).await.unwrap();
    }
    slack::SlackFactory::new(tokio::runtime::Handle::current(), path, Arc::new(store))
        .with_ws_options(ws(Some(fake)))
}

#[tokio::test]
async fn factory_lists_accounts_and_starts() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    let fake = FakeServer::start(vec![]).await;
    let dir = tempfile::tempdir().unwrap();
    let f = factory_for(&server, dir.path(), true, &fake).await;
    assert_eq!(f.id(), &ProviderId("slack".into()));
    assert_eq!(f.accounts(), vec![key()]);
    let p = f.start(&key()).await.unwrap();
    assert_eq!(p.workspace(), &key());
}

#[tokio::test]
async fn factory_start_without_credentials_or_with_expired_session_fails() {
    let server = MockServer::start().await;
    mount(
        &server,
        "auth.test",
        r#"{"ok":false,"error":"invalid_auth"}"#,
    )
    .await;
    let fake = FakeServer::start(vec![]).await;
    let dir = tempfile::tempdir().unwrap();
    let f = factory_for(&server, dir.path(), false, &fake).await;
    assert_eq!(
        f.start(&key()).await.err(),
        Some(domain::ProviderError::SessionExpired)
    );
    let dir2 = tempfile::tempdir().unwrap();
    let f = factory_for(&server, dir2.path(), true, &fake).await;
    assert_eq!(
        f.start(&key()).await.err(),
        Some(domain::ProviderError::SessionExpired)
    );
}

#[test]
fn ext_codec_roundtrips_message_ext() {
    let m: slack_api::Message = serde_json::from_str(
        r#"{"type":"message","subtype":"bot_message","bot_id":"B1","ts":"1.000001","text":"x","blocks":[{"type":"section"}]}"#,
    )
    .unwrap();
    let d = slack::map_message("C1", m);
    let codec = slack::SlackCodec;
    use domain::{ExtCodec, ExtKind};
    let bytes = codec.encode(ExtKind::Message, &d.ext).unwrap();
    let back = codec.decode(ExtKind::Message, &bytes);
    let e = back.get::<SlackMessageExt>().unwrap();
    assert_eq!(e.bot_id.as_deref(), Some("B1"));
    assert!(e.blocks.is_some());
}

// ---- threads, reactions, emoji ---------------------------------------------

fn spike(name: &str) -> String {
    let p = format!(
        "{}/../api/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(p).unwrap()
}

async fn started(server: &MockServer) -> SlackProvider {
    base_mocks(server).await;
    mount(server, "client.counts", &counts(T0)).await;
    SlackProvider::start_with(
        creds(server),
        cfg(server),
        ws(None),
        &tokio::runtime::Handle::current(),
    )
}

#[tokio::test]
async fn replies_maps_thread_and_reply_fields() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(
        &server,
        "conversations.replies",
        &spike("threads/replies_p1.json"),
    )
    .await;
    let page = p
        .replies(
            &ConversationId("D0000007".into()),
            &domain::MessageTs("1794500969.628009".into()),
            None,
            4,
        )
        .await
        .unwrap();
    assert!(page.has_more);
    let root = &page.messages[0];
    assert_eq!(root.reply_count, 9);
    assert_eq!(
        root.latest_reply.as_ref().unwrap().0.as_ref(),
        "1794508024.488979"
    );
    assert_eq!(root.reply_users.len(), 2);
    assert!(root.thread_root.is_none());
    assert_eq!(
        page.messages[1].thread_root.as_ref().unwrap().0.as_ref(),
        "1794500969.628009"
    );
}

#[tokio::test]
async fn reply_broadcast_posts_with_flag() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(Field("reply_broadcast", "true".into()))
        .and(Field("thread_ts", "1700000000.000100".into()))
        .respond_with(json(&spike("threads/post_reply_broadcast.json")))
        .expect(1)
        .mount(&server)
        .await;
    let m = p
        .reply(
            &ConversationId("C0000001".into()),
            &domain::MessageTs("1700000000.000100".into()),
            "also to channel",
            true,
            ClientMsgId("id-1".into()),
        )
        .await
        .unwrap();
    assert!(m.broadcast);
    assert_eq!(m.client_msg_id.unwrap().0.as_ref(), "id-1");
}

#[tokio::test]
async fn react_adds_and_removes_with_skin_tone() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/reactions.add"))
        .and(Field("name", "+1::skin-tone-3".into()))
        .and(Field("timestamp", "1700000000.000100".into()))
        .respond_with(json(&spike("reactions/add_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/reactions.remove"))
        .and(Field("name", "tada".into()))
        .respond_with(json(&spike("reactions/no_reaction.json")))
        .expect(1)
        .mount(&server)
        .await;
    let id = domain::MessageId {
        conversation: ConversationId("C0000001".into()),
        ts: domain::MessageTs("1700000000.000100".into()),
    };
    p.react(&id, &slack_markup::emoji_ref("+1::skin-tone-3"), true)
        .await
        .unwrap();
    p.react(&id, &slack_markup::emoji_ref("tada"), false)
        .await
        .unwrap();
}

#[tokio::test]
async fn mark_thread_read_calls_subscription_mark() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.mark"))
        .and(Field("thread_ts", "1700000000.000100".into()))
        .and(Field("ts", "1700000005.000000".into()))
        .respond_with(json(&spike("subscriptions/thread_mark_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    p.mark_thread_read(
        &ConversationId("C0000001".into()),
        &domain::MessageTs("1700000000.000100".into()),
        &domain::MessageTs("1700000005.000000".into()),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn custom_emoji_resolves_aliases() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(&server, "emoji.list", &spike("emoji/list.json")).await;
    let list = p.custom_emoji().await.unwrap();
    let by: std::collections::HashMap<_, _> = list
        .iter()
        .map(|e| (e.shortcode.to_string(), &e.image))
        .collect();
    let url = domain::ImageRef::Provider(domain::RemoteUrl(
        "https://emoji.example.test/party_example.png".into(),
    ));
    assert_eq!(by["party_example"], &url);
    assert_eq!(by["partying"], &url);
    // An alias of a standard emoji has no image to show.
    assert!(!by.contains_key("thumbsup_alias"));
}

#[test]
fn map_reaction_and_thread_events() {
    let ev = |t: &str| map_events(event(t));
    match ev(r#"{"type":"reaction_added","user":"U2","reaction":"thumbsup","item":{"type":"message","channel":"C1","ts":"1.000100"}}"#)
        .as_slice()
    {
        [ProviderEvent::ReactionChanged { id, user, added, .. }] => {
            assert_eq!((&*id.conversation.0, &*id.ts.0, &*user.0), ("C1", "1.000100", "U2"));
            assert!(*added);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        ev(r#"{"type":"reaction_removed","user":"U2","reaction":"tada","item":{"type":"message","channel":"C1","ts":"1.000100"}}"#)
            .as_slice(),
        [ProviderEvent::ReactionChanged { added: false, .. }]
    ));
    match ev(r#"{"type":"message","subtype":"message_replied","channel":"C1","message":{"type":"message","ts":"1.000100","thread_ts":"1.000100","reply_count":3,"latest_reply":"2.000100","reply_users":["U1","U2"]}}"#)
        .as_slice()
    {
        [ProviderEvent::ThreadUpdated { root, reply_count, latest_reply, reply_users }] => {
            assert_eq!(&*root.ts.0, "1.000100");
            assert_eq!(*reply_count, 3);
            assert_eq!(&*latest_reply.as_ref().unwrap().0, "2.000100");
            assert_eq!(reply_users.len(), 2);
        }
        other => panic!("{other:?}"),
    }
    match ev(r#"{"type":"thread_marked","subscription":{"channel":"C1","thread_ts":"1.000100","last_read":"3.000100","type":"thread"}}"#)
        .as_slice()
    {
        [ProviderEvent::ThreadReadMarker { root, ts }, ProviderEvent::ThreadInboxChanged] => {
            assert_eq!((&*root.ts.0, &*ts.0), ("1.000100", "3.000100"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        ev(r#"{"type":"file_deleted","file_id":"F1"}"#).as_slice(),
        [ProviderEvent::FileDeleted(f)] if &*f.0 == "F1"
    ));
    assert!(matches!(
        ev(r#"{"type":"emoji_changed","subtype":"add","name":"x"}"#).as_slice(),
        [ProviderEvent::CustomEmojiChanged]
    ));
}

#[tokio::test]
async fn start_does_not_wait_for_team_info() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/team.info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(2))
                .set_body_raw(
                    r#"{"ok":true,"team":{"id":"T0000001","name":"Fresh","icon":{"image_132":"https://example.com/new.png"}}}"#,
                    "application/json",
                ),
        )
        .mount(&server)
        .await;
    let fake = FakeServer::start(vec![]).await;
    let dir = tempfile::tempdir().unwrap();
    let f = factory_for(&server, dir.path(), true, &fake).await;
    let t0 = std::time::Instant::now();
    let _p = f.start(&key()).await.unwrap();
    assert!(
        t0.elapsed() < Duration::from_millis(1500),
        "start waited for team.info"
    );
    // The refresh lands afterwards.
    let path = dir.path().join("accounts.json");
    let mut icon = None;
    for _ in 0..80 {
        icon = AccountIndex::load(&path).unwrap().accounts()[0]
            .icon_url
            .clone();
        if icon.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(icon.as_deref(), Some("https://example.com/new.png"));
}

// ---- scheduled messages ------------------------------------------------------

#[tokio::test]
async fn scheduled_round_trip() {
    let server = MockServer::start().await;
    let fx = |n: &str| {
        std::fs::read_to_string(format!(
            "{}/../api/tests/fixtures/scheduled/{n}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    };
    let p = started(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduleMessage"))
        .and(Field("post_at", "1800000000".into()))
        .respond_with(json(&fx("schedule_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduledMessages.list"))
        .and(Field("cursor", "cur2".into()))
        .respond_with(json(&fx("list_p2.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduledMessages.list"))
        .respond_with(json(&fx("list_p1.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat.deleteScheduledMessage"))
        .and(Field("scheduled_message_id", "Q0000001".into()))
        .respond_with(json(&fx("delete_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = ConversationId("C0000001".into());
    let s = p
        .schedule(&c, None, "hello later", 1_800_000_000)
        .await
        .unwrap();
    assert_eq!(s.id.0.as_ref(), "Q0000001");
    let all = p.scheduled().await.unwrap();
    assert_eq!(all.len(), 3);
    assert!(all[1].thread.is_some());
    p.delete_scheduled(&s).await.unwrap();
}

fn sched_fx(n: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/scheduled/{n}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[tokio::test]
async fn scheduled_list_is_capped_and_drops_entries_without_id() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    let looping = sched_fx("list_nulls.json").replace(
        "\"response_metadata\":null",
        "\"response_metadata\":{\"next_cursor\":\"again\"}",
    );
    Mock::given(method("POST"))
        .and(path("/api/chat.scheduledMessages.list"))
        .respond_with(json(&looping))
        .expect(20)
        .mount(&server)
        .await;
    let all = p.scheduled().await.unwrap();
    assert_eq!(all.len(), 20);
    assert!(all.iter().all(|s| !s.id.0.is_empty()));
}

#[tokio::test]
async fn delete_of_gone_scheduled_item_is_not_allowed() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/chat.deleteScheduledMessage"))
        .respond_with(json(&sched_fx("delete_gone.json")))
        .mount(&server)
        .await;
    let item = domain::ScheduledMessage {
        id: domain::ScheduleId("Q0000001".into()),
        conversation: ConversationId("C0000001".into()),
        thread: None,
        text: "x".into(),
        post_at: 1_800_000_000,
    };
    let e = p.delete_scheduled(&item).await.unwrap_err();
    assert!(matches!(e, domain::ProviderError::NotAllowed(_)), "{e:?}");
}

#[test]
fn map_dm_open_state_and_unresolved_peer_name() {
    let none = |_: &str| None;
    let parse = |j: &str| -> slack_api::Conversation { serde_json::from_str(j).unwrap() };
    let closed = slack::map_conversation(
        &parse(r#"{"id":"D1","is_im":true,"user":"U0000009","is_open":false}"#),
        None,
        &none,
    );
    assert!(!closed.is_open);
    // The raw peer id is never the name.
    assert_eq!(&*closed.name, "");
    let open = slack::map_conversation(
        &parse(r#"{"id":"G1","name":"mpdm-a--b-1","is_mpim":true,"is_open":true}"#),
        None,
        &none,
    );
    assert!(open.is_open);
    // A field the payload lacks means open.
    let absent = slack::map_conversation(
        &parse(r#"{"id":"D2","is_im":true,"user":"U1"}"#),
        None,
        &none,
    );
    assert!(absent.is_open);
}

#[test]
fn group_dm_without_open_flag_is_open_only_when_recent_or_unread() {
    let now = 1_800_000_000;
    let day = 86_400;
    let g = |j: &str| -> slack_api::Conversation { serde_json::from_str(j).unwrap() };
    let count = |latest: i64, unread: bool| slack_api::ChannelCount {
        id: "G1".into(),
        latest: format!("{latest}.000100"),
        has_unreads: unread,
        ..Default::default()
    };
    let mpim = g(r#"{"id":"G1","name":"mpdm-a--b-1","is_mpim":true}"#);
    let open = |c: Option<&slack_api::ChannelCount>| slack::group_dm_fallback_open(&mpim, c, now);
    assert_eq!(open(Some(&count(now - 29 * day, false))), Some(true));
    assert_eq!(open(Some(&count(now - 31 * day, false))), Some(false));
    assert_eq!(open(Some(&count(now - 400 * day, true))), Some(true));
    // No counts entry means no recent state.
    assert_eq!(open(None), Some(false));
    // An explicit flag, and anything that is not a group DM, is left alone.
    let flagged = g(r#"{"id":"G1","is_mpim":true,"is_open":false}"#);
    assert_eq!(slack::group_dm_fallback_open(&flagged, None, now), None);
    let ch = g(r#"{"id":"C1","name":"x"}"#);
    assert_eq!(slack::group_dm_fallback_open(&ch, None, now), None);
}

#[test]
fn group_members_keep_their_ids_from_the_boot_listing() {
    let none = |_: &str| None;
    let g: slack_api::Conversation = serde_json::from_str(
        r#"{"id":"C9","name":"mpdm-a--b--c-1","is_mpim":true,"members":["U1","U2","U3"]}"#,
    )
    .unwrap();
    let g = slack::map_conversation(&g, None, &none);
    let ids: Vec<&str> = g.member_ids.iter().map(|u| &*u.0).collect();
    assert_eq!(ids, ["U1", "U2", "U3"]);
}

#[test]
fn live_message_reactions_know_the_signed_in_user() {
    let e = event(
        r#"{"type":"message","channel":"C1","user":"U2","text":"x","ts":"1700000000.000100",
"reactions":[{"name":"tada","count":1,"users":["U0000001"]}]}"#,
    );
    match &slack::map_events_as(e, Some("U0000001"))[..] {
        [ProviderEvent::MessagePosted(m)] => assert!(m.reactions[0].me),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn dropping_the_provider_stops_the_websocket() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    // The first connection drops after a pause; a provider that is still alive would
    // reconnect (a second handshake), one that was dropped must not.
    let ws_server = FakeServer::start(vec![Script {
        steps: vec![hello(), Step::Sleep(Duration::from_millis(300)), Step::Drop],
        ..Script::default()
    }])
    .await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let mut ev = p.events();
    loop {
        if matches!(next(&mut ev).await, ProviderEvent::Connected { .. }) {
            break;
        }
    }
    drop(ev);
    drop(p);
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(ws_server.handshakes().len(), 1, "no reconnect after drop");
}

fn follow_root() -> domain::MessageId {
    domain::MessageId {
        conversation: ConversationId("C0000001".into()),
        ts: domain::MessageTs("1700000000.000100".into()),
    }
}

#[tokio::test]
async fn thread_follow_capability_is_on() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    assert!(p.capabilities().thread_follow);
}

#[tokio::test]
async fn follow_thread_reads_subscription_then_adds_with_thread_ts() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.get"))
        .and(Field("thread_ts", "1700000000.000100".into()))
        .respond_with(json(&spike("subscriptions/thread_get_none.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.add"))
        .and(Field("channel", "C0000001".into()))
        .and(Field("thread_ts", "1700000000.000100".into()))
        .and(Field("last_read", "1700000000.000100".into()))
        .respond_with(json(&spike("subscriptions/thread_add_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    p.set_thread_followed(&follow_root(), true, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn unfollow_thread_removes_with_known_last_read() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(
        &server,
        "subscriptions.thread.get",
        &spike("subscriptions/thread_get_followed.json"),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.remove"))
        .and(Field("thread_ts", "1700000000.000100".into()))
        .and(Field("last_read", "1700000005.000000".into()))
        .respond_with(json(&spike("subscriptions/thread_remove_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    p.set_thread_followed(&follow_root(), false, None)
        .await
        .unwrap();
}

fn follow_mock(m: &str, last_read: &str, fixture: &str, times: u64) -> Mock {
    Mock::given(method("POST"))
        .and(path(format!("/api/subscriptions.thread.{m}")))
        .and(Field("thread_ts", "1700000000.000100".into()))
        .and(Field("last_read", last_read.into()))
        .respond_with(json(&spike(fixture)))
        .expect(times)
}

#[tokio::test]
async fn follow_uses_the_known_read_cursor_without_reading_the_subscription() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.get"))
        .respond_with(json(&spike("subscriptions/thread_get_none.json")))
        .expect(0)
        .mount(&server)
        .await;
    follow_mock(
        "add",
        "1700000009.000000",
        "subscriptions/thread_add_ok.json",
        1,
    )
    .mount(&server)
    .await;
    let read = domain::MessageTs("1700000009.000000".into());
    p.set_thread_followed(&follow_root(), true, Some(&read))
        .await
        .unwrap();
}

#[tokio::test]
async fn follow_fails_without_adding_when_the_subscription_read_fails() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(
        &server,
        "subscriptions.thread.get",
        &spike("subscriptions/thread_get_error.json"),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.add"))
        .respond_with(json(&spike("subscriptions/thread_add_ok.json")))
        .expect(0)
        .mount(&server)
        .await;
    assert!(
        p.set_thread_followed(&follow_root(), true, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn follow_reports_a_refused_add() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(
        &server,
        "subscriptions.thread.get",
        &spike("subscriptions/thread_get_none.json"),
    )
    .await;
    follow_mock(
        "add",
        "1700000000.000100",
        "subscriptions/thread_add_error.json",
        1,
    )
    .mount(&server)
    .await;
    assert!(
        p.set_thread_followed(&follow_root(), true, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unfollow_falls_back_to_the_thread_ts_when_the_subscription_read_fails() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(
        &server,
        "subscriptions.thread.get",
        &spike("subscriptions/thread_get_error.json"),
    )
    .await;
    follow_mock(
        "remove",
        "1700000000.000100",
        "subscriptions/thread_remove_ok.json",
        1,
    )
    .mount(&server)
    .await;
    p.set_thread_followed(&follow_root(), false, None)
        .await
        .unwrap();
}
