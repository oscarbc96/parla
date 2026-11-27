mod support;

use std::sync::Arc;

use domain::ChatProvider as _;
use domain::test_support::cid;
use gpui::{AppContext as _, Bounds, Focusable as _, TestAppContext, point, px, size};
use parla::root_view::{Phase, RootView};
use parla::window_state::{self, PopOutTarget, SavedWindow};
use parla::workspaces::{Workspaces, WorkspacesHandle};
use settings::ConfigPaths;
use store::{MemoryStore, Store};
use support::*;

#[test]
fn window_state_round_trips_and_tolerates_garbage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("windows.json");
    let ws = settings::WorkspaceKey::new("fake", "w1");
    let saved = vec![
        SavedWindow {
            target: PopOutTarget::conversation(ws.clone(), cid("D1")),
            bounds: Some(Bounds {
                origin: point(px(10.0), px(20.0)),
                size: size(px(800.0), px(600.0)),
            }),
        },
        SavedWindow {
            target: PopOutTarget::thread(ws, cid("D1"), domain::MessageTs("1.000000".into())),
            bounds: None,
        },
    ];
    window_state::save(&path, &saved).unwrap();
    let loaded = window_state::load(&path);
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].target, saved[0].target);
    assert!(loaded[0].bounds.is_some());
    assert_eq!(loaded[1].target, saved[1].target);

    std::fs::write(&path, b"not json {{{").unwrap();
    assert!(window_state::load(&path).is_empty(), "corrupt -> empty");

    std::fs::remove_file(&path).unwrap();
    assert!(window_state::load(&path).is_empty(), "missing -> empty");
}

fn init(cx: &mut TestAppContext, dir: &tempfile::TempDir) {
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    let paths = ConfigPaths::at(&config);
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
}

#[gpui::test]
async fn popout_shares_state_with_main(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);
    view.read_with(cxv, |v, _| assert_eq!(*v.phase(), Phase::Ready));

    let session = view.read_with(cxv, |v, _| v.session().unwrap().clone());
    e.providers[0].push_event(domain::ProviderEvent::Connected { reconnect: false });
    session.update(cxv, |s, cx| s.open(cid("D1"), cx));
    settle(cxv);

    let ws = e.providers[0].workspace().clone();
    let target = PopOutTarget::conversation(ws, cid("D1"));
    let handle = cxv
        .update(|_window, cx| parla::popout::open(target, cx))
        .expect("popout opens");
    settle(cxv);

    // Same session entity as main window.
    let pop_session =
        cxv.update(|_window, cx| handle.update(cx, |v, _, _| v.session().clone()).unwrap());
    assert_eq!(
        view.read_with(cxv, |v, _| v.session().cloned()),
        Some(pop_session)
    );

    // Send in the pop-out -> visible in main, one provider call.
    let before = e.providers[0].calls().len();
    cxv.update(|_window, cx| {
        handle
            .update(cx, |v, _window, cx| {
                if let Some(composer) = v.composer() {
                    composer.update(cx, |c, cx| c.set_text("hello".into(), cx));
                }
                v.send(cx);
            })
            .unwrap();
    });
    settle(cxv);

    session.read_with(cxv, |s, _| {
        let msgs = s.state().windows[&cid("D1")].messages();
        assert!(
            msgs.iter().any(|m| m.text.as_ref() == "hello"),
            "message visible in main session"
        );
    });
    let sends = e.providers[0].calls()[before..]
        .iter()
        .filter(|c| matches!(c, domain::test_support::Call::Send { .. }))
        .count();
    assert_eq!(sends, 1, "one provider call");
}

#[gpui::test]
async fn second_popout_of_same_target_focuses_existing(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let ws = e.providers[0].workspace().clone();
    let target = PopOutTarget::conversation(ws, cid("D1"));
    let a = cxv
        .update(|_window, cx| parla::popout::open(target.clone(), cx))
        .unwrap();
    settle(cxv);
    let b = cxv
        .update(|_window, cx| parla::popout::open(target, cx))
        .unwrap();
    assert_eq!(a, b, "same window handle");
}

