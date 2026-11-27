//! The Update and Restart flow of the update notice: consent, progress,
//! failure states and the buttons each state shows.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::{Context, IntoElement, Render, TestAppContext, Window, div, prelude::*};
use parla::toast::ConfigBanner;
use parla::updates::download::Downloader;
use parla::updates::install::InstallError;
use parla::updates::package::{Asset, UpdatePackage, trusted_comment};
use parla::updates::{
    SelfUpdate, UpdateFailure, UpdateManagerHandle, UpdatePhase, UpdateStatus, install_with,
};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

fn sign(k: &Keys) -> String {
    minisign::sign(
        Some(&k.pkey),
        &k.sk,
        Cursor::new(BODY),
        Some(&trusted_comment(&version(), DMG)),
        None,
    )
    .unwrap()
    .into_string()
}

fn package(server: &MockServer) -> UpdatePackage {
    let asset = |name: &str, size| Asset {
        name: name.to_string(),
        url: Url::parse(&format!("{}/{name}", server.uri())).unwrap(),
        size,
    };
    UpdatePackage {
        version: version(),
        tag: "v0.6.0".to_string(),
        dmg: asset(DMG, BODY.len() as u64),
        signature: asset(SIG, 0),
    }
}

async fn mount(server: &MockServer, signature: &str, expect: Option<u64>) {
    for (file, body) in [(SIG, signature.as_bytes()), (DMG, BODY)] {
        let mock = Mock::given(method("GET"))
            .and(path(format!("/{file}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body));
        let mock = match expect {
            Some(n) => mock.expect(n),
            None => mock,
        };
        mock.mount(server).await;
    }
}

/// What the fake install step saw.
#[derive(Default)]
struct Installs {
    dmgs: Vec<(PathBuf, bool)>,
}

struct Fixture {
    _dir: tempfile::TempDir,
    installs: Arc<Mutex<Installs>>,
    bundle: PathBuf,
}

/// A manager with the update check off, ready to self-update from `server`.
fn setup(
    cx: &mut TestAppContext,
    k: &Keys,
    install_result: Result<(), InstallError>,
    can_update: bool,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("settings.json"),
        r#"{"updates":{"check":false}}"#,
    )
    .unwrap();
    let paths = settings::ConfigPaths::at(&config);
    let state = dir.path().join("updates.json");
    let cache = dir.path().join("cache");
    let bundle = dir.path().join("Parla.app");
    let installs = Arc::new(Mutex::new(Installs::default()));
    let seen = installs.clone();
    let su = SelfUpdate {
        downloader: Downloader::new_allow_http(cache, &current()),
        public_key: k.pk.clone(),
        bundle: bundle.clone(),
        install: Box::new(move |dmg| {
            seen.lock()
                .unwrap()
                .dmgs
                .push((dmg.to_path_buf(), dmg.exists()));
            install_result.clone()
        }),
    };
    cx.executor().allow_parking();
    cx.update(|cx| {
        parla::runtime::init_runtime(cx);
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
        cx.set_global(ConfigBanner::default());
        install_with(
            cx,
            Url::parse("http://127.0.0.1:9/releases/latest").unwrap(),
            state,
        );
        if can_update {
            parla::updates::set_self_update(cx, su);
        }
    });
    Fixture {
        _dir: dir,
        installs,
        bundle,
    }
}

fn offer(cx: &mut TestAppContext, package: Option<UpdatePackage>) {
    cx.update(|cx| {
        let h = cx.global::<UpdateManagerHandle>().0.clone();
        h.update(cx, |m, cx| {
            m.apply_status(
                UpdateStatus::Available {
                    version: version(),
                    url: Url::parse("https://github.com/oscarbc96/parla/releases/tag/v0.6.0")
                        .unwrap(),
                    package,
                },
                true,
                cx,
            )
        });
    });
}

fn notice(cx: &mut TestAppContext) -> parla::updates::UpdateNotice {
    cx.update(|cx| {
        cx.global::<ConfigBanner>()
            .update
            .clone()
            .expect("a notice")
    })
}

/// Lets the download and install tasks run until the notice reaches `done`.
async fn settle(cx: &mut TestAppContext, done: impl Fn(&UpdatePhase) -> bool) {
    for _ in 0..200 {
        cx.run_until_parked();
        if cx.update(|cx| {
            cx.global::<ConfigBanner>()
                .update
                .as_ref()
                .is_some_and(|n| done(&n.phase))
        }) {
            return;
        }
        cx.executor()
            .timer(std::time::Duration::from_millis(25))
            .await;
    }
    panic!("the update flow did not settle: {:?}", notice(cx).phase);
}

fn click_install(cx: &mut TestAppContext) {
    cx.update(parla::updates::start_install);
}

#[gpui::test]
async fn offer_without_self_update_keeps_the_download_link_only(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let k = keys();
    let _f = setup(cx, &k, Ok(()), false);
    offer(cx, Some(package(&server)));
    let n = notice(cx);
    assert_eq!(n.phase, UpdatePhase::Offer);
    assert_eq!(n.install_label, None);
    assert_eq!(n.restart_label, None);
    assert_eq!(n.text, "Parla 0.6.0 is available");
    // A click on a notice without Update does nothing.
    click_install(cx);
    assert_eq!(notice(cx).phase, UpdatePhase::Offer);
}

#[gpui::test]
async fn offer_without_a_package_keeps_the_download_link_only(cx: &mut TestAppContext) {
    let k = keys();
    let _f = setup(cx, &k, Ok(()), true);
    offer(cx, None);
    assert_eq!(notice(cx).install_label, None);
}

#[gpui::test]
async fn offer_with_a_package_and_self_update_shows_update(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    mount(&server, "unused", Some(0)).await;
    let k = keys();
    let _f = setup(cx, &k, Ok(()), true);
    offer(cx, Some(package(&server)));
    cx.run_until_parked();
    let n = notice(cx);
    assert_eq!(n.install_label.as_deref(), Some("Update"));
    assert_eq!(n.phase, UpdatePhase::Offer);
    // Nothing downloads before the click.
    server.verify().await;
}

#[gpui::test]
async fn update_downloads_installs_and_waits_for_the_restart(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let k = keys();
    mount(&server, &sign(&k), Some(1)).await;
    let f = setup(cx, &k, Ok(()), true);
    offer(cx, Some(package(&server)));
    let restart = cx.expect_restart();

    click_install(cx);
    settle(cx, |p| matches!(p, UpdatePhase::Ready)).await;

    let n = notice(cx);
    assert_eq!(n.text, "Parla 0.6.0 is ready. Restart to update.");
    assert_eq!(n.restart_label.as_deref(), Some("Restart to update"));
    assert_eq!(n.install_label, None);
    {
        let installs = f.installs.lock().unwrap();
        assert_eq!(installs.dmgs.len(), 1);
        assert!(
            installs.dmgs[0].1,
            "the verified dmg exists when installing"
        );
    }
    server.verify().await;

    cx.update(parla::updates::restart_to_update);
    let (restart_path, _) = restart.await.unwrap();
    assert_eq!(restart_path, Some(f.bundle.clone()));
}

#[gpui::test]
async fn restart_does_nothing_before_the_update_is_in_place(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let k = keys();
    let _f = setup(cx, &k, Ok(()), true);
    offer(cx, Some(package(&server)));
    let mut restart = cx.expect_restart();
    cx.update(parla::updates::restart_to_update);
    assert!(restart.try_recv().unwrap().is_none());
}

#[gpui::test]
async fn a_bad_signature_is_refused_and_can_be_retried(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let other = keys();
    mount(&server, &sign(&other), None).await;
    let k = keys();
    let f = setup(cx, &k, Ok(()), true);
    offer(cx, Some(package(&server)));

    click_install(cx);
    settle(cx, |p| matches!(p, UpdatePhase::Failed(_))).await;

    let n = notice(cx);
    assert_eq!(n.phase, UpdatePhase::Failed(UpdateFailure::Signature));
    assert!(n.text.contains("signature"), "{}", n.text);
    assert_eq!(n.install_label.as_deref(), Some("Try again"));
    assert!(f.installs.lock().unwrap().dmgs.is_empty());
}

#[gpui::test]
async fn a_failed_download_can_be_retried(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let k = keys();
    // No mocks: every request is a 404.
    let f = setup(cx, &k, Ok(()), true);
    offer(cx, Some(package(&server)));

    click_install(cx);
    settle(cx, |p| matches!(p, UpdatePhase::Failed(_))).await;
    assert_eq!(
        notice(cx).phase,
        UpdatePhase::Failed(UpdateFailure::Download)
    );
    assert_eq!(notice(cx).text, "Could not download Parla 0.6.0");

    mount(&server, &sign(&k), None).await;
    click_install(cx);
    settle(cx, |p| matches!(p, UpdatePhase::Ready)).await;
    assert_eq!(f.installs.lock().unwrap().dmgs.len(), 1);
}

#[gpui::test]
async fn an_install_failure_leaves_the_download_link(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let k = keys();
    mount(&server, &sign(&k), None).await;
    let _f = setup(cx, &k, Err(InstallError::Copy), true);
    offer(cx, Some(package(&server)));

    click_install(cx);
    settle(cx, |p| matches!(p, UpdatePhase::Failed(_))).await;

    let n = notice(cx);
    assert_eq!(n.phase, UpdatePhase::Failed(UpdateFailure::Install));
    assert_eq!(n.text, "Could not install Parla 0.6.0");
    assert_eq!(
        n.url,
        "https://github.com/oscarbc96/parla/releases/tag/v0.6.0"
    );
}

#[gpui::test]
async fn a_check_while_ready_does_not_replace_the_notice(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let k = keys();
    mount(&server, &sign(&k), None).await;
    let _f = setup(cx, &k, Ok(()), true);
    offer(cx, Some(package(&server)));
    click_install(cx);
    settle(cx, |p| matches!(p, UpdatePhase::Ready)).await;

    cx.update(|cx| {
        let h = cx.global::<UpdateManagerHandle>().0.clone();
        h.update(cx, |m, cx| {
            m.apply_status(UpdateStatus::UpToDate, true, cx);
            m.apply_status(
                UpdateStatus::Available {
                    version: semver::Version::parse("0.7.0").unwrap(),
                    url: Url::parse("https://github.com/oscarbc96/parla/releases/tag/v0.7.0")
                        .unwrap(),
                    package: None,
                },
                true,
                cx,
            );
        });
    });

    let n = notice(cx);
    assert_eq!(n.phase, UpdatePhase::Ready);
    assert_eq!(n.text, "Parla 0.6.0 is ready. Restart to update.");
    cx.update(|cx| assert!(cx.global::<ConfigBanner>().notices.is_empty()));
}

/// Draws the strip for the current notice.
struct Host;

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notice = cx.global::<ConfigBanner>().update.clone();
        div().children(
            notice.map(|n| parla::update_banner::update_banner(cx, &n).into_any_element()),
        )
    }
}

