//! Pins, bookmarks, saved items, the Threads inbox and single-message fetch through the
//! provider (synthetic fixtures; endpoints unverified, see
//! docs/providers/slack/web-client/messages.md).

use std::sync::Arc;
use std::time::Duration;

use domain::{
    BookmarkId, ChatProvider, ConversationId, MessageId, MessageTs, ProviderError, ProviderEvent,
};
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackProvider, WsOptions};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use slack_realtime::fake_server::FakeServer;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

fn fixture(dir: &str, name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/{dir}/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn mid(c: &str, ts: &str) -> MessageId {
    MessageId {
        conversation: ConversationId(Arc::from(c)),
        ts: MessageTs(Arc::from(ts)),
    }
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
async fn pins_list_and_toggle_map_to_domain() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/pins.list"))
        .respond_with(json(fixture("pins", "list.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/pins.add"))
        .and(Field("timestamp", "1800000000.000100"))
        .respond_with(json(fixture("pins", "already_pinned.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/pins.remove"))
        .respond_with(json(fixture("pins", "no_pin.json")))
        .expect(1)
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    assert!(
        !p.capabilities().pins
            && !p.capabilities().bookmarks
            && !p.capabilities().saved
            && !p.capabilities().thread_inbox,
        "off until a capture confirms the routes"
    );
    let pins = p
        .pins(&ConversationId(Arc::from("C0000001")))
        .await
        .unwrap();
    assert_eq!(pins.len(), 2);
    assert_eq!(pins[0].message, mid("C0000001", "1800000000.000100"));
    assert_eq!(pins[0].by.as_ref().map(|u| &*u.0), Some("U0000002"));
    assert_eq!(pins[0].at, 1_800_000_100);
    // Already pinned and already unpinned are the state the caller asked for.
    p.set_pinned(&mid("C0000001", "1800000000.000100"), true)
        .await
        .unwrap();
    p.set_pinned(&mid("C0000001", "1800000000.000100"), false)
        .await
        .unwrap();
}

#[tokio::test]
async fn bookmarks_map_to_domain() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/bookmarks.list"))
        .respond_with(json(fixture("bookmarks", "list.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/bookmarks.add"))
        .respond_with(json(fixture("bookmarks", "add.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/bookmarks.remove"))
        .and(Field("bookmark_id", "Bk0000003"))
        .respond_with(json(fixture("bookmarks", "ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    let c = ConversationId(Arc::from("C0000001"));
    let list = p.bookmarks(&c).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(&*list[0].id.0, "Bk0000001");
    assert_eq!(&*list[0].title, "Runbook");
    assert_eq!(list[0].conversation, c);
    assert_eq!(list[0].emoji.as_ref().map(|e| &*e.shortcode), Some("book"));
    assert!(list[1].emoji.is_none());
    let b = p
        .add_bookmark(&c, "Design doc", "https://example.com/design")
        .await
        .unwrap();
    assert_eq!(&*b.id.0, "Bk0000003");
    p.remove_bookmark(&c, &BookmarkId(Arc::from("Bk0000003")))
        .await
        .unwrap();
}

#[tokio::test]
async fn saved_pages_and_toggles() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/saved.list"))
        .and(Field("cursor", "c2"))
        .respond_with(json(fixture("saved", "list_p2.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/saved.list"))
        .respond_with(json(fixture("saved", "list_p1.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/saved.add"))
        .and(Field("item_id", "C0000001"))
        .respond_with(json(fixture("saved", "ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/saved.delete"))
        .respond_with(json(fixture("saved", "ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    let p1 = p.saved(None).await.unwrap();
    assert_eq!(p1.next.as_deref(), Some("c2"));
    assert_eq!(p1.items.len(), 2);
    assert_eq!(p1.items[0].message, mid("C0000001", "1800000000.000100"));
    assert_eq!(p1.items[0].saved_at, 1_800_000_200);
    assert_eq!(p1.items[0].due, None);
    assert!(!p1.items[0].done);
    assert!(p1.items[1].done);
    assert_eq!(p1.items[1].due, Some(1_800_009_999));
    let p2 = p.saved(Some(Arc::from("c2"))).await.unwrap();
    assert_eq!(p2.next, None);
    p.set_saved(&mid("C0000001", "1800000000.000100"), true)
        .await
        .unwrap();
    p.set_saved(&mid("C0000001", "1800000000.000100"), false)
        .await
        .unwrap();
}

#[tokio::test]
async fn inbox_maps_unread_and_mentions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/subscriptions.thread.getView"))
        .respond_with(json(fixture("inbox", "list.json")))
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    let page = p.thread_inbox(None).await.unwrap();
    assert_eq!(page.next.as_deref(), Some("t2"));
    assert_eq!(page.items.len(), 2);
    let t = &page.items[0];
    assert_eq!(t.root, mid("C0000001", "1800000000.000100"));
    assert_eq!(t.unread_replies, 2);
    assert_eq!(
        t.mentions, 1,
        "only the reply that mentions the signed-in user"
    );
    assert_eq!(&*t.latest.0, "1800000600.000100");
    assert_eq!(page.items[1].unread_replies, 0);
    assert_eq!(page.items[1].mentions, 0);
}

#[tokio::test]
async fn message_get_root_reply_and_gone() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .and(Field("latest", "1800000000.000100"))
        .respond_with(json(fixture("messages", "root.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .and(Field("latest", "1800000050.000100"))
        .respond_with(json(fixture("messages", "older_instead.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.replies"))
        .and(Field("ts", "1800000050.000100"))
        .respond_with(json(fixture("messages", "reply.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.history"))
        .and(Field("latest", "1700000000.000100"))
        .respond_with(json(fixture("messages", "empty.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/conversations.replies"))
        .and(Field("ts", "1700000000.000100"))
        .respond_with(json(fixture("messages", "not_found.json")))
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    let root = p
        .message(&mid("C0000001", "1800000000.000100"))
        .await
        .unwrap();
    assert_eq!(&*root.text, "an old message");
    assert!(root.thread_root.is_none());
    let reply = p
        .message(&mid("C0000001", "1800000050.000100"))
        .await
        .unwrap();
    assert_eq!(&*reply.text, "a reply");
    assert_eq!(
        reply.thread_root.as_ref().map(|t| &*t.0),
        Some("1800000000.000100")
    );
    match p.message(&mid("C0000001", "1700000000.000100")).await {
        Err(ProviderError::NotAllowed(e)) => assert_eq!(e, "message_not_found"),
        other => panic!("{other:?}"),
    }
}

fn frame(name: &str) -> slack_realtime::event::SlackEvent {
    let text = std::fs::read_to_string(format!(
        "{}/../realtime/tests/fixtures/events/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    slack_realtime::event::parse_frame(&text).unwrap()
}

#[test]
fn pin_and_saved_events_map() {
    use slack::map_events_as;
    let pin = |name: &str, want: bool| match map_events_as(frame(name), None).as_slice() {
        [ProviderEvent::PinChanged { id, pinned }] => {
            assert_eq!(*id, mid("C0000001", "1800000000.000100"));
            assert_eq!(*pinned, want);
        }
        other => panic!("{name}: {other:?}"),
    };
    pin("pin_added", true);
    pin("pin_removed", false);
    match map_events_as(frame("bookmark_changed"), None).as_slice() {
        [ProviderEvent::BookmarksChanged(c)] => assert_eq!(&*c.0, "C0000001"),
        other => panic!("{other:?}"),
    }
    for n in ["saved_added", "saved_removed"] {
        assert!(
            matches!(
                map_events_as(frame(n), None).as_slice(),
                [ProviderEvent::SavedChanged]
            ),
            "{n}"
        );
    }
    assert!(matches!(
        map_events_as(slack_realtime::event::SlackEvent::ThreadInboxChanged, None).as_slice(),
        [ProviderEvent::ThreadInboxChanged]
    ));
    // A subscription change follows (or unfollows) the thread and stales the Threads view.
    let subscribed = map_events_as(
        slack_realtime::event::SlackEvent::ThreadSubscribed {
            channel: "C0000001".into(),
            thread_ts: "1800000000.000100".into(),
            active: false,
        },
        None,
    );
    assert!(matches!(
        subscribed.as_slice(),
        [
            ProviderEvent::ThreadFollowChanged { root, followed: false },
            ProviderEvent::ThreadInboxChanged
        ] if &*root.ts.0 == "1800000000.000100" && &*root.conversation.0 == "C0000001"
    ));
    // A thread read mark moves the cursor and also stales the Threads view.
    let marked = map_events_as(
        slack_realtime::event::SlackEvent::ThreadMarked {
            channel: "C0000001".into(),
            thread_ts: "1800000000.000100".into(),
            ts: "1800000100.000100".into(),
        },
        None,
    );
    assert!(matches!(
        marked.as_slice(),
        [
            ProviderEvent::ThreadReadMarker { .. },
            ProviderEvent::ThreadInboxChanged
        ]
    ));
}

#[test]
fn map_message_sets_pinned_and_saved() {
    let m: slack_api::Message = serde_json::from_str(
        r#"{"type":"message","user":"U1","text":"x","ts":"1800000000.000100","pinned_to":["C0000001"],"is_starred":true}"#,
    )
    .unwrap();
    let d = slack::map_message("C0000001", m);
    assert!(d.pinned);
    assert!(d.saved);
    // Pinned in another conversation only (a shared message) is not a pin here.
    let m: slack_api::Message = serde_json::from_str(
        r#"{"type":"message","user":"U1","text":"x","ts":"1800000000.000100","pinned_to":["C0000009"]}"#,
    )
    .unwrap();
    let d = slack::map_message("C0000001", m);
    assert!(!d.pinned);
    assert!(!d.saved);
}
