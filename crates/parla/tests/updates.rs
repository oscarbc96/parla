use std::sync::Arc;

use jiff::Timestamp;
use parla::updates::{RELEASES_PAGE_PREFIX, UpdateChecker, UpdateStatus, should_show_available};
use url::Url;
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path},
};

fn current() -> semver::Version {
    semver::Version::parse("0.5.0").unwrap()
}

fn checker(base: &str, state: &std::path::Path) -> UpdateChecker {
    UpdateChecker::new(Url::parse(base).unwrap(), current(), state.to_path_buf())
}

fn release(tag: &str, html: &str, draft: bool, prerelease: bool) -> serde_json::Value {
    serde_json::json!({
        "tag_name": tag,
        "html_url": html,
        "draft": draft,
        "prerelease": prerelease,
    })
}

#[gpui::test]
async fn no_request_when_check_off(cx: &mut gpui::TestAppContext) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release(
            "v0.6.0",
            &format!("{RELEASES_PAGE_PREFIX}tag/v0.6.0"),
            false,
            false,
        )))
        .expect(0)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("settings.json"),
        r#"{"updates":{"check":false}}"#,
    )
    .unwrap();
    // Due state: no updates.json written.
    let state = dir.path().join("updates.json");
    let paths = settings::ConfigPaths::at(&config);
    let base = Url::parse(&format!("{}/releases/latest", server.uri())).unwrap();

    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        cx.set_global(parla::toast::ConfigBanner::default());
        parla::updates::install_with(cx, base, state);
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let h = cx.global::<parla::updates::UpdateManagerHandle>().0.clone();
        assert!(
            !h.read(cx).has_timer(),
            "manager must not arm a timer when updates.check is off"
        );
    });
    server.verify().await;
}

#[tokio::test]
async fn at_most_once_per_day_across_restarts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release(
            "v0.5.0",
            &format!("{RELEASES_PAGE_PREFIX}tag/v0.5.0"),
            false,
            false,
        )))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("updates.json");
    let base = format!("{}/releases/latest", server.uri());
    let now = Timestamp::now();
    let c1 = checker(&base, &state);
    assert!(c1.due(now));
    let _ = c1.check(now).await;
    let c2 = checker(&base, &state);
    assert!(!c2.due(now));
    server.verify().await;
}

#[tokio::test]
async fn newer_tag_offers_download() {
    let server = MockServer::start().await;
    let url = format!("{RELEASES_PAGE_PREFIX}tag/v0.6.0");
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(release("v0.6.0", &url, false, false)),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let c = checker(&server.uri(), &dir.path().join("updates.json"));
    match c.check(Timestamp::now()).await {
        UpdateStatus::Available { version, url, .. } => {
            assert_eq!(version, semver::Version::parse("0.6.0").unwrap());
            assert_eq!(
                url.as_str(),
                "https://github.com/oscarbc96/parla/releases/tag/v0.6.0"
            );
        }
        other => panic!("expected available, got {other:?}"),
    }
}

#[tokio::test]
async fn same_or_older_is_up_to_date() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    for tag in ["v0.5.0", "v0.4.9"] {
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(release(
                tag,
                &format!("{RELEASES_PAGE_PREFIX}tag/{tag}"),
                false,
                false,
            )))
            .mount(&server)
            .await;
        let c = checker(&server.uri(), &dir.path().join(format!("u-{tag}.json")));
        assert_eq!(
            c.check(Timestamp::now()).await,
            UpdateStatus::UpToDate,
            "{tag}"
        );
        server.reset().await;
    }
}

#[tokio::test]
async fn prerelease_and_draft_ignored() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    for (draft, pre) in [(true, false), (false, true)] {
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(release(
                "v0.6.0",
                &format!("{RELEASES_PAGE_PREFIX}tag/v0.6.0"),
                draft,
                pre,
            )))
            .mount(&server)
            .await;
        let c = checker(
            &server.uri(),
            &dir.path().join(format!("u-{draft}-{pre}.json")),
        );
        assert_eq!(c.check(Timestamp::now()).await, UpdateStatus::UpToDate);
        server.reset().await;
    }
}

#[tokio::test]
async fn foreign_release_url_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release(
            "v0.6.0",
            "https://example.com/releases/tag/v0.6.0",
            false,
            false,
        )))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let c = checker(&server.uri(), &dir.path().join("updates.json"));
    assert_eq!(c.check(Timestamp::now()).await, UpdateStatus::UpToDate);
}

#[tokio::test]
async fn malformed_tag_is_up_to_date() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release(
            "nightly",
            &format!("{RELEASES_PAGE_PREFIX}tag/nightly"),
            false,
            false,
        )))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let c = checker(&server.uri(), &dir.path().join("updates.json"));
    assert_eq!(c.check(Timestamp::now()).await, UpdateStatus::UpToDate);
}

#[tokio::test]
async fn request_carries_only_accept_and_user_agent() {
    struct Capture(Arc<std::sync::Mutex<Vec<(String, String)>>>);
    impl Respond for Capture {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let mut v = self.0.lock().unwrap();
            v.clear();
            for (k, val) in request.headers.iter() {
                v.push((
                    k.as_str().to_ascii_lowercase(),
                    val.to_str().unwrap_or("").to_string(),
                ));
            }
            ResponseTemplate::new(200).set_body_json(release(
                "v0.5.0",
                &format!("{RELEASES_PAGE_PREFIX}tag/v0.5.0"),
                false,
                false,
            ))
        }
    }
    let server = MockServer::start().await;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    Mock::given(method("GET"))
        .respond_with(Capture(seen.clone()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let c = checker(&server.uri(), &dir.path().join("updates.json"));
    let _ = c.check(Timestamp::now()).await;
    let headers = seen.lock().unwrap().clone();
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "accept" && v == "application/vnd.github+json")
    );
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "user-agent" && v == "Parla/0.5.0")
    );
    assert!(!headers.iter().any(|(k, _)| k == "cookie"));
}

