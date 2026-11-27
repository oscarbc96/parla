//! Email-code sign-in calls against wiremock replaying the synthetic fixtures in
//! `fixtures/signin-email-code/` (hypothesis until the owner's recon run).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use slack_api::{
    ApiError, EmailConfirmation, Secret, SendCodeOutcome, confirm_code, find_workspaces_for_email,
    send_code,
};
use url::Url;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const EMAIL: &str = "person@example.test";
const CODE: &str = "123456";

fn fixture(name: &str) -> serde_json::Value {
    let raw = std::fs::read_to_string(format!(
        "{}/tests/fixtures/signin-email-code/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    serde_json::from_str(&raw).unwrap()
}

/// The recorded reply: status, headers and JSON body.
fn reply(name: &str) -> ResponseTemplate {
    let v = fixture(name);
    let r = &v["response"];
    let mut t = ResponseTemplate::new(r["status"].as_u64().unwrap() as u16)
        .set_body_raw(r["body"]["json"].to_string(), "application/json");
    for h in r["headers"].as_array().unwrap() {
        let (k, val) = (h[0].as_str().unwrap(), h[1].as_str().unwrap());
        if k != "content-type" {
            t = t.append_header(k, val);
        }
    }
    t
}

/// The recorded request's form field names.
fn form_fields(name: &str) -> Vec<String> {
    fixture(name)["request"]["body"]["form"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f[0].as_str().unwrap().to_string())
        .collect()
}

async fn base(server: &MockServer) -> Url {
    Url::parse(&server.uri()).unwrap()
}

fn only_request(reqs: &[Request]) -> &Request {
    assert_eq!(reqs.len(), 1, "one attempt, no retry");
    &reqs[0]
}

fn body(r: &Request) -> String {
    String::from_utf8_lossy(&r.body).into_owned()
}

#[tokio::test]
async fn send_code_posts_the_recorded_form_without_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signup.confirmEmail"))
        .and(header("origin", "https://slack.com"))
        .respond_with(reply("0001-signup.confirmEmail.json"))
        .mount(&server)
        .await;
    let out = send_code(&base(&server).await, EMAIL, "en-US", false)
        .await
        .unwrap();
    assert!(matches!(out, SendCodeOutcome::Sent { .. }), "{out:?}");
    let reqs = server.received_requests().await.unwrap();
    let r = only_request(&reqs);
    let b = body(r);
    for f in form_fields("0001-signup.confirmEmail.json") {
        assert!(b.contains(&format!("name=\"{f}\"")), "field {f}");
    }
    assert!(b.contains(EMAIL));
    assert!(!b.contains("name=\"resend\""));
    assert!(!b.contains("name=\"token\""));
    assert!(r.headers.get("cookie").is_none(), "no d cookie");
    assert!(r.headers.get("referer").is_some());
}

#[tokio::test]
async fn resend_sends_the_resend_variant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signup.confirmEmail"))
        .and(body_string_contains("get_started_resend_code"))
        .respond_with(reply("0002-signup.confirmEmail-resend.json"))
        .mount(&server)
        .await;
    let out = send_code(&base(&server).await, EMAIL, "en-US", true)
        .await
        .unwrap();
    assert!(matches!(out, SendCodeOutcome::Sent { .. }));
    let reqs = server.received_requests().await.unwrap();
    let b = body(only_request(&reqs));
    for f in form_fields("0002-signup.confirmEmail-resend.json") {
        assert!(b.contains(&format!("name=\"{f}\"")), "field {f}");
    }
}

async fn send_with(name: &str) -> Result<SendCodeOutcome, ApiError> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signup.confirmEmail"))
        .respond_with(reply(name))
        .mount(&server)
        .await;
    let out = send_code(&base(&server).await, EMAIL, "en-US", false).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    out
}

