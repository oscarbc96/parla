//! Files: mapping, refresh on file events, host-checked download, upload.

use std::sync::Arc;
use std::time::Duration;

use domain::{
    ChatProvider, ClientMsgId, ConversationId, FileKind, ProviderError, ProviderEvent, RemoteUrl,
    Upload,
};
use futures::StreamExt;
use futures::stream::BoxStream;
use settings::{ProviderId, WorkspaceId, WorkspaceKey};
use slack::{SlackProvider, WsOptions, map_file};
use slack_api::{ClientConfig, SessionCookie, XoxcToken};
use slack_auth::WorkspaceCredentials;
use slack_realtime::fake_server::{FakeServer, Script, Step};
use url::Url;
use wiremock::matchers::{header_exists, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/../api/tests/fixtures/files/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn api_file(name: &str) -> slack_api::File {
    let v: serde_json::Value = serde_json::from_str(&fixture(name)).unwrap();
    serde_json::from_value(v["file"].clone()).unwrap()
}

fn json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json")
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

fn cfg(server: &MockServer, hosts: &[&MockServer]) -> ClientConfig {
    let mut c = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    c.timeout = Duration::from_secs(5);
    c.retry_base_delay = Duration::from_millis(10);
    for h in hosts {
        c.file_hosts = c
            .file_hosts
            .clone()
            .with_test_authority(&h.address().to_string());
    }
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

async fn base_mocks(server: &MockServer) {
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
            r#"{"ok":true,"self":{"id":"U0000001","name":"alice"},"team":{"id":"T0000001","name":"Example","domain":"example","url":"https://app.slack.com/"},"channels":[{"id":"C1","name":"general","is_channel":true,"is_member":true}],"ims":[]}"#,
        ),
        (
            "users.list",
            r#"{"ok":true,"members":[],"response_metadata":{"next_cursor":""}}"#,
        ),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/api/{p}")))
            .respond_with(json(body))
            .mount(server)
            .await;
    }
}

/// Next event within 10 s.
async fn next(s: &mut BoxStream<'static, ProviderEvent>) -> ProviderEvent {
    tokio::time::timeout(Duration::from_secs(10), s.next())
        .await
        .expect("event in time")
        .expect("stream open")
}

#[test]
fn map_file_kinds_and_renditions() {
    let img = map_file(&api_file("info_image.json"));
    assert_eq!(img.kind, FileKind::Image);
    assert_eq!(&*img.id.0, "F0000001");
    assert_eq!(img.thumbs.len(), 4);
    assert_eq!(img.best_thumb(400).unwrap().width, 480);
    assert!(img.download.as_ref().unwrap().0.contains("/download/"));
    assert!(img.video.is_none() && !img.tombstone);

    let pdf = map_file(&api_file("info_pdf.json"));
    assert_eq!(pdf.kind, FileKind::Pdf);
    assert_eq!(pdf.thumbs.len(), 1);

    let snip = map_file(&api_file("info_snippet.json"));
    assert_eq!(snip.kind, FileKind::Snippet);
    assert_eq!(snip.preview.as_deref(), Some("fn main() {}\n"));
    assert_eq!(
        snip.lines,
        Some(9),
        "total lines of the file, not of the preview"
    );
    // Without `lines`, the preview's lines plus `lines_more` are the whole file.
    let mut more = api_file("info_snippet.json");
    more.lines = None;
    more.lines_more = Some(8);
    assert_eq!(map_file(&more).lines, Some(9));

    let clip = map_file(&api_file("info_clip.json"));
    assert_eq!(clip.kind, FileKind::Video);
    let v = clip.video.unwrap();
    assert!(v.mp4_low.is_some() && v.mp4.is_some());
    assert_eq!(v.duration_ms, Some(3200));
    assert!(v.vtt.as_ref().is_some_and(|u| u.0.ends_with("clip.vtt")));
    assert_eq!(v.poster.unwrap().width, 1280);

    // Empty mimetype and filetype: the subtype decides.
    let audio = map_file(&api_file("info_audio_clip.json"));
    assert_eq!(audio.kind, FileKind::Audio);
    assert!(!audio.mime.is_empty());

    assert!(map_file(&api_file("info_tombstone.json")).tombstone);
}

