//! The toast stack in the main window (FakeProvider): `Toasts::push` shows a
//! toast at the bottom left, its action runs and dismisses, it goes by itself.

mod support;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use components::{Toast, ToastKind, Toasts};
use domain::ChatProvider as _;
use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, uid};
use gpui::{TestAppContext, VisualTestContext};
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use settings::ConfigPaths;
use store::{MemoryStore, Store};

fn window(cx: &mut TestAppContext) -> (tempfile::TempDir, &mut VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider));
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, vcx) = cx.add_window_view(|w, cx| RootView::new(&registry, st, w, cx));
    vcx.run_until_parked();
    let session = view.read_with(vcx, |v, _| v.session().unwrap().clone());
    session.update(vcx, |s, cx| s.open(cid("C1"), cx));
    vcx.run_until_parked();
    (dir, vcx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
async fn a_pushed_toast_shows_at_the_bottom_left(cx: &mut TestAppContext) {
    let (_dir, cx) = window(cx);
    draw(cx);
    assert!(cx.debug_bounds("toast-0").is_none());
    cx.update(|_, cx| {
        Toasts::push(cx, Toast::new(ToastKind::Info, "Link copied"));
    });
    draw(cx);
    let b = cx.debug_bounds("toast-0").expect("toast painted");
    let win = cx.update(|w, _| w.bounds());
    assert!(
        b.bottom_right().y <= win.bottom_right().y,
        "inside the window"
    );
    assert!(b.bottom_right().y > win.center().y, "in the lower half");
    assert!(b.center().x < win.center().x, "on the left");
    // Past the rail and the sidebar: it never hides the workspace switcher.
    let rail = cx.debug_bounds("workspace-rail").expect("rail");
    assert!(b.origin.x >= rail.bottom_right().x);
}

#[gpui::test]
async fn the_action_runs_and_the_toast_goes(cx: &mut TestAppContext) {
    let (_dir, cx) = window(cx);
    let ran = Rc::new(Cell::new(0));
    let r = ran.clone();
    cx.update(|_, cx| {
        Toasts::push(
            cx,
            Toast::new(ToastKind::Success, "Done").action("Undo", move |_| r.set(r.get() + 1)),
        );
    });
    draw(cx);
    let b = cx.debug_bounds("toast-action-0").expect("action painted");
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    draw(cx);
    assert_eq!(ran.get(), 1);
    assert!(cx.debug_bounds("toast-0").is_none());
}

#[gpui::test]
async fn a_toast_leaves_on_its_own(cx: &mut TestAppContext) {
    let (_dir, cx) = window(cx);
    cx.update(|_, cx| {
        Toasts::push(
            cx,
            Toast::new(ToastKind::Info, "Saved").timeout(Duration::from_secs(5)),
        );
    });
    draw(cx);
    assert!(cx.debug_bounds("toast-0").is_some());
    cx.executor().advance_clock(Duration::from_secs(6));
    draw(cx);
    assert!(cx.debug_bounds("toast-0").is_none());
}

#[gpui::test]
async fn at_most_the_cap_is_painted(cx: &mut TestAppContext) {
    let (_dir, cx) = window(cx);
    cx.update(|_, cx| {
        for i in 0..components::MAX_VISIBLE_TOASTS + 3 {
            Toasts::push(cx, Toast::new(ToastKind::Info, format!("n{i}")));
        }
    });
    draw(cx);
    let names = [
        "toast-0", "toast-1", "toast-2", "toast-3", "toast-4", "toast-5", "toast-6",
    ];
    assert!(names.len() >= components::MAX_VISIBLE_TOASTS + 3);
    let painted = names
        .iter()
        .filter(|n| cx.debug_bounds(n).is_some())
        .count();
    assert_eq!(painted, components::MAX_VISIBLE_TOASTS);
}

#[gpui::test]
async fn copy_link_in_a_popout_toasts_in_the_popout(cx: &mut TestAppContext) {
    use parla::root_view::Phase;
    use parla::window_state::PopOutTarget;
    use parla::workspaces::WorkspacesHandle;

    let e = support::env(cx, "{}");
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path().join("config"));
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    let w = support::workspaces(cx, &e);
    cx.update(|cx| {
        cx.set_global(parla::removal::AppDirs {
            data: dir.path().join("data"),
            cache: dir.path().join("cache"),
        });
        cx.set_global(WorkspacesHandle(w.clone()));
    });
    let (view, main) = cx.add_window_view(|window, cx| {
        let w = cx.global::<WorkspacesHandle>().0.clone();
        RootView::with_workspaces(&e.registry, w, window, cx)
    });
    support::settle(main);
    view.read_with(main, |v, _| assert_eq!(*v.phase(), Phase::Ready));
    let target = PopOutTarget::conversation(e.providers[0].workspace().clone(), cid("D1"));
    let handle = main
        .update(|_, cx| parla::popout::open(target, cx))
        .expect("popout opens");
    support::settle(main);
    let (timeline, pop_stack) = main.update(|_, cx| {
        handle
            .update(cx, |v, window, cx| {
                window.activate_window();
                (v.timeline().unwrap(), components::Toasts::host(window, cx))
            })
            .unwrap()
    });
    let main_stack = main.update(|window, cx| components::Toasts::host(window, cx));
    assert_ne!(pop_stack, main_stack);
    let id = domain::MessageId {
        conversation: cid("D1"),
        ts: domain::MessageTs("1.000000".into()),
    };
    main.update(|_, cx| timeline.update(cx, |t, cx| assert!(t.copy_link(&id, cx))));
    let titles = |s: &gpui::Entity<components::ToastStack>, cx: &mut VisualTestContext| {
        s.read_with(cx, |s, _| {
            s.visible()
                .iter()
                .map(|t| t.title.to_string())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(titles(&pop_stack, main), ["Link copied"]);
    assert!(titles(&main_stack, main).is_empty());
}