#[tokio::test]
async fn send_code_maps_each_recorded_answer() {
    assert!(matches!(
        send_with("0003-signup.confirmEmail-sso.json").await,
        Ok(SendCodeOutcome::NeedsSso)
    ));
    assert!(matches!(
        send_with("0004-signup.confirmEmail-password.json").await,
        Ok(SendCodeOutcome::NeedsPassword)
    ));
    let invalid = send_with("0005-signup.confirmEmail-invalid.json")
        .await
        .unwrap_err();
    assert!(
        matches!(invalid, ApiError::Slack { ref error, .. } if error == "invalid_email"),
        "{invalid:?}"
    );
    let limited = send_with("0006-signup.confirmEmail-ratelimited.json")
        .await
        .unwrap_err();
    assert_eq!(
        limited,
        ApiError::RateLimited {
            retry_after: Duration::from_secs(30)
        }
    );
}

async fn confirm_with(name: &str) -> (Result<EmailConfirmation, ApiError>, String) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signin.confirmCode"))
        .respond_with(reply(name))
        .mount(&server)
        .await;
    let code = Secret::new(CODE.to_string());
    let out = confirm_code(&base(&server).await, EMAIL, &code).await;
    let reqs = server.received_requests().await.unwrap();
    let b = body(only_request(&reqs));
    (out, b)
}

#[tokio::test]
async fn confirm_code_keeps_only_the_confirmation_cookies() {
    let (out, b) = confirm_with("0010-signin.confirmCode.json").await;
    for f in form_fields("0010-signin.confirmCode.json") {
        assert!(b.contains(&format!("name=\"{f}\"")), "field {f}");
    }
    assert!(b.contains(CODE), "the code is sent");
    let conf = out.unwrap();
    assert_eq!(conf.cookie_count(), 2, "ec and pec");
    let dbg = format!("{conf:?}");
    assert!(!dbg.contains("REDACTED=") && !dbg.contains("ec="), "{dbg}");
}

#[tokio::test]
async fn confirm_code_ignores_cookies_for_other_hosts_and_names() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signin.confirmCode"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(r#"{"ok":true}"#, "application/json")
                .append_header(
                    "set-cookie",
                    "ec=fromelsewhere; Domain=.evil.example; Path=/",
                )
                .append_header("set-cookie", "tracker=1; Domain=.slack.com; Path=/")
                .append_header("set-cookie", "pec=kept-value; Domain=slack.com; Path=/"),
        )
        .mount(&server)
        .await;
    let code = Secret::new(CODE.to_string());
    let conf = confirm_code(&base(&server).await, EMAIL, &code)
        .await
        .unwrap();
    assert_eq!(conf.cookie_count(), 1);
    assert!(!format!("{conf:?}").contains("kept-value"));
}

#[tokio::test]
async fn confirm_code_without_a_confirmation_cookie_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signin.confirmCode"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"ok":true}"#, "application/json"))
        .mount(&server)
        .await;
    let code = Secret::new(CODE.to_string());
    let err = confirm_code(&base(&server).await, EMAIL, &code)
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Decode { .. }), "{err:?}");
}

#[tokio::test]
async fn confirm_code_maps_wrong_expired_and_too_many_distinctly() {
    let (wrong, _) = confirm_with("0007-signin.confirmCode-wrong.json").await;
    assert_eq!(
        wrong.unwrap_err(),
        ApiError::WrongCode {
            attempts_left: Some(4)
        }
    );
    let (expired, _) = confirm_with("0008-signin.confirmCode-expired.json").await;
    assert_eq!(expired.unwrap_err(), ApiError::CodeExpired);
    let (many, _) = confirm_with("0009-signin.confirmCode-too-many.json").await;
    assert_eq!(many.unwrap_err(), ApiError::TooManyAttempts);
}

/// Collects every tracing event and span field the calls emit.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<String>>);

