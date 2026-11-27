//! `Workspaces`: every account runs at once; hidden ones go lean after the
//! configured time and wake on activation. Scratch dirs only.

mod support;

use std::time::Duration;

use domain::test_support::{cid, msg};
use domain::{DraftKey, ProviderEvent};
use gpui::TestAppContext;
use support::*;

const TEN_MIN: &str = r#"{ "performance": { "lean_after_minutes": 10 } }"#;

fn minutes(cx: &mut TestAppContext, m: u64) {
    cx.executor().advance_clock(Duration::from_secs(m * 60));
    settle(cx);
}

/// Wait until `id` is in (or out of) lean mode. `go_lean` and the wake both
/// land through the executor, and the wake first reopens the store on its
/// thread, so wait for the state instead of the fixed settle.
fn wait_lean(
    cx: &mut TestAppContext,
    w: &gpui::Entity<parla::workspaces::Workspaces>,
    id: &str,
    lean: bool,
) {
    let (w, k) = (w.clone(), key(id));
    settle_until(cx, move |cx| {
        w.read_with(cx, |w, cx| {
            w.session(&k).is_some_and(|s| s.read(cx).is_lean() == lean)
        })
    });
}

#[gpui::test]
async fn starts_every_account(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);
    w.read_with(cx, |w, cx| {
        assert_eq!(w.sessions().count(), 3);
        assert_eq!(w.active(), Some(&key("w1")));
        assert_eq!(w.accounts().len(), 3);
        for id in ["w1", "w2", "w3"] {
            assert!(w.session(&key(id)).is_some());
            assert!(!w.session(&key(id)).unwrap().read(cx).state().lean);
        }
    });
}

#[gpui::test]
async fn hidden_workspace_goes_lean_after_setting(cx: &mut TestAppContext) {
    let e = env(cx, TEN_MIN);
    let w = workspaces(cx, &e);
    minutes(cx, 9);
    w.read_with(cx, |w, cx| {
        assert!(!w.session(&key("w2")).unwrap().read(cx).state().lean);
    });
    assert!(store_open(&e, "w2").await);
    minutes(cx, 1);
    wait_lean(cx, &w, "w2", true);
    wait_lean(cx, &w, "w3", true);
    w.read_with(cx, |w, cx| {
        assert!(w.session(&key("w2")).unwrap().read(cx).state().lean);
        assert!(w.session(&key("w3")).unwrap().read(cx).state().lean);
        assert!(!w.session(&key("w1")).unwrap().read(cx).state().lean);
    });
    // Lean closes the store on its thread: wait for the close to land.
    wait_store_closed(cx, &e, "w2");
    assert!(!store_open(&e, "w2").await, "lean closes the store");
    assert!(
        store_open(&e, "w1").await,
        "the active workspace keeps its store"
    );
}

#[gpui::test]
async fn lean_workspace_still_badges_and_notifies(cx: &mut TestAppContext) {
    let e = env(cx, TEN_MIN);
    let w = workspaces(cx, &e);
    minutes(cx, 10);
    let shown = std::rc::Rc::new(std::cell::Cell::new(0));
    struct N(std::rc::Rc<std::cell::Cell<u32>>);
    impl parla::notifications::Notifier for N {
        fn show(&self, _: &domain::notify::Notification, _: &mut gpui::App) {
            self.0.set(self.0.get() + 1);
        }
        fn dismiss_conversation(
            &self,
            _: &settings::WorkspaceKey,
            _: &domain::ConversationId,
            _: &mut gpui::App,
        ) {
        }
        fn set_badge(&self, _: Option<u32>, _: &mut gpui::App) {}
    }
    let n = N(shown.clone());
    cx.update(|cx| parla::notifications::Notifications::new(std::rc::Rc::new(n)).install(cx));
    e.providers[1].push(ProviderEvent::MessagePosted(msg("D1", "2.000000", "ping")));
    settle(cx);
    assert_eq!(shown.get(), 1, "a lean workspace still notifies");
    w.read_with(cx, |w, cx| {
        let s = w.session(&key("w2")).unwrap();
        assert!(s.read(cx).state().lean);
        assert!(s.read(cx).state().conversations.contains_key(&cid("D1")));
    });
}