#[tokio::test]
async fn failure_waits_a_day() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("updates.json");
    let c = checker(&server.uri(), &state);
    assert_eq!(c.check(Timestamp::now()).await, UpdateStatus::Failed);
    assert!(!c.due(Timestamp::now()));
    server.verify().await;
}

#[gpui::test]
async fn skip_version_writes_the_setting_and_hides_the_version(cx: &mut gpui::TestAppContext) {
    // The skip is stored in the settings file, and only a newer version shows.
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("settings.json"), "{}").unwrap();
    let paths = settings::ConfigPaths::at(&config);
    let settings_path = paths.settings.clone();
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        cx.set_global(parla::toast::ConfigBanner::default());
        parla::updates::skip_version("v0.6.0", cx);
    });
    let text = std::fs::read_to_string(&settings_path).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["updates"]["skip"], "v0.6.0", "{text}");

    let available = semver::Version::parse("0.6.0").unwrap();
    assert!(!should_show_available(&available, Some("v0.6.0")));
    assert!(!should_show_available(&available, Some("0.6.0")));
    let newer = semver::Version::parse("0.6.1").unwrap();
    assert!(should_show_available(&newer, Some("v0.6.0")));
}

#[gpui::test]
async fn idle_has_one_one_shot_timer_at_most(cx: &mut gpui::TestAppContext) {
    // The update scheduler uses one one-shot timer to the next due time and
    // re-arms it only after a check.
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("updates.json");
    // Pretend an attempt happened just now so the manager must wait, not check.
    std::fs::write(
        &state,
        serde_json::json!({ "last_attempt": jiff::Timestamp::now().as_second() }).to_string(),
    )
    .unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("settings.json"), "{}").unwrap();
    let paths = settings::ConfigPaths::at(&config);
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        cx.set_global(parla::toast::ConfigBanner::default());
        parla::updates::install_with(
            cx,
            Url::parse("http://127.0.0.1:9/releases/latest").unwrap(),
            state,
        );
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let h = cx.global::<parla::updates::UpdateManagerHandle>().0.clone();
        let has = h.read(cx).has_timer();
        assert!(has, "manager should arm one timer when not due");
    });
}

#[gpui::test]
async fn unwritable_state_does_not_loop(cx: &mut gpui::TestAppContext) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release(
            "v0.5.0",
            &format!("{RELEASES_PAGE_PREFIX}tag/v0.5.0"),
            false,
            false,
        )))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("settings.json"), "{}").unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let state = blocker.join("updates.json");
    let paths = settings::ConfigPaths::at(&config);
    let base = Url::parse(&format!("{}/releases/latest", server.uri())).unwrap();

    cx.executor().allow_parking();
    cx.update(|cx| {
        parla::runtime::init_runtime(cx);
        parla::init_without_watcher(cx, paths);
        cx.set_global(parla::toast::ConfigBanner::default());
        parla::updates::install_with(cx, base, state);
    });
    cx.run_until_parked();
    cx.executor()
        .timer(std::time::Duration::from_millis(250))
        .await;
    cx.run_until_parked();

    server.verify().await;
}

async fn manual_check_notice(
    cx: &mut gpui::TestAppContext,
    status: u16,
    body: serde_json::Value,
) -> Vec<String> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("settings.json"),
        r#"{"updates":{"check":false}}"#,
    )
    .unwrap();
    let paths = settings::ConfigPaths::at(&config);
    let base = Url::parse(&format!("{}/releases/latest", server.uri())).unwrap();
    let state = dir.path().join("updates.json");
    cx.executor().allow_parking();
    cx.update(|cx| {
        parla::runtime::init_runtime(cx);
        parla::init_without_watcher(cx, paths);
        cx.set_global(parla::toast::ConfigBanner::default());
        parla::updates::install_with(cx, base, state);
        cx.dispatch_action(&parla::actions::CheckForUpdates);
    });
    for _ in 0..40 {
        cx.run_until_parked();
        cx.executor()
            .timer(std::time::Duration::from_millis(25))
            .await;
    }
    let out = cx.update(|cx| {
        cx.global::<parla::toast::ConfigBanner>()
            .notices
            .iter()
            .map(|(_, t)| t.clone())
            .collect()
    });
    drop(server);
    out
}

#[gpui::test]
async fn manual_check_says_when_up_to_date(cx: &mut gpui::TestAppContext) {
    let notes = manual_check_notice(
        cx,
        200,
        release(
            "v0.0.0",
            &format!("{RELEASES_PAGE_PREFIX}tag/v0.0.0"),
            false,
            false,
        ),
    )
    .await;
    assert_eq!(notes.len(), 1, "{notes:?}");
}

#[gpui::test]
async fn manual_check_says_when_it_failed(cx: &mut gpui::TestAppContext) {
    let notes = manual_check_notice(cx, 500, serde_json::json!({})).await;
    assert_eq!(notes.len(), 1, "{notes:?}");
}
