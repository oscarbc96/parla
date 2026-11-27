use slack_api::{ClientConfig, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn client(server: &MockServer) -> WebClient {
    let cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    WebClient::new(
        cfg,
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap()
}

async fn serve(server: &MockServer, name: &str) {
    Mock::given(method("POST"))
        .and(path("/api/team.info"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(fixture(name), "application/json"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn team_info_parses_icon_and_enterprise() {
    let server = MockServer::start().await;
    serve(&server, "team/info.json").await;
    let t = client(&server).team_info().await.unwrap();
    assert_eq!(t.id, "T0000001");
    assert_eq!(t.name, "Example Team");
    assert_eq!(t.icon.as_deref(), Some("https://example.com/icons/132.png"));
    assert_eq!(t.enterprise_id, None);
    assert_eq!(t.enterprise_name, None);
}

#[tokio::test]
async fn team_info_enterprise_and_default_icon_dropped() {
    let server = MockServer::start().await;
    serve(&server, "team/info_enterprise.json").await;
    let t = client(&server).team_info().await.unwrap();
    assert_eq!(t.enterprise_id.as_deref(), Some("E0000001"));
    assert_eq!(t.enterprise_name.as_deref(), Some("Example Org"));
    // `image_default: true` means no custom icon was uploaded.
    assert_eq!(t.icon, None);
}

#[tokio::test]
async fn team_info_is_a_single_attempt() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/team.info"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    assert!(client(&server).team_info().await.is_err());
    server.verify().await;
}