#[gpui::test]
async fn wake_restores_open_conversation_and_draft(cx: &mut TestAppContext) {
    let e = env(cx, TEN_MIN);
    let w = workspaces(cx, &e);
    let s2 = w.read_with(cx, |w, _| w.session(&key("w2")).unwrap());
    s2.update(cx, |s, cx| {
        s.open(cid("D1"), cx);
        s.focus(cid("D1"), true, true, cx);
    });
    settle(cx);
    let dk = DraftKey {
        conversation: cid("D1"),
        thread: None,
    };
    s2.update(cx, |s, cx| s.set_draft(dk.clone(), "half typed", cx));
    settle(cx);
    minutes(cx, 10);
    // The lean timer fires through the executor, and the wake reopens the
    // store on its thread first: wait for each state.
    wait_lean(cx, &w, "w2", true);
    s2.read_with(cx, |s, _| {
        assert!(s.state().lean);
        assert!(
            s.state().windows.is_empty(),
            "lean releases message windows"
        );
    });
    w.update(cx, |w, cx| w.activate(&key("w2"), cx));
    settle_until(cx, |cx| {
        s2.read_with(cx, |s, _| {
            !s.state().lean
                && s.state()
                    .windows
                    .get(&cid("D1"))
                    .is_some_and(|w| w.messages().len() == 1)
        })
    });
    s2.read_with(cx, |s, _| {
        assert!(!s.state().lean);
        assert_eq!(
            s.selected(),
            Some(&cid("D1")),
            "the open conversation stays"
        );
        assert_eq!(
            s.draft(&dk).map(|d| d.text.to_string()).as_deref(),
            Some("half typed")
        );
        let win = s.state().windows.get(&cid("D1")).expect("window reloaded");
        assert_eq!(win.messages().len(), 1);
    });
    assert!(store_open(&e, "w2").await, "wake reopens the store");
}

#[gpui::test]
async fn activation_cancels_lean_timer(cx: &mut TestAppContext) {
    let e = env(cx, TEN_MIN);
    let w = workspaces(cx, &e);
    minutes(cx, 9);
    w.update(cx, |w, cx| w.activate(&key("w2"), cx));
    settle(cx);
    minutes(cx, 5);
    w.read_with(cx, |w, cx| {
        assert!(!w.session(&key("w2")).unwrap().read(cx).state().lean);
        // The previously active one started its timer at activation.
        assert!(!w.session(&key("w1")).unwrap().read(cx).state().lean);
    });
    minutes(cx, 5);
    w.read_with(cx, |w, cx| {
        assert!(w.session(&key("w1")).unwrap().read(cx).state().lean);
        assert!(!w.session(&key("w2")).unwrap().read(cx).state().lean);
    });
}

#[gpui::test]
async fn hiding_the_window_starts_the_active_timer(cx: &mut TestAppContext) {
    let e = env(cx, TEN_MIN);
    let w = workspaces(cx, &e);
    w.update(cx, |w, cx| w.set_window_visible(false, cx));
    minutes(cx, 10);
    wait_lean(cx, &w, "w1", true);
    w.update(cx, |w, cx| w.set_window_visible(true, cx));
    wait_lean(cx, &w, "w1", false);
    w.read_with(cx, |w, cx| {
        assert!(!w.session(&key("w1")).unwrap().read(cx).state().lean);
    });
}

