//! History back and forward, and the quick switcher wired to the window.

mod support;

use std::sync::Arc;

use domain::test_support::{cid, conv, mid};
use gpui::TestAppContext;
use parla::nav_history::{CAPACITY, NavEntry, NavHistory};
use parla::removal::AppDirs;
use parla::root_view::RootView;
use parla::workspaces::WorkspacesHandle;
use store::{MemoryStore, Store};
use support::*;

fn entry(w: &str, c: &str) -> NavEntry {
    NavEntry {
        workspace: key(w),
        conversation: cid(c),
        message: None,
    }
}

#[test]
fn back_and_forward_walk_the_entries() {
    let mut h = NavHistory::new();
    for c in ["A", "B", "C"] {
        h.visit(entry("w1", c));
    }
    assert_eq!(h.back(|_| true), Some(entry("w1", "B")));
    assert_eq!(h.back(|_| true), Some(entry("w1", "A")));
    assert_eq!(h.back(|_| true), None);
    assert_eq!(h.forward(|_| true), Some(entry("w1", "B")));
    assert_eq!(h.current(), Some(&entry("w1", "B")));
}

#[test]
fn new_open_drops_forward() {
    let mut h = NavHistory::new();
    for c in ["A", "B", "C"] {
        h.visit(entry("w1", c));
    }
    h.back(|_| true);
    h.back(|_| true);
    h.visit(entry("w1", "D"));
    assert_eq!(h.forward(|_| true), None);
    assert_eq!(h.back(|_| true), Some(entry("w1", "A")));
}

#[test]
fn revisiting_the_current_one_adds_nothing() {
    let mut h = NavHistory::new();
    h.visit(entry("w1", "A"));
    h.visit(entry("w1", "A"));
    assert_eq!(h.len(), 1);
    // The message of a jump is kept.
    let mut jump = entry("w1", "A");
    jump.message = Some(mid("A", "1.000000"));
    h.visit(jump.clone());
    assert_eq!(h.len(), 1);
    assert_eq!(h.current(), Some(&jump));
}

#[test]
fn same_id_in_two_workspaces_is_two_entries() {
    let mut h = NavHistory::new();
    h.visit(entry("w1", "D1"));
    h.visit(entry("w2", "D1"));
    assert_eq!(h.len(), 2);
    assert_eq!(h.back(|_| true), Some(entry("w1", "D1")));
}

#[test]
fn capacity_is_bounded() {
    let mut h = NavHistory::new();
    for i in 0..CAPACITY + 30 {
        h.visit(entry("w1", &format!("C{i}")));
    }
    assert_eq!(h.len(), CAPACITY);
    assert_eq!(
        h.current(),
        Some(&entry("w1", &format!("C{}", CAPACITY + 29)))
    );
    // The oldest were dropped.
    let mut n = 0;
    while h.back(|_| true).is_some() {
        n += 1;
    }
    assert_eq!(n, CAPACITY - 1);
}

#[test]
fn invalid_entries_are_skipped() {
    let mut h = NavHistory::new();
    for c in ["A", "B", "C"] {
        h.visit(entry("w1", c));
    }
    assert_eq!(
        h.back(|e| e.conversation != cid("B")),
        Some(entry("w1", "A"))
    );
    // Nothing valid behind: the position stays.
    assert_eq!(h.back(|_| false), None);
    assert_eq!(h.current(), Some(&entry("w1", "A")));
}

#[test]
fn recent_is_newest_first_without_duplicates() {
    let mut h = NavHistory::new();
    for c in ["A", "B", "A", "C"] {
        h.visit(entry("w1", c));
    }
    let r: Vec<_> = h
        .recent()
        .into_iter()
        .map(|(_, c)| c.0.to_string())
        .collect();
    assert_eq!(r, ["C", "A", "B"]);
}

// ---- the window ----

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
    let ws = vcx.update(|_, cx| cx.global::<WorkspacesHandle>().0.clone());
    settle_visual_until(vcx, |cx| ws.read_with(cx, |w, cx| w.ready_to_lock(cx)));
    (view, vcx)
}

