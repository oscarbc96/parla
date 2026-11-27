//! Channel admin through the app: the session sends the neutral calls, the
//! answers update the list, and the dialogs (create, browse, edit, invite,
//! leave and archive confirms) drive them.

mod support;

use std::sync::Arc;
use std::time::Duration;

use domain::test_support::{Call, FakeFactory, FakeProvider, Op, cid, conv, member, uid};
use domain::{AdminCall, Capabilities, ProviderError};
use gpui::{Entity, TestAppContext, VisualTestContext};
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use parla::session::{SessionEvent, WorkspaceSession};
use settings::ConfigPaths;
use store::{MemoryStore, Store};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        parla::init_without_watcher(cx, ConfigPaths::at(dir.path()));
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn fake() -> Arc<FakeProvider> {
    let p = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            channel_admin: true,
            ..Default::default()
        }),
    );
    p.add_conversation(conv("C1", "general"));
    let mut other = conv("C2", "design-crit");
    other.is_member = false;
    other.purpose = "Show work early".into();
    p.add_conversation(other);
    p.add_conversation(conv("C3", "launch"));
    p.add_member(member("u1", "Me"));
    p.add_member(member("u2", "Ada Lovelace"));
    p.add_member(member("u3", "Grace Hopper"));
    p
}

async fn start(
    cx: &mut TestAppContext,
    p: Arc<FakeProvider>,
) -> (Entity<RootView>, &mut VisualTestContext) {
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(p));
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store, window, cx));
    cx.run_until_parked();
    (view, cx)
}

fn session(view: &Entity<RootView>, cx: &mut VisualTestContext) -> Entity<WorkspaceSession> {
    view.read_with(cx, |v, _| v.session().unwrap().clone())
}

fn admin_calls(p: &FakeProvider) -> Vec<AdminCall> {
    p.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Admin(a) => Some(a),
            _ => None,
        })
        .collect()
}

