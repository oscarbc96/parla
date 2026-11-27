//! Search: query building, snippets and hit mapping (synthetic fixtures; the
//! endpoint shape is unverified, see docs/providers/slack/web-client/search.md).

use std::sync::Arc;
use std::time::Duration;

use domain::{ChatProvider, ConversationId, ConversationKind, Has, HitSource, SearchQuery, UserId};
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackProvider, WsOptions, snippet, to_query};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use slack_realtime::fake_server::FakeServer;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/search/{name}",
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

fn cid(s: &str) -> ConversationId {
    ConversationId(Arc::from(s))
}
fn uid(s: &str) -> UserId {
    UserId(Arc::from(s))
}

fn names(id: &ConversationId) -> Option<Arc<str>> {
    match &*id.0 {
        "C1" => Some(Arc::from("ops")),
        "C2" => Some(Arc::from("has space")),
        "C3" => Some(Arc::from("quo\"te")),
        _ => None,
    }
}
fn people(id: &UserId) -> Option<Arc<str>> {
    match &*id.0 {
        "U1" => Some(Arc::from("ana")),
        "U2" => Some(Arc::from("Ana B")),
        _ => None,
    }
}

#[test]
fn query_escapes_names() {
    let q = SearchQuery {
        text: Arc::from("deploy \"rolled back"),
        conversations: vec![cid("C1"), cid("C2"), cid("C3"), cid("C9")],
        authors: vec![uid("U1"), uid("U2"), uid("U9")],
        before: Some(jiff::civil::date(2026, 1, 2)),
        after: Some(jiff::civil::date(2025, 12, 31)),
        has: vec![Has::File, Has::Link, Has::Reaction, Has::Pin],
    };
    let s = to_query(&q, &names, &people);
    assert_eq!(
        s,
        "deploy rolled back in:#ops in:<#C2> in:<#C3> in:<#C9> from:@ana from:<@U2> from:<@U9> \
         before:2026-01-02 after:2025-12-31 has:file has:link has:reaction has:pin"
    );
    // Free text can never act as an operator: tokens with a colon are quoted.
    for (text, want) in [
        ("-in:ops", "\"-in:ops\""),
        ("is:starred x", "\"is:starred\" x"),
        ("to:me", "\"to:me\""),
        ("before:2026-13-01", "\"before:2026-13-01\""),
        ("in:@", "\"in:@\""),
        ("in:\"a b\"", "\"in:\" \"a b\""),
        ("\"keep in:x\" y", "\"keep in:x\" y"),
    ] {
        let q = SearchQuery {
            text: Arc::from(text),
            ..SearchQuery::default()
        };
        assert_eq!(to_query(&q, &names, &people), want, "{text}");
    }
    // A balanced phrase is kept; text may be empty.
    let q = SearchQuery {
        text: Arc::from("\"two words\" x"),
        ..SearchQuery::default()
    };
    assert_eq!(to_query(&q, &names, &people), "\"two words\" x");
    assert_eq!(
        to_query(&SearchQuery::default(), &names, &people),
        "",
        "nothing to search is an empty query"
    );
}

#[test]
fn snippet_has_no_markup() {
    assert_eq!(
        snippet("rolled \u{e000}back\u{e001} <@U1> in <#C1|ops> *bold* <https://example.com|link>"),
        "rolled back @U1 in #ops bold link"
    );
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
async fn provider_maps_hits_with_remote_source() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/search.modules.messages"))
        .and(Field("query", "back in:<#C0000001>"))
        .respond_with(json(fixture("messages_p1.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/search.modules.files"))
        .respond_with(json(fixture("files.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/search.modules.people"))
        .respond_with(json(fixture("people.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/search.modules.channels"))
        .respond_with(json(fixture("channels.json")))
        .mount(&server)
        .await;
    let (p, _ws) = provider(&server).await;
    assert!(p.capabilities().search);

    let q = SearchQuery {
        text: Arc::from("back"),
        conversations: vec![cid("C0000001")],
        ..SearchQuery::default()
    };
    let page = p.search_messages(&q, None, 20).await.unwrap();
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.next.as_deref(), Some("cur-2"));
    let h = &page.items[0];
    assert_eq!(h.source, HitSource::Remote);
    assert_eq!(&*h.snippet, "rolled back @U0000001");
    assert_eq!(&*h.message.text, "rolled back <@U0000001>");
    assert_eq!(&*h.message.plain, "rolled back @U0000001");
    assert_eq!(h.message.mentions.len(), 1);
    assert_eq!(&*h.message.id.conversation.0, "C0000001");
    assert_eq!(&*h.message.id.ts.0, "1794500969.628009");
    assert_eq!(&*page.items[2].message.id.conversation.0, "D0000001");

    let files = p.search_files(&q, None, 10).await.unwrap();
    assert_eq!(files.items.len(), 2);
    assert_eq!(&*files.items[0].file.id.0, "F0000001");
    assert_eq!(
        files.items[0].conversation.as_ref().map(|c| &*c.0),
        Some("C0000001")
    );
    assert_eq!(
        files.items[0].author.as_ref().map(|u| &*u.0),
        Some("U0000002")
    );
    assert_eq!(&*files.items[0].ts.as_ref().unwrap().0, "1794500969.628009");
    // No channel on the hit: falls back to where the file was shared.
    assert_eq!(
        files.items[1].conversation.as_ref().map(|c| &*c.0),
        Some("C0000002")
    );
    assert_eq!(&*files.items[1].ts.as_ref().unwrap().0, "1794300000.000300");

    let m = p.search_people("a", 10).await.unwrap();
    assert_eq!(&*m[0].display_name, "Ana");
    assert!(m[1].is_bot);
    let c = p.search_conversations("ops", 10).await.unwrap();
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].kind, ConversationKind::Channel);
    assert_eq!(c[1].kind, ConversationKind::PrivateChannel);
}
