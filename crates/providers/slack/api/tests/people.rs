//! Profiles, status, presence, DND and notification prefs against synthetic fixtures.
//! The endpoints are unverified against a capture.

use std::time::Duration;

use slack_api::dnd::DND_TEAM_BATCH;
use slack_api::{ApiError, ClientConfig, PresenceMode, SessionCookie, WebClient, XoxcToken};
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

fn json(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

fn has(req: &Request, name: &str, value: &str) -> bool {
    String::from_utf8_lossy(&req.body).contains(&format!("name=\"{name}\"\r\n\r\n{value}\r\n"))
}

fn field(req: &Request, name: &str) -> Option<String> {
    let b = String::from_utf8_lossy(&req.body).to_string();
    let key = format!("name=\"{name}\"\r\n\r\n");
    let start = b.find(&key)? + key.len();
    let end = b[start..].find("\r\n")? + start;
    Some(b[start..end].to_string())
}

async fn mount(server: &MockServer, m: &str, body: String) {
    Mock::given(method("POST"))
        .and(path(format!("/api/{m}")))
        .respond_with(json(body))
        .mount(server)
        .await;
}

async fn mount_profile(server: &MockServer, profile: &str, info: &str) {
    mount(server, "users.profile.get", fixture(profile)).await;
    mount(server, "users.info", fixture(info)).await;
}

#[tokio::test]
async fn profile_maps_fields_and_tz() {
    let server = MockServer::start().await;
    mount_profile(&server, "people/profile_get.json", "people/user_info.json").await;
    let p = client(&server).profile_get("U0000002").await.unwrap();
    assert_eq!(p.id, "U0000002");
    assert_eq!(p.display_name, "Alex");
    assert_eq!(p.real_name, "Alex Example");
    assert_eq!(p.title, "Engineer");
    assert_eq!(p.pronouns, "they/them");
    assert_eq!(p.tz.as_deref(), Some("Europe/Madrid"));
    assert_eq!(p.status_text, "In a meeting");
    assert_eq!(p.status_emoji, ":spiral_calendar_pad:");
    assert_eq!(p.status_expiration, 1_800_000_000);
    assert_eq!(
        p.image.as_deref(),
        Some("https://files.example.test/image/192")
    );
    // Empty values dropped, sorted by label.
    assert_eq!(
        p.fields,
        vec![
            ("Location".to_string(), "Remote".to_string()),
            ("Team".to_string(), "Zed".to_string())
        ]
    );
}

#[tokio::test]
async fn profile_tolerates_nulls_and_empty_fields_array() {
    let server = MockServer::start().await;
    mount_profile(
        &server,
        "people/profile_nulls.json",
        "people/user_info_nulls.json",
    )
    .await;
    let p = client(&server).profile_get("U0000003").await.unwrap();
    assert_eq!(p.real_name, "Pat Example");
    assert_eq!(p.display_name, "");
    assert_eq!(p.tz, None);
    assert!(p.fields.is_empty());
    assert_eq!(p.status_expiration, 0);
    assert!(!p.deleted && !p.is_bot);
}

#[tokio::test]
async fn profile_error_code_stays_structured() {
    let server = MockServer::start().await;
    mount(
        &server,
        "users.profile.get",
        fixture("people/user_not_found.json"),
    )
    .await;
    mount(&server, "users.info", fixture("people/user_not_found.json")).await;
    let e = client(&server).profile_get("U0000009").await.unwrap_err();
    assert!(
        matches!(e, ApiError::Slack { ref error, .. } if error == "user_not_found"),
        "{e:?}"
    );
}

#[tokio::test]
async fn set_status_params_match_documented_form() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/users.profile.set"))
        .and(|r: &Request| {
            let Some(p) = field(r, "profile") else {
                return false;
            };
            let v: serde_json::Value = serde_json::from_str(&p).unwrap();
            v["status_text"] == "Lunch"
                && v["status_emoji"] == ":taco:"
                && v["status_expiration"] == 1_800_000_000i64
        })
        .respond_with(json(fixture("people/ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .profile_set_status("Lunch", ":taco:", 1_800_000_000)
        .await
        .unwrap();
}

#[tokio::test]
async fn clear_status_sends_empty_fields_and_zero_expiry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/users.profile.set"))
        .and(|r: &Request| {
            let v: serde_json::Value =
                serde_json::from_str(&field(r, "profile").unwrap_or_default()).unwrap_or_default();
            v["status_text"] == "" && v["status_emoji"] == "" && v["status_expiration"] == 0
        })
        .respond_with(json(fixture("people/ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server).profile_set_status("", "", 0).await.unwrap();
}

#[tokio::test]
async fn bad_emoji_stays_a_slack_error() {
    let server = MockServer::start().await;
    mount(
        &server,
        "users.profile.set",
        fixture("people/status_bad_emoji.json"),
    )
    .await;
    let e = client(&server)
        .profile_set_status("x", ":nope:", 0)
        .await
        .unwrap_err();
    assert!(
        matches!(e, ApiError::Slack { ref error, .. } if error == "profile_status_set_failed_not_valid_emoji"),
        "{e:?}"
    );
}

#[tokio::test]
async fn presence_params() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/users.setPresence"))
        .and(|r: &Request| has(r, "presence", "away"))
        .respond_with(json(fixture("people/ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/users.setPresence"))
        .and(|r: &Request| has(r, "presence", "auto"))
        .respond_with(json(fixture("people/ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    c.set_presence(PresenceMode::Away).await.unwrap();
    c.set_presence(PresenceMode::Auto).await.unwrap();
}

#[tokio::test]
async fn dnd_info_maps_fields_and_tolerates_nulls() {
    let server = MockServer::start().await;
    mount(&server, "dnd.info", fixture("dnd/info.json")).await;
    let d = client(&server).dnd_info().await.unwrap();
    assert!(d.dnd_enabled && d.snooze_enabled);
    assert_eq!(d.next_dnd_start_ts, 1_700_000_000);
    assert_eq!(d.next_dnd_end_ts, 1_700_030_000);
    assert_eq!(d.snooze_endtime, 1_700_001_800);

    let server = MockServer::start().await;
    mount(&server, "dnd.info", fixture("dnd/info_nulls.json")).await;
    let d = client(&server).dnd_info().await.unwrap();
    assert!(!d.dnd_enabled && !d.snooze_enabled);
    assert_eq!(d.next_dnd_start_ts, 0);
}

#[tokio::test]
async fn team_info_batches_drops_empty_ids_and_null_entries() {
    let server = MockServer::start().await;
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<usize>::new()));
    let s2 = seen.clone();
    Mock::given(method("POST"))
        .and(path("/api/dnd.teamInfo"))
        .and(move |r: &Request| {
            let n = field(r, "users").map_or(0, |u| u.split(',').count());
            s2.lock().unwrap().push(n);
            true
        })
        .respond_with(json(fixture("dnd/team_info.json")))
        .mount(&server)
        .await;
    let mut users: Vec<String> = (0..DND_TEAM_BATCH + 5)
        .map(|i| format!("U{i:07}"))
        .collect();
    users.push(String::new());
    users.push("U0000000".into()); // duplicate
    let got = client(&server).dnd_team_info(&users).await.unwrap();
    let sizes = seen.lock().unwrap().clone();
    assert_eq!(
        sizes,
        vec![DND_TEAM_BATCH, 5],
        "bounded batches, no empties or dupes"
    );
    // The fixture answers each batch the same way: two real entries, the empty id
    // and the null entry dropped.
    let ids: Vec<&str> = got.iter().map(|(u, _)| u.as_str()).collect();
    assert_eq!(ids, vec!["U0000002", "U0000003"]);
    assert!(got[0].1.dnd_enabled);
}

#[tokio::test]
async fn team_info_without_users_sends_nothing() {
    let server = MockServer::start().await;
    let got = client(&server).dnd_team_info(&[]).await.unwrap();
    assert!(got.is_empty());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn snooze_and_end_params() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/dnd.setSnooze"))
        .and(|r: &Request| has(r, "num_minutes", "30"))
        .respond_with(json(fixture("dnd/snooze_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/dnd.endSnooze"))
        .respond_with(json(fixture("dnd/end_ok.json")))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let d = c.dnd_set_snooze(30).await.unwrap();
    assert!(d.snooze_enabled);
    assert_eq!(d.snooze_endtime, 1_700_001_800);
    let d = c.dnd_end_snooze().await.unwrap();
    assert!(!d.snooze_enabled);
}

#[tokio::test]
async fn end_snooze_not_active_stays_a_slack_error() {
    let server = MockServer::start().await;
    mount(&server, "dnd.endSnooze", fixture("dnd/end_not_active.json")).await;
    let e = client(&server).dnd_end_snooze().await.unwrap_err();
    assert!(
        matches!(e, ApiError::Slack { ref error, .. } if error == "snooze_not_active"),
        "{e:?}"
    );
}

#[tokio::test]
async fn prefs_json_maps_levels_and_mutes() {
    let server = MockServer::start().await;
    mount(&server, "users.prefs.get", fixture("prefs/get.json")).await;
    let p = client(&server).prefs_get().await.unwrap();
    let by = |id: &str| p.channels.iter().find(|c| c.id == id).cloned();
    let c1 = by("C0000001").unwrap();
    assert_eq!(c1.desktop.as_deref(), Some("mentions"));
    assert!(!c1.muted);
    let c2 = by("C0000002").unwrap();
    assert_eq!(c2.desktop.as_deref(), Some("nothing"));
    assert!(c2.muted);
    // Muted only through the muted_channels list.
    let c3 = by("C0000003").unwrap();
    assert_eq!(c3.desktop, None);
    assert!(c3.muted);
    // Empty ids and null entries are dropped.
    assert!(p.channels.iter().all(|c| !c.id.is_empty()));
    assert!(by("C0000009").is_none());
    assert_eq!(p.channels.len(), 3);
}

#[tokio::test]
async fn prefs_flat_shape_and_nulls_and_garbage() {
    let server = MockServer::start().await;
    mount(&server, "users.prefs.get", fixture("prefs/get_flat.json")).await;
    let p = client(&server).prefs_get().await.unwrap();
    assert_eq!(p.channels.len(), 1);
    assert_eq!(p.channels[0].desktop.as_deref(), Some("everything"));

    let server = MockServer::start().await;
    mount(&server, "users.prefs.get", fixture("prefs/get_nulls.json")).await;
    assert!(
        client(&server)
            .prefs_get()
            .await
            .unwrap()
            .channels
            .is_empty()
    );

    let server = MockServer::start().await;
    mount(
        &server,
        "users.prefs.get",
        fixture("prefs/get_garbage.json"),
    )
    .await;
    let p = client(&server).prefs_get().await.unwrap();
    assert_eq!(p.channels.len(), 1, "the muted list still counts");
    assert!(p.channels[0].muted);
}

#[tokio::test]
async fn set_conversation_prefs_writes_merged_json_and_muted_list() {
    let server = MockServer::start().await;
    mount(&server, "users.prefs.get", fixture("prefs/get.json")).await;
    let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
    let w = writes.clone();
    Mock::given(method("POST"))
        .and(path("/api/users.prefs.set"))
        .and(move |r: &Request| {
            w.lock().unwrap().push((
                field(r, "name").unwrap_or_default(),
                field(r, "value").unwrap_or_default(),
            ));
            true
        })
        .respond_with(json(fixture("prefs/set_ok.json")))
        .mount(&server)
        .await;
    client(&server)
        .prefs_set_conversation("C0000001", Some("nothing"), true)
        .await
        .unwrap();
    let writes = writes.lock().unwrap().clone();
    let all = writes
        .iter()
        .find(|(n, _)| n == "all_notifications_prefs")
        .expect("all_notifications_prefs written");
    let v: serde_json::Value = serde_json::from_str(&all.1).unwrap();
    assert_eq!(v["channels"]["C0000001"]["desktop"], "nothing");
    assert_eq!(v["channels"]["C0000001"]["muted"], true);
    // Other channels and unknown keys survive.
    assert_eq!(v["channels"]["C0000002"]["desktop"], "nothing");
    assert_eq!(v["global"]["global_desktop"], "everything");
    let muted = writes
        .iter()
        .find(|(n, _)| n == "muted_channels")
        .expect("muted_channels written");
    let mut ids: Vec<&str> = muted.1.split(',').collect();
    ids.sort();
    assert_eq!(ids, vec!["C0000001", "C0000002", "C0000003"]);
}

#[tokio::test]
async fn set_conversation_prefs_default_level_removes_desktop() {
    let server = MockServer::start().await;
    mount(&server, "users.prefs.get", fixture("prefs/get.json")).await;
    let writes = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
    let w = writes.clone();
    Mock::given(method("POST"))
        .and(path("/api/users.prefs.set"))
        .and(move |r: &Request| {
            w.lock().unwrap().push((
                field(r, "name").unwrap_or_default(),
                field(r, "value").unwrap_or_default(),
            ));
            true
        })
        .respond_with(json(fixture("prefs/set_ok.json")))
        .mount(&server)
        .await;
    client(&server)
        .prefs_set_conversation("C0000002", None, false)
        .await
        .unwrap();
    let writes = writes.lock().unwrap().clone();
    let all = writes
        .iter()
        .find(|(n, _)| n == "all_notifications_prefs")
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&all.1).unwrap();
    assert!(v["channels"]["C0000002"].get("desktop").is_none());
    assert_eq!(v["channels"]["C0000002"]["muted"], false);
    let muted = writes.iter().find(|(n, _)| n == "muted_channels").unwrap();
    assert_eq!(muted.1, "C0000003");
}

async fn set_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/api/users.prefs.set")
        .count()
}

#[tokio::test]
async fn set_conversation_prefs_refuses_when_stored_prefs_are_unreadable() {
    for (get, why) in [
        ("prefs/get_nulls.json", "null"),
        ("prefs/get_garbage.json", "garbage"),
    ] {
        let server = MockServer::start().await;
        mount(&server, "users.prefs.get", fixture(get)).await;
        mount(&server, "users.prefs.set", fixture("prefs/set_ok.json")).await;
        let r = client(&server)
            .prefs_set_conversation("C0000001", Some("nothing"), false)
            .await;
        assert!(r.is_err(), "{why}: must not overwrite what was not read");
        assert_eq!(set_count(&server).await, 0, "{why}: nothing written");
    }
}

#[tokio::test]
async fn set_conversation_prefs_refuses_when_muted_channels_is_missing() {
    let server = MockServer::start().await;
    mount(
        &server,
        "users.prefs.get",
        r#"{"ok":true,"prefs":{"all_notifications_prefs":"{\"channels\":{}}"}}"#.to_string(),
    )
    .await;
    mount(&server, "users.prefs.set", fixture("prefs/set_ok.json")).await;
    let r = client(&server)
        .prefs_set_conversation("C0000001", Some("nothing"), true)
        .await;
    assert!(r.is_err());
    assert_eq!(set_count(&server).await, 0);
}

#[tokio::test]
async fn set_conversation_prefs_leaves_mobile_untouched() {
    let server = MockServer::start().await;
    mount(&server, "users.prefs.get", fixture("prefs/get.json")).await;
    mount(&server, "users.prefs.set", fixture("prefs/set_ok.json")).await;
    client(&server)
        .prefs_set_conversation("C0000001", Some("nothing"), false)
        .await
        .unwrap();
    let reqs = server.received_requests().await.unwrap();
    let all = reqs
        .iter()
        .filter(|r| r.url.path() == "/api/users.prefs.set")
        .filter_map(|r| field(r, "value"))
        .find(|v| v.contains("channels"))
        .expect("all_notifications_prefs written");
    let v: serde_json::Value = serde_json::from_str(&all).unwrap();
    assert_eq!(v["channels"]["C0000001"]["desktop"], "nothing");
    assert_eq!(v["channels"]["C0000001"]["mobile"], "mentions");
}
