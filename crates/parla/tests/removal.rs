//! Removing one workspace and "Delete all data", against scratch dirs only.

mod support;

use gpui::{AppContext as _, TestAppContext};
use support::*;

fn media(e: &Env, id: &str) -> std::path::PathBuf {
    e.cache.join("media/fake").join(id)
}

#[gpui::test]
async fn remove_wipes_one_workspace_only(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);
    for id in ["w1", "w2"] {
        std::fs::create_dir_all(media(&e, id)).unwrap();
        std::fs::write(media(&e, id).join("a.bin"), b"x").unwrap();
    }
    std::fs::create_dir_all(e.config.join("workspaces/fake")).unwrap();
    let settings_file = e.config.join("workspaces/fake/w1.json");
    std::fs::write(&settings_file, "{}").unwrap();
    let (p1, p2) = (e.sqlite.path(&key("w1")), e.sqlite.path(&key("w2")));
    assert!(p1.exists() && p2.exists());

    let task = w.update(cx, |w, cx| w.remove(&key("w1"), cx));
    task.await.unwrap();

    assert!(!p1.exists(), "database gone");
    assert!(p2.exists(), "other database intact");
    assert!(!media(&e, "w1").exists(), "media cache gone");
    assert!(media(&e, "w2").join("a.bin").exists());
    assert!(settings_file.exists(), "the settings file stays");
    let left: Vec<_> = e.registry.accounts().into_iter().map(|(_, k)| k).collect();
    assert_eq!(left, vec![key("w2"), key("w3")]);
    w.read_with(cx, |w, _| {
        assert_eq!(w.sessions().count(), 2);
        assert!(w.session(&key("w1")).is_none());
        assert_eq!(w.active(), Some(&key("w2")), "another workspace takes over");
    });
}

#[gpui::test]
async fn delete_all_data_keeps_config(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);
    std::fs::create_dir_all(media(&e, "w1")).unwrap();
    std::fs::write(e.data.join("state.json"), "{}").unwrap();
    let task = w.update(cx, |w, cx| w.delete_all(cx));
    task.await.unwrap();

    assert!(!e.data.exists(), "data dir gone");
    assert!(!e.cache.exists(), "cache dir gone");
    assert!(e.config.join("settings.json").exists(), "config stays");
    assert!(e.registry.accounts().is_empty());
    w.read_with(cx, |w, _| {
        assert_eq!(w.sessions().count(), 0);
        assert_eq!(w.active(), None);
    });
}

#[test]
fn unsafe_roots_are_refused() {
    for p in ["/", "", "relative"] {
        assert!(
            parla::removal::remove_dir_all_checked(std::path::Path::new(p)).is_err(),
            "{p}"
        );
    }
    let home = dirs::home_dir().unwrap();
    assert!(parla::removal::remove_dir_all_checked(&home).is_err());
}

#[cfg(unix)]
#[gpui::test]
async fn failed_remove_keeps_the_account_listed(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);
    // A read-only store dir makes deleting the database fail.
    let store_dir = e.sqlite.path(&key("w1")).parent().unwrap().to_path_buf();
    let rw = std::fs::metadata(&store_dir).unwrap().permissions();
    std::fs::set_permissions(&store_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let task = w.update(cx, |w, cx| w.remove(&key("w1"), cx));
    let result = task.await;
    std::fs::set_permissions(&store_dir, rw).unwrap();
    assert!(result.is_err(), "the removal must fail for this test");
    w.read_with(cx, |w, _| {
        assert!(w.accounts().iter().any(|a| a.key == key("w1")));
    });
}

#[cfg(unix)]
#[gpui::test]
async fn failed_delete_all_refreshes_the_account_list(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);
    // Every workspace is removed, then deleting the data dir itself fails.
    let parent = e.data.parent().unwrap().to_path_buf();
    let rw = std::fs::metadata(&parent).unwrap().permissions();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();
    let task = w.update(cx, |w, cx| w.delete_all(cx));
    let result = task.await;
    std::fs::set_permissions(&parent, rw).unwrap();
    assert!(result.is_err(), "the deletion must fail for this test");
    assert!(
        e.registry.account_infos().is_empty(),
        "accounts were removed"
    );
    w.read_with(cx, |w, _| {
        assert!(w.accounts().is_empty(), "the rail lists what still exists");
    });
}

#[gpui::test]
async fn removing_a_workspace_while_it_starts_keeps_it_removed(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let (registry, data, cache) = (e.registry.clone(), e.data.clone(), e.cache.clone());
    let w = cx.new(|cx| {
        let store: std::sync::Arc<dyn store::Store> =
            std::sync::Arc::new(store::MemoryStore::default());
        parla::workspaces::Workspaces::new(&registry, store, cx).with_dirs(data, cache)
    });
    w.update(cx, |w, cx| w.start(key("w1"), cx));
    let task = w.update(cx, |w, cx| w.remove(&key("w1"), cx));
    task.await.unwrap();
    settle(cx);
    w.read_with(cx, |w, _| {
        assert!(w.session(&key("w1")).is_none(), "late start dropped");
    });
}
