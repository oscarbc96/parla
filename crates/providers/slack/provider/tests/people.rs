//! DND, profiles, status, presence and notification prefs through the provider. The
//! endpoints are unverified against a capture: fixtures are synthetic.

use std::sync::Arc;
use std::time::Duration;

use domain::{
    ChatProvider, ConversationId, ConversationPrefs, EmojiRef, NotifyLevel, PresenceSetting,
    ProviderError, ProviderEvent, Status, UserId,
};
use futures::StreamExt;
use futures::stream::BoxStream;
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackProvider, WsOptions};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use slack_realtime::event::parse_frame;
use slack_realtime::fake_server::{FakeServer, Script, Step};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, Respond, ResponseTemplate};

fn map_events(e: slack_realtime::event::SlackEvent) -> Vec<ProviderEvent> {
    slack::map_events_as(e, None)
}

fn fx(n: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/{n}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn rt_fx(n: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../realtime/tests/fixtures/events/{n}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .trim()
    .to_string()
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

fn cfg(server: &MockServer) -> ClientConfig {
    let mut c = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    c.timeout = Duration::from_secs(5);
    c.retry_base_delay = Duration::from_millis(10);
    c
}

fn json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json")
}

async fn mount(server: &MockServer, p: &str, body: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/api/{p}")))
        .respond_with(json(body))
        .mount(server)
        .await;
}

struct Field(&'static str, String);

impl Match for Field {
    fn matches(&self, r: &Request) -> bool {
        let b = String::from_utf8_lossy(&r.body);
        b.contains(&format!("name=\"{}\"\r\n\r\n{}\r\n", self.0, self.1))
    }
}

fn body_field(r: &Request, name: &str) -> Option<String> {
    let b = String::from_utf8_lossy(&r.body).to_string();
    let key = format!("name=\"{name}\"\r\n\r\n");
    let start = b.find(&key)? + key.len();
    let end = b[start..].find("\r\n")? + start;
    Some(b[start..end].to_string())
}

const BOOT: &str = r#"{"ok":true,"self":{"id":"U0000001","name":"alice"},
"team":{"id":"T0000001","name":"Example","domain":"example","url":"https://app.slack.com/"},
"channels":[{"id":"C1","name":"general","is_channel":true,"is_member":true}],"ims":[]}"#;
const USERS: &str = r#"{"ok":true,"members":[{"id":"U0000001","name":"alice","profile":{"display_name":"Alice"}}],"response_metadata":{"next_cursor":""}}"#;
const COUNTS: &str = r#"{"ok":true,"channels":[],"mpims":[],"ims":[]}"#;

async fn base(server: &MockServer) {
    mount(
        server,
        "client.shouldReload",
        r#"{"ok":true,"should_reload":false,"recommended_version":"1794525555.123"}"#,
    )
    .await;
    mount(server, "auth.test", r#"{"ok":true,"url":"https://example.slack.com/","team":"Example","user":"alice","team_id":"T0000001","user_id":"U0000001"}"#).await;
    mount(server, "client.userBoot", BOOT).await;
    mount(server, "users.list", USERS).await;
    mount(server, "client.counts", COUNTS).await;
}

async fn started(server: &MockServer) -> SlackProvider {
    base(server).await;
    SlackProvider::start_with(
        creds(server),
        cfg(server),
        WsOptions::default(),
        &tokio::runtime::Handle::current(),
    )
}

async fn next(s: &mut BoxStream<'static, ProviderEvent>) -> ProviderEvent {
    tokio::time::timeout(Duration::from_secs(10), s.next())
        .await
        .expect("event in time")
        .expect("stream open")
}

fn uid(s: &str) -> UserId {
    UserId(Arc::from(s))
}

async fn mount_profile(server: &MockServer) {
    mount(server, "users.profile.get", &fx("people/profile_get.json")).await;
    mount(server, "users.info", &fx("people/user_info.json")).await;
}

#[tokio::test]
async fn profile_maps_fields_and_tz() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount_profile(&server).await;
    let pr = p.profile(&uid("U0000002")).await.unwrap();
    assert_eq!(&*pr.display_name, "Alex");
    assert_eq!(pr.title.as_deref(), Some("Engineer"));
    assert_eq!(pr.pronouns.as_deref(), Some("they/them"));
    assert_eq!(pr.tz.as_deref(), Some("Europe/Madrid"));
    assert_eq!(pr.fields.len(), 2);
    let st = pr.status.unwrap();
    assert_eq!(&*st.text, "In a meeting");
    assert_eq!(st.expires, Some(1_800_000_000));
    assert_eq!(&*st.emoji.unwrap().shortcode, "spiral_calendar_pad");
    assert!(pr.image.is_some());
}

#[tokio::test]
async fn profile_of_unknown_user_is_not_allowed() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(
        &server,
        "users.profile.get",
        &fx("people/user_not_found.json"),
    )
    .await;
    mount(&server, "users.info", &fx("people/user_not_found.json")).await;
    let e = p.profile(&uid("U0000009")).await.unwrap_err();
    assert!(matches!(e, ProviderError::NotAllowed(_)), "{e:?}");
    let e = p.profile(&uid("")).await.unwrap_err();
    assert!(matches!(e, ProviderError::Other(_)), "{e:?}");
}