fn click(cx: &mut VisualTestContext, sel: &'static str) {
    // Two frames: a deferred menu lays out on the frame after it opens.
    for _ in 0..2 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    let b = cx
        .debug_bounds(sel)
        .unwrap_or_else(|| panic!("{sel} is drawn"));
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
async fn creating_a_channel_opens_it(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    let s = session(&view, cx);
    s.update(cx, |s, cx| {
        s.admin(
            AdminCall::Create {
                name: "launch-plan".into(),
                private: true,
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert_eq!(
        admin_calls(&p),
        [AdminCall::Create {
            name: "launch-plan".into(),
            private: true
        }]
    );
    s.read_with(cx, |s, _| {
        let open = s.selected().cloned().expect("the new channel is open");
        let c = &s.state().conversations[&open];
        assert_eq!(&*c.name, "launch-plan");
        assert!(c.is_member);
    });
}

#[gpui::test]
async fn a_failed_call_tells_why(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    let s = session(&view, cx);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let _sub = cx.update(|_, cx| {
        cx.subscribe(&s, move |_, ev: &SessionEvent, _| {
            if let SessionEvent::AdminFinished { result, .. } = ev {
                sink.lock().unwrap().push(result.clone());
            }
        })
    });
    p.fail_next(Op::Admin, ProviderError::NotAllowed("restricted".into()));
    s.update(cx, |s, cx| s.admin(AdminCall::Leave(cid("C1")), cx));
    cx.run_until_parked();
    let seen = seen.lock().unwrap().clone();
    assert!(
        matches!(seen.as_slice(), [Err(why)] if !why.is_empty()),
        "{seen:?}"
    );
    s.read_with(cx, |s, _| {
        assert!(s.state().conversations[&cid("C1")].is_member)
    });
}

#[gpui::test]
async fn the_channels_plus_opens_the_browser_and_join_opens_the_channel(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    let sidebar = view.read_with(cx, |v, _| v.sidebar().unwrap().clone());
    sidebar.update(cx, |_, cx| {
        cx.emit(components::SidebarEvent::BrowseChannels)
    });
    cx.run_until_parked();
    let browser = view
        .read_with(cx, |v, _| v.channel_browser().cloned())
        .expect("the browser is open");
    browser.read_with(cx, |b, _| {
        let names: Vec<_> = b.rows().iter().map(|r| r.name.to_string()).collect();
        assert_eq!(names, ["general", "design-crit", "launch"]);
    });
    click(cx, "browse-join-1");
    assert_eq!(admin_calls(&p), [AdminCall::Join(cid("C2"))]);
    let s = session(&view, cx);
    s.read_with(cx, |s, _| {
        assert_eq!(s.selected(), Some(&cid("C2")));
        assert!(s.state().conversations[&cid("C2")].is_member);
    });
    assert!(view.read_with(cx, |v, _| v.channel_browser().is_none()));
}

#[gpui::test]
async fn the_create_dialog_checks_the_name_then_creates(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    view.update_in(cx, |v, window, cx| v.create_channel(window, cx));
    cx.run_until_parked();
    let dialog = view
        .read_with(cx, |v, _| v.create_channel_dialog().cloned())
        .expect("the create dialog is open");
    dialog.update(cx, |d, cx| d.set_name("Launch.Plan", cx));
    cx.run_until_parked();
    dialog.read_with(cx, |d, cx| assert!(d.problem(cx).is_some()));
    // An invalid name sends nothing.
    click(cx, "create-channel-submit");
    assert!(admin_calls(&p).is_empty());
    dialog.update(cx, |d, cx| {
        d.set_name("Launch Plan", cx);
        d.set_private(true, cx);
    });
    click(cx, "create-channel-submit");
    assert_eq!(
        admin_calls(&p),
        [AdminCall::Create {
            name: "launch-plan".into(),
            private: true
        }]
    );
    assert!(view.read_with(cx, |v, _| v.create_channel_dialog().is_none()));
}

#[gpui::test]
async fn a_taken_name_keeps_the_dialog_open_with_the_reason(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    view.update_in(cx, |v, window, cx| v.create_channel(window, cx));
    cx.run_until_parked();
    let dialog = view
        .read_with(cx, |v, _| v.create_channel_dialog().cloned())
        .unwrap();
    p.fail_next(Op::Admin, ProviderError::Other("name_taken".into()));
    dialog.update(cx, |d, cx| d.set_name("general", cx));
    click(cx, "create-channel-submit");
    let d = view
        .read_with(cx, |v, _| v.create_channel_dialog().cloned())
        .expect("still open");
    d.read_with(cx, |d, _| assert!(d.error().is_some()));
}

fn open_menu(view: &Entity<RootView>, cx: &mut VisualTestContext, c: &str) {
    let sidebar = view.read_with(cx, |v, _| v.sidebar().unwrap().clone());
    sidebar.update(cx, |_, cx| {
        cx.emit(components::SidebarEvent::ContextMenu {
            conversation: cid(c),
            position: gpui::point(gpui::px(100.0), gpui::px(100.0)),
        })
    });
    cx.run_until_parked();
}

#[gpui::test]
async fn leave_and_archive_ask_first(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    open_menu(&view, cx, "C1");
    for _ in 0..2 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
    // Leave and Archive sit apart, after a divider.
    assert!(cx.debug_bounds("notify-extra-divider-7").is_some());
    assert!(cx.debug_bounds("notify-extra-divider-8").is_none());
    click(cx, "notify-extra-7");
    assert!(view.read_with(cx, |v, _| v.admin_confirm().is_some()));
    // The confirm opens on Cancel: Enter there sends nothing.
    let d = view
        .read_with(cx, |v, _| v.admin_confirm().cloned())
        .unwrap();
    cx.update(|w, cx| {
        use gpui::Focusable as _;
        assert!(
            d.read(cx).focus_handle(cx).is_focused(w),
            "cancel has the focus"
        )
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(admin_calls(&p).is_empty());
    assert!(view.read_with(cx, |v, _| v.admin_confirm().is_none()));
    open_menu(&view, cx, "C1");
    click(cx, "notify-extra-7");
    // Cancel sends nothing.
    click(cx, "confirm-cancel");
    assert!(admin_calls(&p).is_empty());
    assert!(view.read_with(cx, |v, _| v.admin_confirm().is_none()));
    open_menu(&view, cx, "C3");
    click(cx, "notify-extra-8");
    click(cx, "confirm-ok");
    open_menu(&view, cx, "C1");
    click(cx, "notify-extra-7");
    click(cx, "confirm-ok");
    assert_eq!(
        admin_calls(&p),
        [AdminCall::Archive(cid("C3")), AdminCall::Leave(cid("C1"))]
    );
}

#[gpui::test]
async fn editing_sends_only_what_changed(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    let s = session(&view, cx);
    s.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| v.edit_channel(cid("C1"), window, cx));
    cx.run_until_parked();
    let d = view
        .read_with(cx, |v, _| v.edit_channel_dialog().cloned())
        .expect("the edit dialog is open");
    d.update(cx, |d, cx| {
        d.set_field(components::EditField::Topic, "Weekly notes", cx);
        d.set_field(components::EditField::Purpose, "Company news", cx);
    });
    click(cx, "edit-channel-save");
    assert_eq!(
        admin_calls(&p),
        [
            AdminCall::SetTopic {
                conversation: cid("C1"),
                topic: "Weekly notes".into()
            },
            AdminCall::SetPurpose {
                conversation: cid("C1"),
                purpose: "Company news".into()
            },
        ]
    );
    s.read_with(cx, |s, _| {
        assert_eq!(&*s.state().conversations[&cid("C1")].topic, "Weekly notes");
    });
}

#[gpui::test]
async fn inviting_picks_people_who_are_not_in_yet(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    // The provider's member list, not only what is known locally.
    p.set_channel_members(&cid("C1"), &[uid("u1"), uid("u2")]);
    view.update_in(cx, |v, window, cx| v.invite_people(cid("C1"), window, cx));
    cx.run_until_parked();
    let d = view
        .read_with(cx, |v, _| v.invite_dialog().cloned())
        .expect("the invite dialog is open");
    d.read_with(cx, |d, _| {
        let names: Vec<_> = d.shown().iter().map(|r| r.name.to_string()).collect();
        assert_eq!(names, ["Grace Hopper"]);
    });
    d.update(cx, |d, cx| d.set_query("grace", cx));
    cx.run_until_parked();
    d.read_with(cx, |d, _| {
        let names: Vec<_> = d.shown().iter().map(|r| r.name.to_string()).collect();
        assert_eq!(names, ["Grace Hopper"]);
    });
    click(cx, "invite-row-0");
    click(cx, "invite-submit");
    assert_eq!(
        admin_calls(&p),
        [AdminCall::Invite {
            conversation: cid("C1"),
            users: vec![uid("u3")]
        }]
    );
}

#[gpui::test]
async fn an_invite_says_who_could_not_be_added(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    p.set_channel_members(&cid("C1"), &[uid("u1"), uid("u2")]);
    let (view, cx) = start(cx, p.clone()).await;
    let s = session(&view, cx);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let _sub = cx.update(|_, cx| {
        cx.subscribe(&s, move |_, ev: &SessionEvent, _| {
            if let SessionEvent::AdminFinished { result, .. } = ev {
                sink.lock().unwrap().push(result.clone());
            }
        })
    });
    s.update(cx, |s, cx| {
        s.admin(
            AdminCall::Invite {
                conversation: cid("C1"),
                users: vec![uid("u2"), uid("u3")],
            },
            cx,
        )
    });
    cx.run_until_parked();
    let seen = seen.lock().unwrap().clone();
    match seen.as_slice() {
        [Err(why)] => {
            assert!(why.contains("Ada Lovelace"), "{why}");
            assert!(!why.contains("Grace Hopper"), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

#[gpui::test]
async fn leaving_the_open_channel_moves_to_the_next(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    let s = session(&view, cx);
    s.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    s.update(cx, |s, cx| s.admin(AdminCall::Leave(cid("C1")), cx));
    cx.run_until_parked();
    let now = s.read_with(cx, |s, _| s.selected().cloned());
    assert_eq!(now, Some(cid("C3")));
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("C3"))
    );
}

#[gpui::test]
async fn a_live_archive_of_the_last_open_channel_closes_it(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    let s = session(&view, cx);
    s.update(cx, |s, cx| s.open(cid("C3"), cx));
    cx.run_until_parked();
    for c in ["C3", "C1"] {
        let mut m = domain::ConversationMeta::new(cid(c));
        m.is_archived = Some(true);
        p.push_event(domain::ProviderEvent::ConversationMetaChanged(m));
        cx.run_until_parked();
    }
    // C3 went first (to C1), then C1: nothing is left to show.
    assert_eq!(s.read_with(cx, |s, _| s.selected().cloned()), None);
    assert!(view.read_with(cx, |v, _| v.composer().is_none()));
}

#[gpui::test]
async fn typing_in_the_browser_asks_once_it_settles(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let p = fake();
    let (view, cx) = start(cx, p.clone()).await;
    view.update_in(cx, |v, window, cx| v.browse_channels(window, cx));
    cx.run_until_parked();
    let b = view
        .read_with(cx, |v, _| v.channel_browser().cloned())
        .unwrap();
    let browses = |p: &FakeProvider| {
        p.calls()
            .iter()
            .filter(|c| matches!(c, Call::BrowseChannels(..)))
            .count()
    };
    let before = browses(&p);
    for q in ["d", "de", "des"] {
        b.update(cx, |b, cx| b.set_query(q, cx));
        cx.run_until_parked();
    }
    assert_eq!(browses(&p), before, "nothing sent while typing");
    cx.executor().advance_clock(components::BROWSE_DEBOUNCE);
    cx.run_until_parked();
    assert_eq!(browses(&p), before + 1);
    b.read_with(cx, |b, _| {
        let names: Vec<_> = b.rows().iter().map(|r| r.name.to_string()).collect();
        assert_eq!(names, ["design-crit"]);
    });
}

#[gpui::test]
async fn switching_workspace_closes_the_admin_dialog(cx: &mut TestAppContext) {
    let ps: Vec<_> = ["w1", "w2"]
        .iter()
        .map(|id| {
            let p = Arc::new(
                FakeProvider::new(id, uid("u1")).with_capabilities(Capabilities {
                    channel_admin: true,
                    ..Default::default()
                }),
            );
            p.add_conversation(conv("C1", "general"));
            p.add_member(member("u1", "Me"));
            p
        })
        .collect();
    let e = support::env_with(cx, "{}", ps);
    let r = e.registry.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&r, st, window, cx));
    let settle = |cx: &mut VisualTestContext| {
        for _ in 0..30 {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    settle(cx);
    view.update_in(cx, |v, window, cx| v.create_channel(window, cx));
    settle(cx);
    assert!(view.read_with(cx, |v, _| v.create_channel_dialog().is_some()));
    let ws = cx.update(|_, cx| cx.global::<parla::workspaces::WorkspacesHandle>().0.clone());
    let other = ws.read_with(cx, |w, _| {
        if w.active() == Some(&support::key("w1")) {
            support::key("w2")
        } else {
            support::key("w1")
        }
    });
    ws.update(cx, |w, cx| w.activate(&other, cx));
    settle(cx);
    assert!(view.read_with(cx, |v, _| v.create_channel_dialog().is_none()));
}