fn settle_visual(cx: &mut gpui::VisualTestContext) {
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn where_am_i(view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext) -> (String, String) {
    view.read_with(cx, |v, cx| {
        (
            v.session().unwrap().read(cx).state().workspace.id.0.clone(),
            v.shown_conversation()
                .map(|c| c.0.to_string())
                .unwrap_or_default(),
        )
    })
}

/// Each workspace gets a channel `C<n>` besides the DM `D1`.
fn add_channels(e: &Env) {
    for (i, p) in e.providers.iter().enumerate() {
        p.add_conversation(conv(&format!("C{}", i + 1), &format!("chan{}", i + 1)));
    }
}

fn open(view: &gpui::Entity<RootView>, w: &str, c: &str, cx: &mut gpui::VisualTestContext) {
    let (w, c) = (key(w), cid(c));
    let want = (w.id.0.clone(), c.0.to_string());
    view.update_in(cx, |v, window, cx| v.navigate(w, c, None, window, cx));
    settle_visual_until(cx, |cx| where_am_i(view, cx) == want);
}

/// Wait until every workspace has loaded its channel from its provider (the
/// switcher lists what the sessions hold when it opens).
fn wait_channels_loaded(cx: &mut gpui::VisualTestContext) {
    let ws = cx.update(|_, cx| cx.global::<WorkspacesHandle>().0.clone());
    settle_visual_until(cx, |cx| {
        ws.read_with(cx, |w, cx| {
            (1..=3).all(|i| {
                w.session(&key(&format!("w{i}"))).is_some_and(|s| {
                    s.read(cx)
                        .state()
                        .conversations
                        .contains_key(&cid(&format!("C{i}")))
                })
            })
        })
    });
}

/// Opens the switcher; the window ignores the request until it is ready.
fn open_switcher_when_ready(view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext) {
    settle_visual_until(cx, |cx| {
        if view.read_with(cx, |v, _| v.quick_switcher().is_none()) {
            view.update_in(cx, |v, window, cx| v.open_quick_switcher(window, cx));
        }
        view.read_with(cx, |v, _| v.quick_switcher().is_some())
    });
}

/// Wait (real time, bounded) until the window shows `c` of workspace `w`,
/// then assert it: the store and providers answer from their own threads.
fn expect_at(view: &gpui::Entity<RootView>, cx: &mut gpui::VisualTestContext, w: &str, c: &str) {
    let want = (w.to_string(), c.to_string());
    settle_visual_until(cx, |cx| where_am_i(view, cx) == want);
    assert_eq!(where_am_i(view, cx), want);
}

#[gpui::test]
async fn back_forward_across_workspaces(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    add_channels(&e);
    let (view, cx) = view(cx, &e);
    wait_channels_loaded(cx);
    open(&view, "w1", "D1", cx);
    open(&view, "w2", "C2", cx);
    open(&view, "w3", "D1", cx);
    assert_eq!(where_am_i(&view, cx), ("w3".into(), "D1".into()));
    view.update_in(cx, |v, window, cx| v.go_back(window, cx));
    expect_at(&view, cx, "w2", "C2");
    view.update_in(cx, |v, window, cx| v.go_back(window, cx));
    expect_at(&view, cx, "w1", "D1");
    view.update_in(cx, |v, window, cx| v.go_forward(window, cx));
    expect_at(&view, cx, "w2", "C2");
    view.update_in(cx, |v, window, cx| v.go_forward(window, cx));
    expect_at(&view, cx, "w3", "D1");
}

#[gpui::test]
async fn new_open_after_back_drops_forward(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    add_channels(&e);
    let (view, cx) = view(cx, &e);
    wait_channels_loaded(cx);
    open(&view, "w1", "D1", cx);
    open(&view, "w1", "C1", cx);
    view.update_in(cx, |v, window, cx| v.go_back(window, cx));
    expect_at(&view, cx, "w1", "D1");
    open(&view, "w2", "C2", cx);
    view.update_in(cx, |v, window, cx| v.go_forward(window, cx));
    expect_at(&view, cx, "w2", "C2"); // forward is empty
    view.update_in(cx, |v, window, cx| v.go_back(window, cx));
    expect_at(&view, cx, "w1", "D1");
}

#[gpui::test]
async fn pick_activates_owning_workspace(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    add_channels(&e);
    let (view, cx) = view(cx, &e);
    wait_channels_loaded(cx);
    assert_eq!(where_am_i(&view, cx).0, "w1");
    // Open the switcher, find a channel that only workspace 3 has, confirm.
    open_switcher_when_ready(&view, cx);
    let picker = view
        .read_with(cx, |v, _| v.quick_switcher().cloned())
        .expect("open");
    let all = picker.read_with(cx, |p, _| p.delegate().chosen().is_none());
    assert!(all);
    cx.simulate_input("chan3");
    // Enter picks the highlighted match, so wait until the list is filtered.
    settle_visual_until(cx, |cx| {
        picker.read_with(cx, |p, _| {
            use components::PickerDelegate as _;
            let m = p.delegate().matches();
            !m.is_empty() && m.iter().all(|i| i.workspace.id.0 == "w3")
        })
    });
    cx.simulate_keystrokes("enter");
    expect_at(&view, cx, "w3", "C3");
    view.read_with(cx, |v, _| {
        assert!(v.quick_switcher().is_none(), "closed after confirm")
    });
    // The window's active workspace is the owner.
    let ws = cx.update(|_, cx| cx.global::<WorkspacesHandle>().0.clone());
    assert_eq!(
        ws.read_with(cx, |w, _| w.active().cloned()),
        Some(key("w3"))
    );
}

#[gpui::test]
async fn switcher_lists_every_workspace_and_escape_closes(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    add_channels(&e);
    let (view, cx) = view(cx, &e);
    wait_channels_loaded(cx);
    open_switcher_when_ready(&view, cx);
    let picker = view
        .read_with(cx, |v, _| v.quick_switcher().cloned())
        .unwrap();
    let listed = |cx: &mut gpui::VisualTestContext| -> std::collections::BTreeSet<String> {
        picker.read_with(cx, |p, _| {
            use components::PickerDelegate as _;
            p.delegate()
                .matches()
                .iter()
                .map(|i| i.workspace.id.0.clone())
                .collect()
        })
    };
    settle_visual_until(cx, |cx| listed(cx).len() == 3);
    let workspaces = listed(cx);
    assert_eq!(workspaces.len(), 3);
    cx.simulate_keystrokes("escape");
    settle_visual(cx);
    view.read_with(cx, |v, _| assert!(v.quick_switcher().is_none()));
}

#[test]
fn shipped_keymaps_bind_the_switcher_and_history() {
    use settings::{parse_keymap, resolve_bindings};
    for (name, text, m) in [
        (
            "macos",
            include_str!("../../../assets/keymaps/default-macos.json"),
            "cmd",
        ),
        (
            "linux",
            include_str!("../../../assets/keymaps/default-linux.json"),
            "ctrl",
        ),
        (
            "windows",
            include_str!("../../../assets/keymaps/default-windows.json"),
            "ctrl",
        ),
    ] {
        let blocks = parse_keymap(std::path::Path::new(name), text).unwrap();
        let (bindings, warnings) = resolve_bindings(&blocks, &[], &parla::actions::is_known);
        assert!(warnings.is_empty(), "{name}: {warnings:?}");
        let has = |k: String, a: &str| bindings.iter().any(|b| b.keystrokes == k && b.action == a);
        assert!(has(format!("{m}-k"), "workspace::QuickSwitch"), "{name}");
        assert!(has(format!("{m}-["), "workspace::GoBack"), "{name}");
        assert!(has(format!("{m}-]"), "workspace::GoForward"), "{name}");
        // No chord starts with the switcher key (it would delay it).
        assert!(
            !bindings
                .iter()
                .any(|b| b.keystrokes.starts_with(&format!("{m}-k "))),
            "{name}"
        );
    }
}

#[gpui::test]
async fn cmd_k_works_before_any_conversation_is_open(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let (view, cx) = view(cx, &e);
    view.read_with(cx, |v, _| assert!(v.shown_conversation().is_none()));
    cx.simulate_keystrokes("cmd-k");
    settle_visual(cx);
    view.read_with(cx, |v, _| assert!(v.quick_switcher().is_some()));
    cx.simulate_keystrokes("cmd-k");
    settle_visual(cx);
    view.read_with(cx, |v, _| {
        assert!(v.quick_switcher().is_none(), "toggles closed")
    });
}

#[test]
fn can_back_and_forward_follow_the_cursor() {
    let mut h = NavHistory::new();
    assert!(!h.can_back() && !h.can_forward());
    h.visit(entry("w1", "A"));
    h.visit(entry("w1", "B"));
    assert!(h.can_back() && !h.can_forward());
    h.back(|_| true);
    assert!(!h.can_back() && h.can_forward());
}

#[test]
fn buttons_are_enabled_only_when_a_valid_entry_is_reachable() {
    let mut h = NavHistory::new();
    for c in ["A", "B", "GONE"] {
        h.visit(entry("w1", c));
    }
    let gone = |e: &NavEntry| &*e.conversation.0 != "GONE";
    assert!(h.can_back_to(gone));
    h.back(|_| true);
    h.back(|_| true);
    // At "A": nothing behind.
    assert!(!h.can_back_to(gone));
    assert!(h.can_forward_to(gone), "B is valid");
    h.forward(|_| true);
    h.forward(|_| true);
    assert!(h.can_back_to(gone));
    assert!(!h.can_forward_to(gone));
    let mut g = NavHistory::new();
    for c in ["GONE", "B"] {
        g.visit(entry("w1", c));
    }
    assert!(
        g.can_back() && !g.can_back_to(gone),
        "only a gone entry behind"
    );
}