#[tokio::test]
async fn set_status_params_match_documented_form() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/users.profile.set"))
        .and(|r: &Request| {
            let v: serde_json::Value =
                serde_json::from_str(&body_field(r, "profile").unwrap_or_default())
                    .unwrap_or_default();
            v["status_text"] == "Lunch"
                && v["status_emoji"] == ":wave::skin-tone-3:"
                && v["status_expiration"] == 1_800_000_000i64
        })
        .respond_with(json(&fx("people/ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/users.profile.set"))
        .and(|r: &Request| {
            let v: serde_json::Value =
                serde_json::from_str(&body_field(r, "profile").unwrap_or_default())
                    .unwrap_or_default();
            v["status_text"] == "" && v["status_emoji"] == "" && v["status_expiration"] == 0
        })
        .respond_with(json(&fx("people/ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    p.set_status(Some(Status {
        emoji: Some(EmojiRef {
            shortcode: "wave".into(),
            unicode: None,
            skin_tone: Some(3),
        }),
        text: "Lunch".into(),
        expires: Some(1_800_000_000),
    }))
    .await
    .unwrap();
    p.set_status(None).await.unwrap();
}

#[tokio::test]
async fn bad_status_emoji_is_not_allowed() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(
        &server,
        "users.profile.set",
        &fx("people/status_bad_emoji.json"),
    )
    .await;
    let e = p
        .set_status(Some(Status {
            emoji: None,
            text: "x".into(),
            expires: None,
        }))
        .await
        .unwrap_err();
    assert!(matches!(e, ProviderError::NotAllowed(_)), "{e:?}");
}

