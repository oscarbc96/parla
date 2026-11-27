//! The window follows the manager, and removal asks first. Scratch dirs only.

mod support;

use std::sync::Arc;

use gpui::TestAppContext;
use parla::removal::AppDirs;
use parla::root_view::{Phase, RootView};
use parla::workspaces::WorkspacesHandle;
use store::{MemoryStore, Store};
use support::*;

fn view<'a>(
    cx: &'a mut TestAppContext,
    e: &Env,
) -> (gpui::Entity<RootView>, &'a mut gpui::VisualTestContext) {
    let dirs = AppDirs {
        data: e.data.clone(),
        cache: e.cache.clone(),
    };
    cx.update(|cx| {
        cx.set_global(dirs);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    let r = e.registry.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, vcx) = cx.add_window_view(|window, cx| RootView::new(&r, st, window, cx));
    // The accounts start on their own threads: the window only shows one once
    // its session exists, and a removal before that has no workspace to ask
    // about. Every store must exist before a removal asserts on its file.
    let (shown, w) = (
        view.clone(),
        vcx.update(|_, cx| cx.global::<WorkspacesHandle>().0.clone()),
    );
    settle_visual_until(vcx, move |vcx| {
        shown.read_with(vcx, |v, _| v.session().is_some())
            && w.read_with(vcx, |w, cx| w.ready_to_lock(cx))
    });
    (view, vcx)
}

fn settle_visual(cx: &mut gpui::VisualTestContext) {
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn shown(view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext) -> String {
    view.read_with(cx, |v, cx| {
        v.session().unwrap().read(cx).state().workspace.id.0.clone()
    })
}

#[gpui::test]
async fn window_follows_the_active_workspace(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let (view, cx) = view(cx, &e);
    assert_eq!(shown(&view, cx), "w1");
    let ws = cx.update(|_, cx| cx.global::<WorkspacesHandle>().0.clone());
    ws.update(cx, |w, cx| w.activate(&key("w3"), cx));
    settle_visual(cx);
    assert_eq!(shown(&view, cx), "w3");
}

#[gpui::test]
async fn remove_asks_first_and_cancel_removes_nothing(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let (view, cx) = view(cx, &e);
    view.update_in(cx, |v, window, cx| v.remove_workspace(window, cx));
    cx.run_until_parked();
    assert!(cx.has_pending_prompt(), "a confirmation is shown");
    cx.simulate_prompt_answer("Cancel");
    settle_visual(cx);
    assert_eq!(e.registry.accounts().len(), 3);
    assert!(e.sqlite.path(&key("w1")).exists());
}

#[gpui::test]
async fn confirmed_remove_deletes_and_shows_the_next_workspace(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let (view, cx) = view(cx, &e);
    view.update_in(cx, |v, window, cx| v.remove_workspace(window, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Remove workspace");
    // The database, its key and the media cache go on the store's threads.
    let (v, gone) = (view.clone(), e.sqlite.path(&key("w1")));
    settle_visual_until(cx, move |cx| {
        !gone.exists()
            && v.read_with(cx, |v, cx| {
                v.session()
                    .is_some_and(|s| s.read(cx).state().workspace.id.0 == "w2")
            })
    });
    assert!(!e.sqlite.path(&key("w1")).exists());
    assert!(e.sqlite.path(&key("w2")).exists());
    assert_eq!(shown(&view, cx), "w2");
}

#[gpui::test]
async fn delete_all_returns_to_sign_in(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let (view, cx) = view(cx, &e);
    view.update_in(cx, |v, window, cx| v.delete_all_data(window, cx));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Delete all data");
    // The data dir goes on the store's threads; the window follows at once.
    let (v, data) = (view.clone(), e.data.clone());
    settle_visual_until(cx, move |cx| {
        !data.exists() && v.read_with(cx, |v, _| v.phase() == &Phase::NoAccounts)
    });
    assert!(!e.data.exists());
    assert!(e.config.join("settings.json").exists());
    view.read_with(cx, |v, _| assert_eq!(v.phase(), &Phase::NoAccounts));
}
