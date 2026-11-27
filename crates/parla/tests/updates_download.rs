use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use parla::updates::download::{DownloadError, Downloader, MAX_SIGNATURE_BYTES, MAX_UPDATE_BYTES};
use parla::updates::package::{Asset, UpdatePackage, VerifyError, trusted_comment};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const DMG: &str = "Parla-0.6.0-macos-universal.dmg";
const SIG: &str = "Parla-0.6.0-macos-universal.dmg.minisig";
const BODY: &[u8] = b"pretend this is a disk image";

fn current() -> semver::Version {
    semver::Version::parse("0.5.0").unwrap()
}

fn version() -> semver::Version {
    semver::Version::parse("0.6.0").unwrap()
}

struct Keys {
    pk: String,
    sk: minisign::SecretKey,
    pkey: minisign::PublicKey,
}

fn keys() -> Keys {
    let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    Keys {
        pk: kp.pk.to_base64(),
        sk: kp.sk,
        pkey: kp.pk,
    }
}

fn sign(k: &Keys, data: &[u8]) -> String {
    minisign::sign(
        Some(&k.pkey),
        &k.sk,
        Cursor::new(data),
        Some(&trusted_comment(&version(), DMG)),
        None,
    )
    .unwrap()
    .into_string()
}

fn package(server: &MockServer, size: u64) -> UpdatePackage {
    let asset = |name: &str, size| Asset {
        name: name.to_string(),
        url: url::Url::parse(&format!("{}/{name}", server.uri())).unwrap(),
        size,
    };
    UpdatePackage {
        version: version(),
        tag: "v0.6.0".to_string(),
        dmg: asset(DMG, size),
        signature: asset(SIG, 0),
    }
}

fn downloader(dir: &Path) -> Downloader {
    Downloader::new_allow_http(dir.to_path_buf(), &current())
}

async fn mount_package(server: &MockServer, body: &[u8], signature: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/{SIG}")))
        .respond_with(ResponseTemplate::new(200).set_body_string(signature))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{DMG}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(server)
        .await;
}

fn assert_no_package_files(dir: &Path) {
    assert!(!dir.join(DMG).exists());
    assert!(!dir.join(format!("{DMG}.part")).exists());
}

#[tokio::test]
async fn good_download_verifies_renames_and_reports_progress() {
    let server = MockServer::start().await;
    let k = keys();
    mount_package(&server, BODY, &sign(&k, BODY)).await;
    let dir = tempfile::tempdir().unwrap();
    let progress_events = Arc::new(Mutex::new(Vec::new()));
    let events = progress_events.clone();
    let progress = move |received, total| events.lock().unwrap().push((received, total));

    let path = downloader(dir.path())
        .download(&package(&server, BODY.len() as u64), &k.pk, &progress)
        .await
        .unwrap();

    assert_eq!(path, dir.path().join(DMG));
    assert_eq!(std::fs::read(&path).unwrap(), BODY);
    assert!(!dir.path().join(format!("{DMG}.part")).exists());
    let events = progress_events.lock().unwrap();
    assert_eq!(events.last(), Some(&(BODY.len() as u64, BODY.len() as u64)));
}

#[tokio::test]
async fn bad_signature_deletes_the_download() {
    let server = MockServer::start().await;
    let k = keys();
    mount_package(&server, BODY, &sign(&k, b"different bytes")).await;
    let dir = tempfile::tempdir().unwrap();

    let result = downloader(dir.path())
        .download(&package(&server, BODY.len() as u64), &k.pk, &|_, _| {})
        .await;

    assert!(matches!(
        result,
        Err(DownloadError::Verify(VerifyError::BadSignature))
    ));
    assert_no_package_files(dir.path());
}

#[tokio::test]
async fn body_longer_than_declared_is_too_large() {
    let server = MockServer::start().await;
    let k = keys();
    mount_package(&server, BODY, &sign(&k, BODY)).await;
    let dir = tempfile::tempdir().unwrap();

    let result = downloader(dir.path())
        .download(&package(&server, BODY.len() as u64 - 1), &k.pk, &|_, _| {})
        .await;

    assert_eq!(result, Err(DownloadError::TooLarge));
    assert_no_package_files(dir.path());
}

#[tokio::test]
async fn body_shorter_than_declared_is_a_size_mismatch() {
    let server = MockServer::start().await;
    let k = keys();
    mount_package(&server, BODY, &sign(&k, BODY)).await;
    let dir = tempfile::tempdir().unwrap();

    let result = downloader(dir.path())
        .download(&package(&server, BODY.len() as u64 + 1), &k.pk, &|_, _| {})
        .await;

    assert_eq!(result, Err(DownloadError::SizeMismatch));
    assert_no_package_files(dir.path());
}

