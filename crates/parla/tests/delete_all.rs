//! "Delete all data" leaves only the config directory.

mod support;

use gpui::TestAppContext;
use store_sqlite::DbId;
use support::*;

#[gpui::test]
async fn delete_all_leaves_only_config(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);

    // Every file kind Parla writes under data_dir and cache_dir.
    std::fs::create_dir_all(e.data.join("crashes")).unwrap();
    std::fs::write(e.data.join("crashes/report.txt"), b"boom").unwrap();
    std::fs::write(e.data.join("parla.log"), b"line\n").unwrap();
    std::fs::write(e.data.join("parla.log.1"), b"old\n").unwrap();
    std::fs::write(e.data.join("state.json"), b"{}").unwrap();
    std::fs::write(e.data.join("updates.json"), b"{}").unwrap();
    std::fs::write(e.data.join("windows.json"), b"{}").unwrap();
    std::fs::write(e.data.join("development_credentials.json"), b"{}").unwrap();
    e.sqlite.open_app().await.unwrap();
    assert!(e.sqlite.app_path().exists());

    std::fs::create_dir_all(e.cache.join("media/fake/w1")).unwrap();
    std::fs::write(e.cache.join("media/fake/w1/a.bin"), b"x").unwrap();
    std::fs::create_dir_all(e.cache.join("call-window/website-data")).unwrap();
    std::fs::write(e.cache.join("call-window/website-data/blob"), b"x").unwrap();

    for id in ["w1", "w2", "w3"] {
        assert!(e.keys.contains(&DbId::Workspace(key(id))));
    }
    assert!(e.keys.contains(&DbId::App));

    let task = w.update(cx, |w, cx| w.delete_all(cx));
    task.await.unwrap();

    assert!(!e.data.exists(), "data dir gone");
    assert!(!e.cache.exists(), "cache dir gone");
    assert!(e.config.join("settings.json").exists(), "config stays");
    assert!(e.keys.is_empty(), "no keychain item remains");
    assert!(e.registry.accounts().is_empty());
    w.read_with(cx, |w, _| {
        assert_eq!(w.sessions().count(), 0);
        assert_eq!(w.active(), None);
    });
}