#[gpui::test]
async fn per_workspace_settings_apply(cx: &mut TestAppContext) {
    let e = env(cx, r#"{ "notifications": { "desktop": "all" } }"#);
    std::fs::create_dir_all(e.config.join("workspaces/fake")).unwrap();
    std::fs::write(
        e.config.join("workspaces/fake/w2.json"),
        r#"{ "notifications": { "desktop": "none" } }"#,
    )
    .unwrap();
    let shown = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    struct N(std::rc::Rc<std::cell::RefCell<Vec<String>>>);
    impl parla::notifications::Notifier for N {
        fn show(&self, n: &domain::notify::Notification, _: &mut gpui::App) {
            self.0.borrow_mut().push(n.workspace.id.0.clone());
        }
        fn dismiss_conversation(
            &self,
            _: &settings::WorkspaceKey,
            _: &domain::ConversationId,
            _: &mut gpui::App,
        ) {
        }
        fn set_badge(&self, _: Option<u32>, _: &mut gpui::App) {}
    }
    cx.update(|cx| {
        parla::notifications::Notifications::new(std::rc::Rc::new(N(shown.clone()))).install(cx)
    });
    let _w = workspaces(cx, &e);
    for p in &e.providers[..2] {
        p.push(ProviderEvent::MessagePosted(msg("D1", "2.000000", "ping")));
    }
    settle(cx);
    settle_until(cx, |_| !shown.borrow().is_empty());
    assert_eq!(*shown.borrow(), vec!["w1".to_string()]);
}

#[gpui::test]
async fn idle_schedules_only_lean_timers(cx: &mut TestAppContext) {
    let e = env(cx, TEN_MIN);
    let w = workspaces(cx, &e);
    // Two hidden workspaces, one timer each; the active one has none.
    w.read_with(cx, |w, _| assert_eq!(w.lean_timers(), 2));
    minutes(cx, 10);
    w.read_with(cx, |w, _| {
        assert_eq!(w.lean_timers(), 0, "fired timers are gone")
    });
    w.update(cx, |w, cx| w.set_window_visible(false, cx));
    w.read_with(cx, |w, _| assert_eq!(w.lean_timers(), 1));
}

#[gpui::test]
async fn forgetting_a_workspace_clears_its_badge(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);
    let badge = std::rc::Rc::new(std::cell::Cell::new(None));
    struct N(std::rc::Rc<std::cell::Cell<Option<u32>>>);
    impl parla::notifications::Notifier for N {
        fn show(&self, _: &domain::notify::Notification, _: &mut gpui::App) {}
        fn dismiss_conversation(
            &self,
            _: &settings::WorkspaceKey,
            _: &domain::ConversationId,
            _: &mut gpui::App,
        ) {
        }
        fn set_badge(&self, b: Option<u32>, _: &mut gpui::App) {
            self.0.set(b);
        }
    }
    let n = N(badge.clone());
    cx.update(|cx| parla::notifications::Notifications::new(std::rc::Rc::new(n)).install(cx));
    e.providers[1].push(ProviderEvent::MessagePosted(msg("D1", "2.000000", "ping")));
    settle_until(cx, |_| badge.get() == Some(1));
    assert_eq!(badge.get(), Some(1));
    w.update(cx, |w, cx| w.forget(&key("w2"), cx));
    assert_eq!(badge.get(), None, "the dead workspace keeps no mentions");
}

#[gpui::test]
async fn clicking_a_message_notification_activates_its_workspace(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    use std::rc::Rc;
    struct N(Rc<RefCell<Vec<domain::notify::Notification>>>);
    impl parla::notifications::Notifier for N {
        fn show(&self, n: &domain::notify::Notification, _: &mut gpui::App) {
            self.0.borrow_mut().push(n.clone());
        }
        fn dismiss_conversation(
            &self,
            _: &settings::WorkspaceKey,
            _: &domain::ConversationId,
            _: &mut gpui::App,
        ) {
        }
        fn set_badge(&self, _: Option<u32>, _: &mut gpui::App) {}
    }
    let shown = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| parla::notifications::Notifications::new(Rc::new(N(shown.clone()))).install(cx));
    let e = env(cx, "{}");
    let w = workspaces(cx, &e);
    cx.update(|cx| cx.set_global(parla::workspaces::WorkspacesHandle(w.clone())));
    e.providers[1].push(ProviderEvent::MessagePosted(msg("D1", "2.000000", "ping")));
    settle(cx);
    let n = shown.borrow().first().cloned().expect("notified");
    w.read_with(cx, |w, _| assert_eq!(w.active(), Some(&key("w1"))));
    let tag = parla::notifications::tag(&n);
    assert!(cx.update(|cx| parla::notifications::Notifications::handle_response(&tag, cx)));
    settle(cx);
    w.read_with(cx, |w, _| assert_eq!(w.active(), Some(&key("w2"))));
}