struct Fields<'a>(&'a mut String);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={v:?} ", f.name()));
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, a: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        a.record(&mut Fields(&mut self.0.lock().unwrap()));
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, v: &tracing::span::Record<'_>) {
        v.record(&mut Fields(&mut self.0.lock().unwrap()));
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, e: &tracing::Event<'_>) {
        e.record(&mut Fields(&mut self.0.lock().unwrap()));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn no_email_code_or_cookie_reaches_tracing() {
    let cap = Capture::default();
    let _guard = tracing::subscriber::set_default(cap.clone());
    tracing::info!(probe = "capture works");
    let (conf, _) = confirm_with("0010-signin.confirmCode.json").await;
    let conf = conf.unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signin.findWorkspaces"))
        .respond_with(reply("0011-signin.findWorkspaces-email.json"))
        .mount(&server)
        .await;
    find_workspaces_for_email(&base(&server).await, &conf, EMAIL)
        .await
        .unwrap();
    let _ = format!("{conf:?}");
    let out = cap.0.lock().unwrap().clone();
    assert!(out.contains("capture works"), "the capture is live");
    for secret in [EMAIL, CODE, "REDACTED", "ec=", "pec="] {
        assert!(!out.contains(secret), "{secret} in tracing output");
    }
}

#[tokio::test]
async fn workspaces_for_email_sends_the_chooser_variant_with_the_confirmation() {
    let (conf, _) = confirm_with("0010-signin.confirmCode.json").await;
    let conf = conf.unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signin.findWorkspaces"))
        .respond_with(reply("0011-signin.findWorkspaces-email.json"))
        .mount(&server)
        .await;
    let rows = find_workspaces_for_email(&base(&server).await, &conf, EMAIL)
        .await
        .unwrap();
    let reqs = server.received_requests().await.unwrap();
    let r = only_request(&reqs);
    let b = body(r);
    for f in form_fields("0011-signin.findWorkspaces-email.json") {
        assert!(b.contains(&format!("name=\"{f}\"")), "field {f}");
    }
    assert!(b.contains("get_started_workspaces") && b.contains(EMAIL));
    let cookie = r.headers.get("cookie").unwrap().to_str().unwrap();
    assert!(
        cookie.contains("ec=") && cookie.contains("pec="),
        "confirmation cookies"
    );
    assert!(!cookie.contains("d="), "no session cookie");
    // The team without an id is skipped.
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].team.id, "T0000001");
    assert_eq!(rows[0].team.domain, "workspace1");
    assert!(rows[1].team.sso_required);
    // The fixture's links are redacted placeholders, not URLs: dropped.
    assert!(rows.iter().all(|r| r.login_link.is_none()));
}

#[tokio::test]
async fn workspaces_for_email_keeps_only_provider_links() {
    let (conf, _) = confirm_with("0010-signin.confirmCode.json").await;
    let conf = conf.unwrap();
    let body = r#"{"ok":true,"current_teams":[{"email":"person@example.test","teams":[
        {"id":"T0000001","name":"One","domain":"one",
         "magic_login_url":"https://app.slack.com/t/one/login/abc123"},
        {"id":"T0000002","name":"Two","domain":"two",
         "magic_login_url":"https://evil.example/t/two/login/abc123"},
        {"id":"T0000003","name":"Three","domain":"three",
         "magic_login_url":"http://app.slack.com/t/three/login/abc123"},
        {"id":"T0000001","name":"One again","domain":"one"}]}]}"#;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/signin.findWorkspaces"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;
    let rows = find_workspaces_for_email(&base(&server).await, &conf, EMAIL)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3, "deduplicated, rows kept without a link");
    assert_eq!(
        rows[0].login_link.as_ref().map(Secret::expose),
        Some("https://app.slack.com/t/one/login/abc123")
    );
    assert!(rows[1].login_link.is_none(), "foreign host dropped");
    assert!(rows[2].login_link.is_none(), "plain http dropped");
    assert!(!format!("{:?}", rows[0]).contains("abc123"));
}
