//! Launch at login: the setting drives a login item through a seam, once.

#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::rc::Rc;

use gpui::TestAppContext;
use parla::instance::LaunchMode;
#[cfg(target_os = "macos")]
use parla::login_item::login_exe;
use parla::login_item::{FakeLoginItem, LoginItems, login_args};
use parla::{handle_change, init_without_watcher};
use settings::{ChangedFile, ConfigPaths};

struct Env {
    _dir: tempfile::TempDir,
    paths: ConfigPaths,
}

fn setup(cx: &mut TestAppContext, text: &str, initial: bool) -> (Env, Rc<FakeLoginItem>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    std::fs::write(&paths.settings, text).unwrap();
    let p = paths.clone();
    cx.update(|cx| init_without_watcher(cx, p));
    let item = Rc::new(FakeLoginItem::new(initial));
    let installed = item.clone();
    cx.update(|cx| LoginItems::install(cx, installed));
    (Env { _dir: dir, paths }, item)
}

fn rewrite(cx: &mut TestAppContext, env: &Env, text: &str) {
    std::fs::write(&env.paths.settings, text).unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Settings));
    cx.run_until_parked();
}

#[gpui::test]
async fn setting_toggles_login_item_once(cx: &mut TestAppContext) {
    let (env, item) = setup(cx, r#"{ "launch_at_login": false }"#, false);
    // Start-up matched, so nothing was written.
    assert!(item.sets().is_empty());

    rewrite(cx, &env, r#"{ "launch_at_login": true }"#);
    assert_eq!(item.sets(), vec![true]);

    // An unrelated settings edit re-applies the same state: still one write.
    rewrite(
        cx,
        &env,
        r#"{ "launch_at_login": true, "density": "cozy" }"#,
    );
    assert_eq!(item.sets(), vec![true]);
}

#[gpui::test]
async fn login_item_not_touched_when_state_matches(cx: &mut TestAppContext) {
    let (env, item) = setup(cx, r#"{ "launch_at_login": true }"#, true);
    assert!(item.sets().is_empty());
    assert!(item.gets() > 0);

    // The item was already on: no write, even though the setting was read.
    rewrite(
        cx,
        &env,
        r#"{ "launch_at_login": true, "density": "cozy" }"#,
    );
    assert!(item.sets().is_empty());

    rewrite(cx, &env, r#"{ "launch_at_login": false }"#);
    assert_eq!(item.sets(), vec![false]);
}

#[gpui::test]
async fn unreadable_registered_path_still_enables_the_item(cx: &mut TestAppContext) {
    let (env, item) = setup(cx, r#"{ "launch_at_login": false }"#, false);
    item.fail_next_registered_path();
    // The stale-path check cannot read the entry: the setting is applied
    // anyway (as with no entry), and the error is only logged.
    rewrite(cx, &env, r#"{ "launch_at_login": true }"#);
    assert_eq!(item.sets(), vec![true]);
}

#[gpui::test]
async fn login_item_error_is_not_fatal(cx: &mut TestAppContext) {
    let (env, item) = setup(cx, r#"{ "launch_at_login": false }"#, false);
    item.fail_next_set();
    // The failed write is logged, not returned to the settings watcher.
    rewrite(cx, &env, r#"{ "launch_at_login": true }"#);
    assert!(item.sets().is_empty());

    // Matching the old state writes nothing; a later toggle tries again.
    rewrite(cx, &env, r#"{ "launch_at_login": false }"#);
    rewrite(
        cx,
        &env,
        r#"{ "launch_at_login": true, "density": "cozy" }"#,
    );
    assert_eq!(item.sets(), vec![true]);
}

#[test]
fn login_launch_starts_hidden() {
    let args = login_args();
    assert!(args.contains(&"--hidden"), "{args:?}");
    assert_eq!(
        LaunchMode::from_args(std::iter::once("parla").chain(args.iter().copied())),
        LaunchMode::Hidden
    );
}

#[cfg(target_os = "macos")]
#[test]
fn login_exe_requires_a_bundle_executable() {
    let bundled = PathBuf::from("/Applications/Parla.app/Contents/MacOS/parla");
    assert_eq!(login_exe(&bundled), Some(bundled.clone()));

    // A bare binary has no bundle to register, and a non-executable path
    // inside a bundle is not the Launch Agent's program either.
    assert_eq!(login_exe(&PathBuf::from("/usr/local/bin/parla")), None);
    assert_eq!(
        login_exe(&PathBuf::from(
            "/Applications/Parla.app/Contents/Resources/parla"
        )),
        None
    );
}

#[gpui::test]
async fn login_item_stale_path_is_rewritten_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    std::fs::write(&paths.settings, r#"{ "launch_at_login": true }"#).unwrap();
    let p = paths.clone();
    cx.update(|cx| init_without_watcher(cx, p));
    let item = Rc::new(FakeLoginItem::new(true));
    item.set_registered_path(Some(std::path::PathBuf::from(
        "/stale/Parla.app/Contents/MacOS/parla",
    )));
    let installed = item.clone();
    cx.update(|cx| LoginItems::install(cx, installed));
    // install applied: stale -> enable (overwrites the plist)
    assert_eq!(item.sets(), vec![true]);

    // unrelated settings edit re-applies, but path now matches: no write
    std::fs::write(
        &paths.settings,
        r#"{ "launch_at_login": true, "density": "cozy" }"#,
    )
    .unwrap();
    cx.update(|cx| handle_change(cx, ChangedFile::Settings));
    cx.run_until_parked();
    assert_eq!(item.sets(), vec![true]);
}

#[gpui::test]
async fn login_item_matching_path_writes_nothing(cx: &mut TestAppContext) {
    let (env, item) = setup(cx, r#"{ "launch_at_login": true }"#, true);
    item.set_registered_path(None);
    rewrite(
        cx,
        &env,
        r#"{ "launch_at_login": true, "density": "cozy" }"#,
    );
    assert!(item.sets().is_empty());
}

#[gpui::test]
async fn login_item_setting_off_never_rewrites(cx: &mut TestAppContext) {
    let (env, item) = setup(cx, r#"{ "launch_at_login": false }"#, true);
    item.set_registered_path(Some(std::path::PathBuf::from(
        "/stale/Parla.app/Contents/MacOS/parla",
    )));
    rewrite(
        cx,
        &env,
        r#"{ "launch_at_login": false, "density": "cozy" }"#,
    );
    assert_eq!(item.sets(), vec![false]);
}