#[tokio::test]
async fn presence_maps() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/users.setPresence"))
        .and(Field("presence", "away".into()))
        .respond_with(json(&fx("people/ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    p.set_presence(PresenceSetting::Away).await.unwrap();
}

#[tokio::test]
async fn dnd_and_snooze_and_end() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(&server, "dnd.info", &fx("dnd/info.json")).await;
    let d = p.dnd().await.unwrap();
    assert!(d.enabled);
    assert_eq!(d.next_start, Some(1_700_000_000));
    assert_eq!(d.snooze_until, Some(1_700_001_800));

    Mock::given(method("POST"))
        .and(path("/api/dnd.setSnooze"))
        .and(Field("num_minutes", "30".into()))
        .respond_with(json(&fx("dnd/snooze_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let d = p.snooze(30).await.unwrap();
    // The schedule comes from `dnd.info`, the snooze from the answer.
    assert!(d.enabled);
    assert_eq!(d.next_end, Some(1_700_030_000));
    assert_eq!(d.snooze_until, Some(1_700_001_800));

    // Ending a snooze that is already over succeeds and reports the real state.
    let server2 = MockServer::start().await;
    let p2 = started(&server2).await;
    mount(&server2, "dnd.endSnooze", &fx("dnd/end_not_active.json")).await;
    mount(&server2, "dnd.info", &fx("dnd/info_nulls.json")).await;
    let d = p2.end_snooze().await.unwrap();
    assert!(!d.enabled && d.snooze_until.is_none());
}

#[tokio::test]
async fn dnd_users_maps_and_refuses_too_many() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(&server, "dnd.teamInfo", &fx("dnd/team_info.json")).await;
    let m = p
        .dnd_users(&[uid("U0000002"), uid("U0000003"), uid("")])
        .await
        .unwrap();
    assert_eq!(m.len(), 2);
    assert!(m[&uid("U0000002")].enabled);
    assert!(!m[&uid("U0000003")].enabled);
    let many: Vec<UserId> = (0..501).map(|i| uid(&format!("U{i:07}"))).collect();
    let e = p.dnd_users(&many).await.unwrap_err();
    assert!(matches!(e, ProviderError::Other(_)), "{e:?}");
}

#[tokio::test]
async fn prefs_json_maps_levels_and_mutes() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(&server, "users.prefs.get", &fx("prefs/get.json")).await;
    let m = p.notify_prefs().await.unwrap();
    let c = |s: &str| ConversationId(Arc::from(s));
    assert_eq!(
        m[&c("C0000001")],
        ConversationPrefs {
            level: NotifyLevel::Mentions,
            muted: false
        }
    );
    assert_eq!(
        m[&c("C0000002")],
        ConversationPrefs {
            level: NotifyLevel::Nothing,
            muted: true
        }
    );
    assert_eq!(
        m[&c("C0000003")],
        ConversationPrefs {
            level: NotifyLevel::Default,
            muted: true
        }
    );
    assert_eq!(m.len(), 3);
}

#[tokio::test]
async fn set_notify_prefs_writes_level_and_mute() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(&server, "users.prefs.get", &fx("prefs/get.json")).await;
    Mock::given(method("POST"))
        .and(path("/api/users.prefs.set"))
        .and(|r: &Request| {
            body_field(r, "name").as_deref() == Some("all_notifications_prefs")
                && body_field(r, "value").is_some_and(|v| {
                    let v: serde_json::Value = serde_json::from_str(&v).unwrap();
                    v["channels"]["C0000001"]["desktop"] == "everything"
                })
        })
        .respond_with(json(&fx("prefs/set_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    p.set_notify_prefs(
        &ConversationId("C0000001".into()),
        ConversationPrefs {
            level: NotifyLevel::All,
            muted: false,
        },
    )
    .await
    .unwrap();
}

#[test]
fn dnd_events_parse_and_map() {
    let e = map_events(parse_frame(&rt_fx("dnd_updated")).unwrap());
    match &e[..] {
        [ProviderEvent::DndChanged { user, status }] => {
            assert_eq!(&*user.0, "U0000001");
            assert_eq!(status.snooze_until, Some(1_700_001_800));
            assert!(status.enabled);
        }
        other => panic!("{other:?}"),
    }
    let e = map_events(parse_frame(&rt_fx("dnd_updated_user")).unwrap());
    match &e[..] {
        [ProviderEvent::DndChanged { user, status }] => {
            assert_eq!(&*user.0, "U0000002");
            assert_eq!(status.snooze_until, None);
            assert_eq!(status.next_end, Some(1_700_030_000));
        }
        other => panic!("{other:?}"),
    }
    // No user and null status: nothing to attribute it to.
    assert!(map_events(parse_frame(&rt_fx("dnd_updated_nulls")).unwrap()).is_empty());
}

#[test]
fn user_change_maps_status_and_profile_changed() {
    let e = map_events(parse_frame(&rt_fx("user_change_status")).unwrap());
    match &e[..] {
        [
            ProviderEvent::MemberUpdated(m),
            ProviderEvent::ProfileChanged(u),
        ] => {
            assert_eq!(&*u.0, "U0000002");
            let s = m.status.as_ref().expect("status");
            assert_eq!(&*s.text, "Lunch");
            assert_eq!(s.expires, Some(1_800_000_000));
            assert_eq!(&*s.emoji.as_ref().unwrap().shortcode, "taco");
        }
        other => panic!("{other:?}"),
    }
    let cleared = r#"{"type":"user_change","user":{"id":"U0000002","name":"alex","profile":{"status_text":"","status_emoji":""}}}"#;
    match &map_events(parse_frame(cleared).unwrap())[0] {
        ProviderEvent::MemberUpdated(m) => assert!(m.status.is_none()),
        other => panic!("{other:?}"),
    }
    let no_id = r#"{"type":"user_change","user":{"id":"","profile":{}}}"#;
    assert!(map_events(parse_frame(no_id).unwrap()).is_empty());
}

#[tokio::test]
async fn pref_change_refetches_notify_prefs_only_for_known_names() {
    // Pure mapping: the frame alone never carries enough to build the prefs.
    for n in [
        "pref_change_notify",
        "pref_change_muted",
        "pref_change_other",
    ] {
        assert!(map_events(parse_frame(&rt_fx(n)).unwrap()).is_empty());
    }
    let server = MockServer::start().await;
    base(&server).await;
    mount(&server, "users.prefs.get", &fx("prefs/get.json")).await;
    let ws = FakeServer::start(vec![Script {
        steps: vec![
            Step::Send(r#"{"type":"hello"}"#.into()),
            Step::Send(rt_fx("pref_change_other")),
            Step::Send(rt_fx("pref_change_notify")),
            Step::Sleep(Duration::from_millis(500)),
        ],
        ..Script::default()
    }])
    .await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        WsOptions {
            base: Some(ws.url()),
            conn: None,
        },
        &tokio::runtime::Handle::current(),
    );
    let mut ev = p.events();
    p.bootstrap().await.unwrap();
    loop {
        if let ProviderEvent::NotifyPrefsChanged(m) = next(&mut ev).await {
            assert_eq!(m.len(), 3);
            break;
        }
    }
    let n = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/api/users.prefs.get")
        .count();
    assert_eq!(n, 1, "the unknown pref name did not refetch");
}

#[tokio::test]
async fn self_dnd_frame_without_user_is_attributed_to_self() {
    let server = MockServer::start().await;
    base(&server).await;
    let frame = r#"{"type":"dnd_updated","dnd_status":{"dnd_enabled":true,"next_dnd_start_ts":1700000000,"next_dnd_end_ts":1700030000}}"#;
    let ws = FakeServer::start(vec![Script {
        steps: vec![
            Step::Send(r#"{"type":"hello"}"#.into()),
            Step::Send(frame.into()),
            Step::Sleep(Duration::from_millis(500)),
        ],
        ..Script::default()
    }])
    .await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server),
        WsOptions {
            base: Some(ws.url()),
            conn: None,
        },
        &tokio::runtime::Handle::current(),
    );
    let mut ev = p.events();
    p.bootstrap().await.unwrap();
    loop {
        if let ProviderEvent::DndChanged { user, status } = next(&mut ev).await {
            assert_eq!(&*user.0, "U0000001");
            assert!(status.enabled);
            break;
        }
    }
}

#[tokio::test]
async fn capabilities_set_dnd_status_notify_prefs() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    let c = p.capabilities();
    assert!(c.dnd && c.status);
    assert!(
        !c.notify_prefs,
        "off until a capture confirms the prefs shape"
    );
}

/// A users.prefs.get that answers with the stored value after a delay and a users.prefs.set
/// that stores what it is given: a lost update shows up as a missing conversation.
struct PrefsStore(Arc<std::sync::Mutex<(String, String)>>);

struct PrefsGet(Arc<std::sync::Mutex<(String, String)>>);

impl Respond for PrefsGet {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let (all, muted) = self.0.lock().unwrap().clone();
        let body = serde_json::json!({"ok": true, "prefs": {
            "all_notifications_prefs": all, "muted_channels": muted}});
        json(&body.to_string()).set_delay(Duration::from_millis(150))
    }
}