fn buttons(cx: &mut gpui::VisualTestContext) -> Vec<&'static str> {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    [
        "update-install",
        "update-restart",
        "update-download",
        "update-skip",
    ]
    .into_iter()
    .filter(|id| cx.debug_bounds(id).is_some())
    .collect()
}

#[gpui::test]
async fn buttons_follow_the_phase(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let k = keys();
    mount(&server, &sign(&k), None).await;
    let _f = setup(cx, &k, Ok(()), true);
    let mut app = cx.clone();
    let (_host, vcx) = cx.add_window_view(|_, _| Host);

    offer(&mut app, Some(package(&server)));
    assert_eq!(
        buttons(vcx),
        ["update-install", "update-download", "update-skip"]
    );

    click_install(&mut app);
    settle(&mut app, |p| matches!(p, UpdatePhase::Ready)).await;
    assert_eq!(buttons(vcx), ["update-restart"]);
}

#[gpui::test]
async fn without_self_update_the_strip_has_download_and_skip_only(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let k = keys();
    let _f = setup(cx, &k, Ok(()), false);
    let mut app = cx.clone();
    let (_host, vcx) = cx.add_window_view(|_, _| Host);
    offer(&mut app, Some(package(&server)));
    assert_eq!(buttons(vcx), ["update-download", "update-skip"]);
}

#[gpui::test]
async fn clicking_update_in_the_strip_starts_the_download(cx: &mut TestAppContext) {
    let server = MockServer::start().await;
    let k = keys();
    mount(&server, &sign(&k), Some(1)).await;
    let _f = setup(cx, &k, Ok(()), true);
    let mut app = cx.clone();
    let (_host, vcx) = cx.add_window_view(|_, _| Host);
    offer(&mut app, Some(package(&server)));
    buttons(vcx);

    let b = vcx.debug_bounds("update-install").unwrap();
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    settle(&mut app, |p| matches!(p, UpdatePhase::Ready)).await;
    server.verify().await;

    assert_eq!(buttons(vcx), ["update-restart"]);
    let restart = app.expect_restart();
    let b = vcx.debug_bounds("update-restart").unwrap();
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    assert!(restart.await.is_ok());
}
