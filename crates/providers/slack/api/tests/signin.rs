use slack_api::{ApiError, SessionCookie, find_workspaces};
use url::Url;
use wiremock::matchers::{body_string_contains, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The recorded exchange's response body.
fn response(name: &str) -> String {
    let raw = std::fs::read_to_string(format!(
        "{}/tests/fixtures/signin-find-workspaces/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    v["response"]["body"]["json"].to_string()
}

async fn serve(server: &MockServer, name: &str) {
    Mock::given(method("POST"))
        .and(path("/api/signin.findWorkspaces"))
        .and(header_exists("cookie"))
        .and(body_string_contains("only_logged_in_teams"))
        .and(body_string_contains("ssb_signin"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(response(name), "application/json"))
        .mount(server)
        .await;
}

fn cookie() -> SessionCookie {
    SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap()
}

#[tokio::test]
async fn lists_signed_in_workspaces() {
    let server = MockServer::start().await;
    serve(&server, "0045-signin.findWorkspaces.json").await;
    let base = Url::parse(&server.uri()).unwrap();
    let ws = find_workspaces(&base, &cookie()).await.unwrap();
    assert!(!ws.is_empty());
    for w in &ws {
        assert!(w.id.starts_with('T'));
        assert!(!w.domain.is_empty());
        assert!(!w.name.is_empty());
        assert!(w.icon.as_deref().is_none_or(|u| u.starts_with("https://")));
    }
    let mut ids: Vec<_> = ws.iter().map(|w| &w.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), ws.len(), "ids are unique");
}

#[tokio::test]
async fn maps_ratelimited_and_invalid_auth_replies() {
    for (body, want_rate_limited) in [
        (r#"{"ok":false,"error":"ratelimited"}"#, true),
        (r#"{"ok":false,"error":"invalid_auth"}"#, false),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/signin.findWorkspaces"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .mount(&server)
            .await;
        let base = Url::parse(&server.uri()).unwrap();
        let err = find_workspaces(&base, &cookie()).await.unwrap_err();
        if want_rate_limited {
            assert!(matches!(err, ApiError::RateLimited { .. }), "{err:?}");
        } else {
            assert!(matches!(err, ApiError::InvalidAuth), "{err:?}");
        }
    }
}

#[tokio::test]
async fn other_server_errors_stay_slack_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signin.findWorkspaces"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(r#"{"ok":false,"error":"nope"}"#, "application/json"),
        )
        .mount(&server)
        .await;
    let base = Url::parse(&server.uri()).unwrap();
    let err = find_workspaces(&base, &cookie()).await.unwrap_err();
    assert!(
        matches!(err, ApiError::Slack { ref error, .. } if error == "nope"),
        "{err:?}"
    );
}

#[tokio::test]
async fn merges_accounts_and_skips_teams_without_an_id() {
    let body = r#"{"ok":true,"current_teams":[
      {"email":"a@example.test","teams":[
        {"id":"T0000001","name":"One","domain":"one","sso_required":true},
        {"name":"No id","domain":"noid"}]},
      {"email":"b@example.test","teams":[
        {"id":"T0000001","name":"One again","domain":"one"},
        {"id":"T0000002","name":"Two","domain":"two","is_logged_out":true,
         "two_factor_required":true}]}]}"#;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signin.findWorkspaces"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;
    let base = Url::parse(&server.uri()).unwrap();
    let ws = find_workspaces(&base, &cookie()).await.unwrap();
    assert_eq!(ws.len(), 2);
    assert!(ws[0].sso_required && !ws[0].is_logged_out);
    assert!(ws[1].is_logged_out && ws[1].two_factor_required);
    assert_eq!(ws[1].domain, "two");
}
