use std::sync::Arc;
use std::time::Duration;

use redact::record::Recorder;
use slack_api::{ClientConfig, Priority, SessionCookie, WebClient, XoxcToken, method};
use slack_redact::Rules;
use url::Url;
use wiremock::matchers::{method as http_method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "xoxc-1-2-3-abcdef";
const COOKIE: &str = "xoxd-secretcookie%2Fvalue";

#[tokio::test]
async fn record_writes_redacted_exchange() {
    let server = MockServer::start().await;
    let body = std::fs::read_to_string(format!(
        "{}/tests/fixtures/auth/auth_test_ok.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    Mock::given(http_method("POST"))
        .and(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::new(dir.path(), Arc::new(Rules::default())).unwrap();
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    let client = WebClient::new(
        cfg,
        XoxcToken::parse(TOKEN).unwrap(),
        SessionCookie::parse(COOKIE).unwrap(),
    )
    .unwrap()
    .with_recorder(Arc::new(rec));
    let _: serde_json::Value = client
        .call(method::AUTH_TEST, &[], Priority::User)
        .await
        .unwrap();

    let file = dir.path().join("0001-auth.test.json");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("xox"), "{text}");
    assert!(!text.contains("secretcookie"), "{text}");
    assert!(text.contains("\"status\": 200"));
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        assert!(redact::check_text(&text, &Rules::default()).is_empty());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