impl Respond for PrefsStore {
    fn respond(&self, r: &Request) -> ResponseTemplate {
        let (Some(n), Some(v)) = (body_field(r, "name"), body_field(r, "value")) else {
            return json(r#"{"ok":false,"error":"bad"}"#);
        };
        let mut g = self.0.lock().unwrap();
        if n == "all_notifications_prefs" {
            g.0 = v;
        } else {
            g.1 = v;
        }
        json(r#"{"ok":true}"#)
    }
}

#[tokio::test]
async fn concurrent_prefs_writes_for_different_conversations_both_survive() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    let state = Arc::new(std::sync::Mutex::new((
        r#"{"channels":{}}"#.to_string(),
        String::new(),
    )));
    Mock::given(method("POST"))
        .and(path("/api/users.prefs.get"))
        .respond_with(PrefsGet(state.clone()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/users.prefs.set"))
        .respond_with(PrefsStore(state.clone()))
        .mount(&server)
        .await;
    let prefs = ConversationPrefs {
        level: NotifyLevel::Nothing,
        muted: false,
    };
    let (a, b) = (
        ConversationId("C0000001".into()),
        ConversationId("C0000002".into()),
    );
    let (ra, rb) = futures::join!(p.set_notify_prefs(&a, prefs), p.set_notify_prefs(&b, prefs));
    ra.unwrap();
    rb.unwrap();
    let all: serde_json::Value = serde_json::from_str(&state.lock().unwrap().0).unwrap();
    assert_eq!(all["channels"]["C0000001"]["desktop"], "nothing");
    assert_eq!(all["channels"]["C0000002"]["desktop"], "nothing");
}

#[tokio::test]
async fn snooze_fails_when_the_follow_up_read_fails() {
    let server = MockServer::start().await;
    let p = started(&server).await;
    mount(
        &server,
        "dnd.setSnooze",
        r#"{"ok":true,"snooze_enabled":true,"snooze_endtime":1700001800}"#,
    )
    .await;
    mount(
        &server,
        "dnd.info",
        r#"{"ok":false,"error":"internal_error"}"#,
    )
    .await;
    assert!(p.snooze(30).await.is_err());
}
