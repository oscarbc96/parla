use std::time::Duration;

use slack_api::{
    ApiError, ClientConfig, FileHosts, SessionCookie, WebClient, XoxcToken, is_file_host,
};
use url::Url;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/files/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn auth(server: &MockServer) -> String {
    server.address().to_string()
}

/// A client whose file-host list admits the given mock servers.
fn client_for(api: &MockServer, hosts: &[&MockServer]) -> WebClient {
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&api.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    cfg.retry_base_delay = Duration::from_millis(10);
    let mut fh = FileHosts::default();
    for h in hosts {
        fh = fh.with_test_authority(&auth(h));
    }
    cfg.file_hosts = fh;
    WebClient::new(
        cfg,
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap()
}

fn json(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

fn has(req: &Request, name: &str, value: &str) -> bool {
    String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\"\r\n\r\n{value}"))
}

async fn info_of(name: &str) -> slack_api::File {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/files.info"))
        .and(|r: &Request| has(r, "file", "F0000001"))
        .respond_with(json(fixture(name)))
        .mount(&server)
        .await;
    client_for(&server, &[])
        .files_info("F0000001")
        .await
        .unwrap()
}

#[tokio::test]
async fn info_collects_thumbs_and_renditions() {
    let img = info_of("info_image.json").await;
    assert_eq!(img.id, "F0000001");
    assert_eq!(img.mimetype.as_deref(), Some("image/png"));
    let widths: Vec<u32> = img.thumbs.iter().map(|t| t.width).collect();
    assert!(widths.contains(&360) && widths.contains(&480) && widths.contains(&64));
    let t = img.thumbs.iter().find(|t| t.width == 480).unwrap();
    assert_eq!((t.height, t.url.contains("photo_480")), (320, true));
    assert_eq!((img.original_w, img.original_h), (Some(1200), Some(800)));
    assert_eq!(
        img.share_ts("C0000001").as_deref(),
        Some("1794500969.628009")
    );

    let pdf = info_of("info_pdf.json").await;
    assert_eq!(
        pdf.thumb_pdf.as_ref().map(|t| (t.width, t.height)),
        Some((909, 1286))
    );

    let snip = info_of("info_snippet.json").await;
    assert_eq!(snip.mode.as_deref(), Some("snippet"));
    assert_eq!(snip.preview.as_deref(), Some("fn main() {}\n"));

    let clip = info_of("info_clip.json").await;
    assert_eq!(clip.subtype.as_deref(), Some("slack_video"));
    assert!(clip.mp4_low.is_some() && clip.mp4.is_some() && clip.hls.is_some());
    assert_eq!(clip.duration_ms, Some(3200));
    assert!(clip.vtt.as_deref().is_some_and(|u| u.ends_with("clip.vtt")));
    assert_eq!(
        clip.thumb_video.as_ref().map(|t| (t.width, t.height)),
        Some((1280, 720))
    );
}

#[tokio::test]
async fn tombstone_detected() {
    let f = info_of("info_tombstone.json").await;
    assert!(f.is_tombstone());
    assert!(!info_of("info_image.json").await.is_tombstone());
}

#[tokio::test]
async fn upload_runs_three_steps_in_order() {
    let api = MockServer::start().await;
    let files = MockServer::start().await;
    let upload_url = format!("{}/upload/v1/EXAMPLE", files.uri());
    let body = fixture("get_upload_url.json").replace(
        "https://files.example.test/upload/v1/EXAMPLEUPLOADTOKEN",
        &upload_url,
    );
    Mock::given(method("POST"))
        .and(path("/api/files.getUploadURLExternal"))
        .and(|r: &Request| has(r, "filename", "photo.png") && has(r, "length", "5"))
        .respond_with(json(body))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/EXAMPLE"))
        .and(|r: &Request| r.body == b"hello")
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&files)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/files.completeUploadExternal"))
        .and(|r: &Request| {
            has(r, "channel_id", "C0000001")
                && has(r, "thread_ts", "1794500969.628009")
                && has(r, "initial_comment", "look")
                && String::from_utf8_lossy(&r.body).contains("F0000010")
        })
        .respond_with(json(fixture("complete_upload.json")))
        .expect(1)
        .mount(&api)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("photo.png");
    std::fs::write(&p, b"hello").unwrap();
    let out = client_for(&api, &[&files])
        .upload_files(
            "C0000001",
            Some("1794500969.628009"),
            &[(&p, "photo.png", None)],
            Some("look"),
        )
        .await
        .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id, "F0000010");

    let order: Vec<String> = api
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_string())
        .collect();
    assert_eq!(
        order,
        [
            "/api/files.getUploadURLExternal",
            "/api/files.completeUploadExternal"
        ]
    );
}

