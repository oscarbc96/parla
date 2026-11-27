//! `usergroups.list` (synthetic fixture shaped after the public method; the web
//! client's own call is not captured yet, see docs/providers/slack/web-client/people.md).

use std::time::Duration;

use slack_api::{ClientConfig, SessionCookie, WebClient, XoxcToken};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn client(server: &MockServer) -> WebClient {
    let mut cfg = ClientConfig::for_workspace(&Url::parse(&server.uri()).unwrap());
    cfg.timeout = Duration::from_secs(5);
    cfg.retry_base_delay = Duration::from_millis(10);
    WebClient::new(
        cfg,
        XoxcToken::parse("xoxc-1-2-3-abcdef").unwrap(),
        SessionCookie::parse("xoxd-secretcookie%2Fvalue").unwrap(),
    )
    .unwrap()
}

fn has(req: &Request, name: &str, value: &str) -> bool {
    String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\"\r\n\r\n{value}"))
}

#[tokio::test]
async fn usergroups_list_keeps_live_groups_with_counts_and_members() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/usergroups.list"))
        .and(|r: &Request| {
            has(r, "include_count", "true")
                && has(r, "include_users", "true")
                && has(r, "include_disabled", "false")
        })
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(fixture("usergroups/list.json"), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let all = client(&server).usergroups_list().await.unwrap();
    let ids: Vec<_> = all.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(
        ids,
        ["S0000001", "S0000002"],
        "deleted and id-less groups are dropped"
    );
    assert_eq!(all[0].handle, "eng");
    assert_eq!(all[0].name, "Engineering");
    assert_eq!(all[0].user_count, 3);
    assert_eq!(all[0].users, ["U0000001", "U0000002", "U0000003"]);
    assert_eq!(all[1].user_count, 2, "a string count is read too");
}