#[gpui::test]
async fn closing_popout_releases_window(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: data.clone(),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let ws = e.providers[0].workspace().clone();
    let target = PopOutTarget::conversation(ws.clone(), cid("D1"));
    let path = window_state::path_in(&data);
    let handle = cxv
        .update(|_window, cx| parla::popout::open(target.clone(), cx))
        .unwrap();
    settle(cxv);
    assert_eq!(window_state::load(&path).len(), 1, "saved while open");
    w.read_with(cxv, |w, _| assert_eq!(w.popout_count(&ws), 1));

    cxv.update(|_window, cx| {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    settle(cxv);

    assert_eq!(
        window_state::load(&path).len(),
        0,
        "windows.json cleared on close"
    );
    w.read_with(cxv, |w, _| assert_eq!(w.popout_count(&ws), 0));

    let again = cxv
        .update(|_window, cx| parla::popout::open(target, cx))
        .unwrap();
    assert_ne!(handle, again, "closed window is released");
}

#[gpui::test]
async fn popout_keeps_workspace_out_of_lean(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let k1 = e.providers[0].workspace().clone();
    let k2 = e.providers[1].workspace().clone();
    w.update(cxv, |w, cx| w.activate(&k2, cx));
    settle(cxv);

    let s1 = w.read_with(cxv, |w, _| w.session(&k1).unwrap());
    s1.update(cxv, |s, cx| s.set_lean(false, cx));
    let target = PopOutTarget::conversation(k1.clone(), cid("D1"));
    let handle = cxv
        .update(|_window, cx| parla::popout::open(target.clone(), cx))
        .unwrap();
    settle(cxv);

    w.read_with(cxv, |w, _| assert_eq!(w.popout_count(&k1), 1));
    s1.read_with(cxv, |s, _| {
        assert!(!s.is_lean(), "pop-out keeps workspace out of lean")
    });

    // Real close must release the pop-out count.
    cxv.update(|_window, cx| {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    settle(cxv);
    w.read_with(cxv, |w, _| assert_eq!(w.popout_count(&k1), 0));
}

#[gpui::test]
async fn quit_preserves_popouts_for_restore(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: data.clone(),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let ws = e.providers[0].workspace().clone();
    let target = PopOutTarget::conversation(ws, cid("D1"));
    let handle = cxv
        .update(|_window, cx| parla::popout::open(target, cx))
        .unwrap();
    settle(cxv);

    let path = window_state::path_in(&data);
    assert_eq!(window_state::load(&path).len(), 1, "saved while open");

    cxv.update(|_window, cx| parla::popout::prepare_for_quit(cx));
    cxv.update(|_window, cx| {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    settle(cxv);

    assert_eq!(
        window_state::load(&path).len(),
        1,
        "quit keeps windows.json for restore"
    );
}

#[gpui::test]
async fn add_session_restores_pending_popouts_without_panic(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let path = window_state::path_in(&data);
    let k1 = e.providers[0].workspace().clone();
    let target = PopOutTarget::conversation(k1.clone(), cid("D1"));
    window_state::save(
        &path,
        &[SavedWindow {
            target: target.clone(),
            bounds: None,
        }],
    )
    .unwrap();
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: data.clone(),
            cache: dir.path().join("cache"),
        });
        let _ = parla::popout::restore(&path, cx);
    });
    let (registry, data_d, cache_d) = (e.registry.clone(), e.data.clone(), e.cache.clone());
    let w = cx.new(|cx| {
        let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
        let mut w = Workspaces::new(&registry, store, cx).with_dirs(data_d, cache_d);
        w.start_all(cx);
        w
    });
    cx.update(|cx| cx.set_global(WorkspacesHandle(w.clone())));
    settle_until(cx, |cx| {
        cx.update(|cx| parla::popout::find(&target, cx).is_some())
    });
}

#[gpui::test]
async fn conversation_popout_loads_history_without_main_open(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);
    view.read_with(cxv, |v, _| assert_eq!(*v.phase(), Phase::Ready));

    let session = view.read_with(cxv, |v, _| v.session().unwrap().clone());
    let selected_before = session.read_with(cxv, |s, _| s.selected().cloned());
    // Deliberately do not open in main window.
    let ws = e.providers[0].workspace().clone();
    let _handle = cxv
        .update(|_window, cx| parla::popout::open(PopOutTarget::conversation(ws, cid("D1")), cx))
        .expect("popout opens");
    settle(cxv);

    session.read_with(cxv, |s, _| {
        assert!(
            !s.state().windows[&cid("D1")].is_empty(),
            "history loaded for pop-out"
        );
        assert_eq!(
            s.selected().cloned(),
            selected_before,
            "main window selection unchanged"
        );
    });
}

#[gpui::test]
async fn thread_popout_shares_viewers_with_main_panel(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);
    let session = view.read_with(cxv, |v, _| v.session().unwrap().clone());
    session.update(cxv, |s, cx| s.open(cid("D1"), cx));
    settle(cxv);

    let root = domain::MessageId {
        conversation: cid("D1"),
        ts: domain::MessageTs("1.000000".into()),
    };
    view.update_in(cxv, |v, window, cx| {
        v.open_thread_panel(root.clone(), window, cx)
    });
    settle(cxv);

    let ws = e.providers[0].workspace().clone();
    let pop = cxv
        .update(|_window, cx| {
            parla::popout::open_thread(ws, cid("D1"), domain::MessageTs("1.000000".into()), cx)
        })
        .expect("thread popout opens");
    settle(cxv);

    // Closing the main panel the real way must not remove the thread while the
    // pop-out still views it.
    let panel = view.read_with(cxv, |v, _| v.thread_panel().unwrap().clone());
    let focus = panel.read_with(cxv, |p, cx| p.composer().read(cx).focus_handle(cx));
    cxv.update(|window, cx| window.focus(&focus, cx));
    cxv.dispatch_action(components::ClosePanel);
    settle(cxv);
    view.read_with(cxv, |v, _| {
        assert!(v.thread_panel().is_none(), "main panel closed")
    });
    session.read_with(cxv, |s, _| {
        assert!(
            s.state().threads.contains_key(&root),
            "thread stays while pop-out views it"
        );
    });

    // Closing the pop-out releases its ref.
    cxv.update(|_window, cx| {
        pop.update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    settle(cxv);
    session.read_with(cxv, |s, _| {
        assert!(
            !s.state().threads.contains_key(&root),
            "thread removed when no viewers remain"
        );
    });
}

#[gpui::test]
async fn thread_popout_close_releases_only_its_viewer(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);
    let session = view.read_with(cxv, |v, _| v.session().unwrap().clone());
    session.update(cxv, |s, cx| s.open(cid("D1"), cx));
    settle(cxv);

    let root = domain::MessageId {
        conversation: cid("D1"),
        ts: domain::MessageTs("1.000000".into()),
    };
    view.update_in(cxv, |v, window, cx| {
        v.open_thread_panel(root.clone(), window, cx)
    });
    settle(cxv);

    let ws = e.providers[0].workspace().clone();
    let pop = cxv
        .update(|_window, cx| {
            parla::popout::open_thread(ws, cid("D1"), domain::MessageTs("1.000000".into()), cx)
        })
        .expect("thread popout opens");
    settle(cxv);

    cxv.update(|_window, cx| {
        pop.update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    settle(cxv);
    session.read_with(cxv, |s, _| {
        assert!(
            s.state().threads.contains_key(&root),
            "thread stays while main panel views it"
        );
    });

    let panel = view.read_with(cxv, |v, _| v.thread_panel().unwrap().clone());
    let focus = panel.read_with(cxv, |p, cx| p.composer().read(cx).focus_handle(cx));
    cxv.update(|window, cx| window.focus(&focus, cx));
    cxv.dispatch_action(components::ClosePanel);
    settle(cxv);
    session.read_with(cxv, |s, _| {
        assert!(
            !s.state().threads.contains_key(&root),
            "thread removed when no viewers remain"
        );
    });
}

#[gpui::test]
async fn remove_workspace_closes_its_popouts(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: data.clone(),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let k1 = e.providers[0].workspace().clone();
    let _handle = cxv
        .update(|_window, cx| {
            parla::popout::open(PopOutTarget::conversation(k1.clone(), cid("D1")), cx)
        })
        .unwrap();
    settle(cxv);
    let path = window_state::path_in(&data);
    assert_eq!(window_state::load(&path).len(), 1);

    w.update(cxv, |w, cx| {
        drop(w.remove(&k1, cx));
    });
    settle(cxv);

    assert_eq!(
        window_state::load(&path).len(),
        0,
        "removed workspace pop-out dropped"
    );
    w.read_with(cxv, |w, _| assert_eq!(w.popout_count(&k1), 0));
}

#[gpui::test]
async fn delete_all_closes_popouts_without_repersist(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: e.data.clone(),
            cache: e.cache.clone(),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let k1 = e.providers[0].workspace().clone();
    let path = window_state::path_in(&e.data);
    let _handle = cxv
        .update(|_window, cx| {
            parla::popout::open(PopOutTarget::conversation(k1.clone(), cid("D1")), cx)
        })
        .unwrap();
    settle(cxv);
    assert_eq!(window_state::load(&path).len(), 1);

    let task = w.update(cxv, |w, cx| w.delete_all(cx));
    task.await.unwrap();
    settle(cxv);
    cxv.update(|_window, cx| parla::popout::prepare_for_quit(cx));
    settle(cxv);

    assert!(
        !path.exists(),
        "windows.json not re-persisted after delete-all"
    );
}

#[gpui::test]
async fn restore_skips_removed_workspace(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: data.clone(),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let path = window_state::path_in(&data);
    let k1 = e.providers[0].workspace().clone();
    let k2 = e.providers[1].workspace().clone();
    let gone = settings::WorkspaceKey::new("fake", "gone");
    let windows = vec![
        SavedWindow {
            target: PopOutTarget::conversation(k1, cid("D1")),
            bounds: None,
        },
        SavedWindow {
            target: PopOutTarget::conversation(gone, cid("D1")),
            bounds: None,
        },
        SavedWindow {
            target: PopOutTarget::conversation(k2, cid("D1")),
            bounds: None,
        },
    ];
    window_state::save(&path, &windows).unwrap();

    let restored = cxv.update(|_window, cx| parla::popout::restore(&path, cx));
    settle(cxv);
    assert_eq!(restored, 2, "skips removed workspace");
}

#[gpui::test]
async fn quit_after_delete_all_does_not_recreate_windows(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: e.data.clone(),
            cache: e.cache.clone(),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let k1 = e.providers[0].workspace().clone();
    let path = window_state::path_in(&e.data);
    let _handle = cxv
        .update(|_window, cx| {
            parla::popout::open(PopOutTarget::conversation(k1.clone(), cid("D1")), cx)
        })
        .unwrap();
    settle(cxv);

    let task = w.update(cxv, |w, cx| w.delete_all(cx));
    task.await.unwrap();
    settle(cxv);

    cxv.update(|_window, cx| parla::popout::prepare_for_quit(cx));
    settle(cxv);
    assert!(
        !path.exists(),
        "quit after delete-all must not recreate windows.json"
    );
}

#[gpui::test]
async fn persistence_resumes_after_new_session(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: e.data.clone(),
            cache: e.cache.clone(),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let k1 = e.providers[0].workspace().clone();
    let path = window_state::path_in(&e.data);
    let _handle = cxv
        .update(|_window, cx| {
            parla::popout::open(PopOutTarget::conversation(k1.clone(), cid("D1")), cx)
        })
        .unwrap();
    settle(cxv);
    assert_eq!(window_state::load(&path).len(), 1);

    let task = w.update(cxv, |w, cx| w.delete_all(cx));
    task.await.unwrap();
    settle(cxv);
    cxv.update(|_window, cx| parla::popout::prepare_for_quit(cx));
    settle(cxv);
    assert!(!path.exists(), "windows.json cleared by delete-all");

    w.update(cxv, |w, cx| w.adopt(k1.clone(), cx));
    settle_until(cxv, |cx| w.read_with(cx, |w, _| w.session(&k1).is_some()));
    let _handle = cxv
        .update(|_window, cx| {
            parla::popout::open(PopOutTarget::conversation(k1.clone(), cid("D1")), cx)
        })
        .unwrap();
    settle(cxv);
    assert_eq!(
        window_state::load(&path).len(),
        1,
        "windows.json written after a new session"
    );
}

#[gpui::test]
async fn quit_handler_registered_once(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let k1 = e.providers[0].workspace().clone();
    let k2 = e.providers[1].workspace().clone();
    let k3 = e.providers[2].workspace().clone();
    for (k, c) in [(k1, "D1"), (k2, "D1"), (k3, "D1")] {
        let _ = cxv
            .update(|_window, cx| parla::popout::open(PopOutTarget::conversation(k, cid(c)), cx))
            .unwrap();
    }
    settle(cxv);
    cxv.read(|cx| {
        assert_eq!(
            parla::popout::quit_handler_registrations(cx),
            1,
            "quit handler installed once"
        );
    });
}

#[gpui::test]
async fn popout_thread_close_releases_only_its_viewer_once(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);
    let session = view.read_with(cxv, |v, _| v.session().unwrap().clone());
    session.update(cxv, |s, cx| s.open(cid("D1"), cx));
    settle(cxv);

    let root = domain::MessageId {
        conversation: cid("D1"),
        ts: domain::MessageTs("1.000000".into()),
    };
    view.update_in(cxv, |v, window, cx| {
        v.open_thread_panel(root.clone(), window, cx)
    });
    settle(cxv);

    let ws = e.providers[0].workspace().clone();
    let pop = cxv
        .update(|_window, cx| {
            parla::popout::open_thread(ws, cid("D1"), domain::MessageTs("1.000000".into()), cx)
        })
        .expect("thread popout opens");
    settle(cxv);

    pop.update(cxv, |v, window, cx| {
        let panel = v.thread_panel().expect("expected thread popout");
        let focus = panel.read(cx).composer().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.dispatch_action(Box::new(components::ClosePanel), cx);
    })
    .unwrap();
    settle(cxv);

    session.read_with(cxv, |s, _| {
        assert!(
            s.state().threads.contains_key(&root),
            "thread stays while main panel views it"
        );
    });
}

#[gpui::test]
async fn sign_out_closes_workspace_popouts(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: data.clone(),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let k1 = e.providers[0].workspace().clone();
    let path = window_state::path_in(&data);
    let target = PopOutTarget::conversation(k1.clone(), cid("D1"));
    let _handle = cxv
        .update(|_window, cx| parla::popout::open(target.clone(), cx))
        .unwrap();
    settle(cxv);
    assert_eq!(window_state::load(&path).len(), 1);

    w.update(cxv, |w, cx| w.forget(&k1, cx));
    for _ in 0..400 {
        settle(cxv);
        if cxv.update(|_, cx| parla::popout::find(&target, cx).is_none()) {
            break;
        }
    }
    assert!(
        cxv.update(|_, cx| parla::popout::find(&target, cx).is_none()),
        "sign-out closes workspace pop-outs"
    );
    assert_eq!(window_state::load(&path).len(), 0);
    w.read_with(cxv, |w, _| assert_eq!(w.popout_count(&k1), 0));
}

#[gpui::test]
async fn re_sign_in_closes_old_workspace_popouts(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: data.clone(),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);

    let k1 = e.providers[0].workspace().clone();
    let path = window_state::path_in(&data);
    let target = PopOutTarget::conversation(k1.clone(), cid("D1"));
    let _handle = cxv
        .update(|_window, cx| parla::popout::open(target.clone(), cx))
        .unwrap();
    settle(cxv);
    assert_eq!(window_state::load(&path).len(), 1);
    w.read_with(cxv, |w, _| assert_eq!(w.popout_count(&k1), 1));

    w.update(cxv, |w, cx| w.adopt(k1.clone(), cx));
    for _ in 0..800 {
        settle(cxv);
        if cxv.update(|_, cx| parla::popout::find(&target, cx).is_none()) {
            break;
        }
    }
    assert!(
        cxv.update(|_, cx| parla::popout::find(&target, cx).is_none()),
        "re-sign-in closes old session pop-outs"
    );
    assert_eq!(window_state::load(&path).len(), 0);
    w.read_with(cxv, |w, _| assert_eq!(w.popout_count(&k1), 0));
}

#[gpui::test]
async fn popout_saves_its_draft_on_close_and_restores_it(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);
    let session = view.read_with(cxv, |v, _| v.session().unwrap().clone());
    session.update(cxv, |s, cx| s.open(cid("D1"), cx));
    settle(cxv);

    let ws = e.providers[0].workspace().clone();
    let target = PopOutTarget::conversation(ws, cid("D1"));
    let handle = cxv
        .update(|_window, cx| parla::popout::open(target.clone(), cx))
        .expect("popout opens");
    settle(cxv);
    cxv.update(|_window, cx| {
        handle
            .update(cx, |v, _, cx| {
                let c = v.composer().unwrap();
                c.update(cx, |c, cx| c.set_text("wip".into(), cx));
            })
            .unwrap();
        handle
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    settle(cxv);
    let key = domain::DraftKey {
        conversation: cid("D1"),
        thread: None,
    };
    assert_eq!(
        session.read_with(cxv, |s, _| s.draft(&key).map(|d| d.text.to_string())),
        Some("wip".to_string()),
        "saved when the pop-out closed"
    );

    let handle = cxv
        .update(|_window, cx| parla::popout::open(target, cx))
        .expect("popout reopens");
    settle(cxv);
    let text = cxv.update(|_window, cx| {
        handle
            .update(cx, |v, _, cx| {
                v.composer().unwrap().read(cx).text(cx).to_string()
            })
            .unwrap()
    });
    assert_eq!(text, "wip", "restored on open");
}

#[gpui::test]
async fn switching_workspace_saves_drafts_and_closes_the_thread(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);
    let first = view.read_with(cxv, |v, _| v.session().unwrap().clone());
    first.update(cxv, |s, cx| s.open(cid("D1"), cx));
    settle(cxv);
    let root = domain::MessageId {
        conversation: cid("D1"),
        ts: domain::MessageTs("1.000000".into()),
    };
    view.update_in(cxv, |v, window, cx| {
        v.open_thread_panel(root.clone(), window, cx)
    });
    settle(cxv);
    let composer = view.read_with(cxv, |v, _| v.composer().unwrap().clone());
    composer.update(cxv, |c, cx| c.set_text("unsent".into(), cx));

    w.update(cxv, |w, cx| w.activate(&key("w2"), cx));
    settle(cxv);

    let second = view.read_with(cxv, |v, _| v.session().unwrap().clone());
    assert_ne!(first, second, "switched");
    assert!(view.read_with(cxv, |v, _| v.thread_panel().is_none()));
    let dkey = domain::DraftKey {
        conversation: cid("D1"),
        thread: None,
    };
    assert_eq!(
        first.read_with(cxv, |s, _| s.draft(&dkey).map(|d| d.text.to_string())),
        Some("unsent".to_string())
    );
    first.read_with(cxv, |s, _| {
        assert!(!s.state().threads.contains_key(&root), "thread closed");
    });
}

#[gpui::test]
async fn restore_is_off_with_windows_restore_false(cx: &mut TestAppContext) {
    let e = env(cx, r#"{ "windows": { "restore": false } }"#);
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("settings.json"),
        r#"{ "windows": { "restore": false } }"#,
    )
    .unwrap();
    init(cx, &dir);
    let w = workspaces(cx, &e);
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: data.clone(),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (_view, cxv) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    settle(cxv);
    let path = window_state::path_in(&data);
    let k1 = e.providers[0].workspace().clone();
    window_state::save(
        &path,
        &[SavedWindow {
            target: PopOutTarget::conversation(k1, cid("D1")),
            bounds: None,
        }],
    )
    .unwrap();
    let restored = cxv.update(|_window, cx| parla::popout::restore(&path, cx));
    settle(cxv);
    assert_eq!(restored, 0);
}