#[tokio::test]
async fn messages_carry_files_and_file_change_refreshes() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/client.counts"))
        .respond_with(json(r#"{"ok":true,"channels":[],"mpims":[],"ims":[]}"#))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/files.info"))
        .respond_with(json(&fixture("info_clip.json")))
        .expect(1)
        .mount(&server)
        .await;
    let msg = r#"{"type":"message","channel":"C1","user":"U0000002","text":"","ts":"1700000001.000100","files":[{"id":"F0000004","name":"Video clip.mp4","subtype":"slack_video","mimetype":"video/mp4","mode":"hosted"}]}"#;
    let ws_server = FakeServer::start(vec![Script {
        steps: vec![
            Step::Send(r#"{"type":"hello"}"#.into()),
            Step::Send(msg.into()),
            Step::Send(r#"{"type":"file_change","file_id":"F0000004"}"#.into()),
        ],
        ..Script::default()
    }])
    .await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server, &[]),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let mut ev = p.events();
    let mut posted = None;
    for _ in 0..30 {
        if let ProviderEvent::MessagePosted(m) = next(&mut ev).await {
            posted = Some(m);
            break;
        }
    }
    let posted = posted.expect("message posted");
    assert_eq!(posted.files.len(), 1);
    assert!(posted.files[0].video.is_none());
    let mut updated = None;
    for _ in 0..30 {
        if let ProviderEvent::MessageUpdated(m) = next(&mut ev).await {
            updated = Some(m);
            break;
        }
    }
    let updated = updated.expect("message updated");
    assert_eq!(updated.id, posted.id);
    let v = updated.files[0]
        .video
        .as_ref()
        .expect("renditions after refresh");
    assert!(v.mp4_low.is_some());
}

#[tokio::test]
async fn download_only_from_file_hosts_with_cookie() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    let files = MockServer::start().await;
    let evil = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .and(header_exists("cookie"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(&b"payload"[..]))
        .mount(&files)
        .await;
    let ws_server = FakeServer::start(vec![]).await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server, &[&files]),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out");
    let n = p
        .download(&RemoteUrl(format!("{}/f", files.uri()).into()), &dest)
        .await
        .unwrap();
    assert_eq!(n, 7);
    let got = p
        .fetch(&RemoteUrl(format!("{}/f", files.uri()).into()), 100)
        .await
        .unwrap();
    assert_eq!(got, b"payload");
    let e = p
        .fetch(&RemoteUrl(format!("{}/f", files.uri()).into()), 3)
        .await
        .unwrap_err();
    assert_eq!(e, ProviderError::TooLarge, "over the limit is too large");

    // A host that is not allowed is refused and never contacted.
    let bad = RemoteUrl(format!("{}/f", evil.uri()).into());
    let e = p.download(&bad, &dir.path().join("bad")).await.unwrap_err();
    assert!(matches!(e, ProviderError::Localized(_)), "{e:?}");
    let e = p.fetch(&bad, 10).await.unwrap_err();
    assert!(matches!(e, ProviderError::Localized(_)), "{e:?}");
    assert!(evil.received_requests().await.unwrap().is_empty());
    assert!(!dir.path().join("bad").exists());
}

#[tokio::test]
async fn upload_shares_files_and_returns_the_message() {
    let server = MockServer::start().await;
    base_mocks(&server).await;
    let files = MockServer::start().await;
    let upload_url = format!("{}/upload/v1/EXAMPLE", files.uri());
    Mock::given(method("POST"))
        .and(path("/api/files.getUploadURLExternal"))
        .respond_with(json(&fixture("get_upload_url.json").replace(
            "https://files.example.test/upload/v1/EXAMPLEUPLOADTOKEN",
            &upload_url,
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/EXAMPLE"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&files)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/files.completeUploadExternal"))
        .and(|r: &Request| String::from_utf8_lossy(&r.body).contains("C1"))
        .respond_with(json(&fixture("complete_upload.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/files.info"))
        .respond_with(json(&fixture("info_image.json").replace("C0000001", "C1")))
        .mount(&server)
        .await;
    let ws_server = FakeServer::start(vec![]).await;
    let p = SlackProvider::start_with(
        creds(&server),
        cfg(&server, &[&files]),
        ws(Some(&ws_server)),
        &tokio::runtime::Handle::current(),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photo.png");
    std::fs::write(&path, b"hello").unwrap();
    let m = p
        .upload(
            &ConversationId("C1".into()),
            None,
            vec![Upload {
                path,
                name: "photo.png".into(),
                mime: "image/png".into(),
                size: 5,
                filetype: None,
            }],
            Some("look"),
            ClientMsgId("cm1".into()),
        )
        .await
        .unwrap();
    assert_eq!(&*m.id.ts.0, "1794500969.628009");
    assert_eq!(&*m.id.conversation.0, "C1");
    assert_eq!(m.files.len(), 1);
    assert_eq!(m.files[0].kind, FileKind::Image);
    assert_eq!(&*m.text, "look");
    assert_eq!(m.client_msg_id.as_ref().unwrap().0.as_ref(), "cm1");
}