async fn file_server(body: &'static [u8]) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .and(header_exists("authorization"))
        .and(header("cookie", "d=xoxd-secretcookie%2Fvalue"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&s)
        .await;
    s
}

#[tokio::test]
async fn fetch_sends_auth_to_file_host() {
    let api = MockServer::start().await;
    let files = file_server(b"bytes").await;
    let c = client_for(&api, &[&files]);
    let got = c
        .fetch_private(&format!("{}/f", files.uri()), 100)
        .await
        .unwrap();
    assert_eq!(got, b"bytes");
    let req = &files.received_requests().await.unwrap()[0];
    assert_eq!(
        req.headers.get("authorization").unwrap().to_str().unwrap(),
        "Bearer xoxc-1-2-3-abcdef"
    );

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("out.bin");
    let n = c
        .download_to(&format!("{}/f", files.uri()), &dest)
        .await
        .unwrap();
    assert_eq!(n, 5);
    assert_eq!(std::fs::read(&dest).unwrap(), b"bytes");
    assert!(!dir.path().join("out.bin.part").exists());
}

#[tokio::test]
async fn fetch_refuses_foreign_host() {
    let api = MockServer::start().await;
    let evil = MockServer::start().await;
    // Default host list: neither mock server is a file host.
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&api.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    let c = WebClient::new(
        cfg,
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap();
    for url in [
        "https://evil.example.test/x".to_string(),
        format!("{}/x", evil.uri()),
        "https://files.slack.com.evil.example.test/x".to_string(),
    ] {
        assert_eq!(c.fetch_private(&url, 10).await, Err(ApiError::ForeignHost));
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            c.download_to(&url, &dir.path().join("x")).await,
            Err(ApiError::ForeignHost)
        );
    }
    assert!(evil.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn redirect_to_foreign_host_drops_auth() {
    let api = MockServer::start().await;
    let origin = MockServer::start().await;
    let other = MockServer::start().await;
    let stranger = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(&b"ok"[..]))
        .mount(&other)
        .await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/f", other.uri())),
        )
        .mount(&origin)
        .await;
    Mock::given(method("GET"))
        .and(path("/evil"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/f", stranger.uri())),
        )
        .mount(&origin)
        .await;

    // `other` is an allowed file host, `stranger` is not.
    let c = client_for(&api, &[&origin, &other]);
    let got = c
        .fetch_private(&format!("{}/start", origin.uri()), 100)
        .await
        .unwrap();
    assert_eq!(got, b"ok");
    let first = &origin.received_requests().await.unwrap()[0];
    assert!(first.headers.contains_key("authorization") && first.headers.contains_key("cookie"));
    let hop = &other.received_requests().await.unwrap()[0];
    assert!(!hop.headers.contains_key("authorization"));
    assert!(!hop.headers.contains_key("cookie"));

    let r = c
        .fetch_private(&format!("{}/evil", origin.uri()), 100)
        .await;
    assert_eq!(r, Err(ApiError::ForeignHost));
    assert!(stranger.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn fetch_aborts_over_max_bytes() {
    let api = MockServer::start().await;
    let files = file_server(b"0123456789").await;
    let c = client_for(&api, &[&files]);
    let r = c.fetch_private(&format!("{}/f", files.uri()), 4).await;
    assert!(matches!(r, Err(ApiError::TooLarge { .. })), "{r:?}");
}

#[test]
fn plain_http_file_url_refused() {
    assert!(!is_file_host(
        &Url::parse("http://files.slack.com/x").unwrap()
    ));
    assert!(is_file_host(
        &Url::parse("https://files.slack.com/x").unwrap()
    ));
    assert!(is_file_host(
        &Url::parse("https://slack-files.com/x").unwrap()
    ));
    assert!(is_file_host(
        &Url::parse("https://a.slack-edge.com/x").unwrap()
    ));
    assert!(is_file_host(
        &Url::parse("https://example.slack.com/x").unwrap()
    ));
    assert!(!is_file_host(
        &Url::parse("https://evilslack.com/x").unwrap()
    ));
    assert!(!is_file_host(
        &Url::parse("https://slack.com.evil.test/x").unwrap()
    ));
    assert!(!is_file_host(
        &Url::parse("https://files.slack.com:8443/x").unwrap()
    ));
}

#[tokio::test]
async fn upload_of_a_snippet_sends_its_snippet_type() {
    let api = MockServer::start().await;
    let files = MockServer::start().await;
    let upload_url = format!("{}/upload/v1/EXAMPLE", files.uri());
    let body = fixture("get_upload_url.json").replace(
        "https://files.example.test/upload/v1/EXAMPLEUPLOADTOKEN",
        &upload_url,
    );
    Mock::given(method("POST"))
        .and(path("/api/files.getUploadURLExternal"))
        .and(|r: &Request| has(r, "filename", "main.rs") && has(r, "snippet_type", "rust"))
        .respond_with(json(body))
        .expect(1)
        .mount(&api)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/EXAMPLE"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&files)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/files.completeUploadExternal"))
        .respond_with(json(fixture("complete_upload.json")))
        .mount(&api)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("main.rs");
    std::fs::write(&p, b"fn main() {}").unwrap();
    client_for(&api, &[&files])
        .upload_files("C0000001", None, &[(&p, "main.rs", Some("rust"))], None)
        .await
        .unwrap();
}