#[tokio::test]
async fn zero_or_oversize_declarations_make_no_request() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let downloader = downloader(dir.path());

    for size in [0, MAX_UPDATE_BYTES + 1] {
        let result = downloader
            .download(&package(&server, size), "unused", &|_, _| {})
            .await;
        assert_eq!(result, Err(DownloadError::TooLarge));
    }

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn dmg_http_error_is_http_and_leaves_no_file() {
    let server = MockServer::start().await;
    let k = keys();
    Mock::given(method("GET"))
        .and(path(format!("/{SIG}")))
        .respond_with(ResponseTemplate::new(200).set_body_string(sign(&k, BODY)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{DMG}")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();

    let result = downloader(dir.path())
        .download(&package(&server, BODY.len() as u64), &k.pk, &|_, _| {})
        .await;

    assert_eq!(result, Err(DownloadError::Http));
    assert_no_package_files(dir.path());
}

#[tokio::test]
async fn signature_over_the_limit_is_too_large() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{SIG}")))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_SIGNATURE_BYTES + 1]),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();

    let result = downloader(dir.path())
        .download(&package(&server, BODY.len() as u64), "unused", &|_, _| {})
        .await;

    assert_eq!(result, Err(DownloadError::TooLarge));
    assert_no_package_files(dir.path());
}

#[tokio::test]
async fn old_files_are_removed_before_downloading() {
    let server = MockServer::start().await;
    let k = keys();
    mount_package(&server, BODY, &sign(&k, BODY)).await;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("updates");
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("old.dmg");
    let stale = dir.join("stale.dmg.part");
    std::fs::write(&old, b"old").unwrap();
    std::fs::write(&stale, b"stale").unwrap();

    downloader(&dir)
        .download(&package(&server, BODY.len() as u64), &k.pk, &|_, _| {})
        .await
        .unwrap();

    assert!(!old.exists());
    assert!(!stale.exists());
}

struct DelayedResponse {
    body: String,
    started: Mutex<Option<futures::channel::oneshot::Sender<()>>>,
}

impl Respond for DelayedResponse {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        ResponseTemplate::new(200)
            .set_body_string(self.body.clone())
            .set_delay(Duration::from_millis(250))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_download_is_busy() {
    let server = MockServer::start().await;
    let k = keys();
    let (started_tx, started_rx) = futures::channel::oneshot::channel();
    Mock::given(method("GET"))
        .and(path(format!("/{SIG}")))
        .respond_with(DelayedResponse {
            body: sign(&k, BODY),
            started: Mutex::new(Some(started_tx)),
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{DMG}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let downloader = Arc::new(downloader(dir.path()));
    let first_downloader = downloader.clone();
    let first_package = package(&server, BODY.len() as u64);
    let first_key = k.pk.clone();
    let first = tokio::spawn(async move {
        first_downloader
            .download(&first_package, &first_key, &|_, _| {})
            .await
    });

    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .unwrap()
        .unwrap();

    let second = downloader
        .download(&package(&server, BODY.len() as u64), &k.pk, &|_, _| {})
        .await;
    assert_eq!(second, Err(DownloadError::Busy));
    assert!(first.await.unwrap().is_ok());
}

#[tokio::test]
async fn requests_carry_only_accept_and_user_agent() {
    let server = MockServer::start().await;
    let k = keys();
    mount_package(&server, BODY, &sign(&k, BODY)).await;
    let dir = tempfile::tempdir().unwrap();

    downloader(dir.path())
        .download(&package(&server, BODY.len() as u64), &k.pk, &|_, _| {})
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(
            request.headers.get("accept").unwrap().to_str().unwrap(),
            "application/octet-stream"
        );
        assert_eq!(
            request.headers.get("user-agent").unwrap().to_str().unwrap(),
            "Parla/0.5.0"
        );
        assert!(request.headers.get("cookie").is_none());
        assert!(request.headers.get("authorization").is_none());
        let names: Vec<_> = request.headers.keys().map(|name| name.as_str()).collect();
        assert!(
            names
                .iter()
                .all(|name| matches!(*name, "accept" | "host" | "user-agent")),
            "unexpected request headers: {names:?}"
        );
    }
}

#[tokio::test]
async fn production_downloader_refuses_http_without_a_request() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let downloader = Downloader::new(dir.path().to_path_buf(), &current());

    let result = downloader
        .download(&package(&server, BODY.len() as u64), "unused", &|_, _| {})
        .await;

    assert_eq!(result, Err(DownloadError::Insecure));
    assert!(server.received_requests().await.unwrap().is_empty());
}
