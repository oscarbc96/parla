//! `parla::media::Fetcher`: provider URLs go through the provider, web URLs
//! through a credential-free https-only client.

use std::sync::Arc;

use components::media::{MediaError, MediaFetcher};
use domain::test_support::{Call, FakeProvider, uid};
use domain::{ChatProvider, ImageRef, RemoteUrl};
use parla::media::Fetcher;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider() -> Arc<FakeProvider> {
    Arc::new(FakeProvider::new("w1", uid("u1")))
}

#[tokio::test]
async fn provider_url_goes_through_provider() {
    let p = provider();
    let url = RemoteUrl("https://files.example.test/a.png".into());
    p.add_file(url.clone(), vec![1, 2, 3]);
    let dynp: Arc<dyn ChatProvider> = p.clone();
    let f = Fetcher::new(dynp, None);
    let got = f.fetch(&ImageRef::Provider(url.clone()), 1024).await;
    assert_eq!(got, Ok(vec![1, 2, 3]));
    assert!(p.calls().contains(&Call::Fetch(url)));
}

#[tokio::test]
async fn web_http_is_refused_without_a_request() {
    let f = Fetcher::new(provider(), None);
    let got = f
        .fetch(&ImageRef::Web("http://example.invalid/a.png".into()), 1024)
        .await;
    assert_eq!(got, Err(MediaError::Refused));
    let got = f
        .fetch(&ImageRef::Web("file:///etc/passwd".into()), 1024)
        .await;
    assert_eq!(got, Err(MediaError::Refused));
}

#[tokio::test]
async fn web_fetch_sends_no_cookie_and_no_authorization() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![7u8; 10]))
        .mount(&server)
        .await;
    let p = provider();
    let f = Fetcher::allow_http_for_tests(p.clone());
    let url = format!("{}/p.png", server.uri());
    let got = f.fetch(&ImageRef::Web(url.into()), 1024).await;
    assert_eq!(got, Ok(vec![7u8; 10]));
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    assert!(reqs[0].headers.get("cookie").is_none());
    assert!(reqs[0].headers.get("authorization").is_none());
    // The provider was never involved.
    assert!(p.calls().is_empty());
}

#[tokio::test]
async fn oversize_refused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 5000]))
        .mount(&server)
        .await;
    let f = Fetcher::allow_http_for_tests(provider());
    let got = f
        .fetch(
            &ImageRef::Web(format!("{}/big.png", server.uri()).into()),
            1000,
        )
        .await;
    assert_eq!(got, Err(MediaError::TooLarge));
}

#[tokio::test]
async fn non_success_status_is_a_network_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let f = Fetcher::allow_http_for_tests(provider());
    let got = f
        .fetch(
            &ImageRef::Web(format!("{}/x.png", server.uri()).into()),
            1000,
        )
        .await;
    assert!(matches!(got, Err(MediaError::Network(_))));
}

#[tokio::test]
async fn a_provider_fetch_over_the_limit_is_too_large() {
    let p = provider();
    let url = RemoteUrl("https://files.example.test/big.rs".into());
    p.add_file(url.clone(), vec![b'x'; 10]);
    let dynp: Arc<dyn ChatProvider> = p;
    let f = Fetcher::new(dynp, None);
    let got = f.fetch(&ImageRef::Provider(url), 3).await;
    assert_eq!(got, Err(MediaError::TooLarge));
}

#[test]
fn dropping_a_fetch_aborts_its_runtime_task() {
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    struct Flag(Arc<AtomicBool>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(true, SeqCst);
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = Flag(dropped.clone());
    let fut = parla::media::spawn_aborting(
        rt.handle(),
        Box::pin(async move {
            let _f = flag;
            futures::future::pending::<Result<Vec<u8>, MediaError>>().await
        }),
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(!dropped.load(SeqCst), "running");
    drop(fut);
    for _ in 0..100 {
        if dropped.load(SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        dropped.load(SeqCst),
        "the task was aborted, not left running"
    );
}
