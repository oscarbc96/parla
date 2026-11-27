//! The cross-workspace views over three fake workspaces: the manager lists
//! unreads, mentions and scheduled items of all of them, a row opens in its
//! own workspace, and "Mark all read" marks each conversation at its newest.

mod support;

use std::sync::Arc;
use std::time::Duration;

use components::{AggregateEvent, AggregateKind, AggregateSource, AggregateTarget, AllUnreads};
use domain::ScheduleId;
use domain::test_support::{Call, cid, conv, ts};
use gpui::TestAppContext;
use parla::removal::AppDirs;
use parla::root_view::RootView;
use parla::workspaces::WorkspacesHandle;
use store::{MemoryStore, Store};
use support::*;

const TEN_MIN: &str = r#"{ "locale": "en", "performance": { "lean_after_minutes": 10 } }"#;

/// Workspace `i` (1-based) gets unread `C<i>`; w2's carries mentions.
fn seed(e: &Env) {
    for (i, p) in e.providers.iter().enumerate() {
        let mut c = conv(&format!("C{}", i + 1), &format!("chan{}", i + 1));
        c.unread = true;
        c.latest = Some(ts(&format!("{}.000001", 5 + i)));
        c.mentions = if i == 1 { 2 } else { 0 };
        p.add_conversation(c);
    }
}

fn pin_english(cx: &mut TestAppContext) {
    cx.update(|cx| i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap())));
}

#[gpui::test]
async fn manager_lists_unreads_of_every_workspace_mentions_first(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    seed(&e);
    pin_english(cx);
    let w = workspaces(cx, &e);
    settle_until(cx, |cx| w.read_with(cx, |w, cx| w.unreads(cx).len() == 3));
    let rows = w.read_with(cx, |w, cx| w.unreads(cx));
    let got: Vec<(String, String)> = rows
        .iter()
        .map(|r| (r.workspace.key.id.0.clone(), r.label.to_string()))
        .collect();
    assert_eq!(
        got,
        [
            ("w2".to_string(), "chan2".to_string()),
            ("w3".to_string(), "chan3".to_string()),
            ("w1".to_string(), "chan1".to_string()),
        ],
        "mentions first, then newest"
    );
    assert_eq!(&*rows[0].workspace.name, "w2");
}

#[gpui::test]
async fn lean_workspace_mentions_have_no_held_messages(cx: &mut TestAppContext) {
    let e = env(cx, TEN_MIN);
    seed(&e);
    pin_english(cx);
    let w = workspaces(cx, &e);
    settle_until(cx, |cx| w.read_with(cx, |w, cx| w.mentions(cx).len() == 1));
    cx.executor().advance_clock(Duration::from_secs(10 * 60));
    settle(cx);
    let rows = w.read_with(cx, |w, cx| w.mentions(cx));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].row.workspace.key, key("w2"));
    assert_eq!(rows[0].row.mentions, 2);
    assert!(rows[0].messages.is_empty());
}

#[gpui::test]
async fn scheduled_lists_local_items_of_every_workspace(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    pin_english(cx);
    let w = workspaces(cx, &e);
    for (id, at) in [("w3", 4_000_000_300_i64), ("w1", 4_000_000_100)] {
        let s = w.read_with(cx, |w, _| w.session(&key(id)).unwrap());
        s.update(cx, |s, cx| {
            s.schedule(cid("D1"), None, format!("later {id}"), vec![], at, cx)
        });
    }
    // Both rows exist and carry the conversation's name once its workspace
    // has bootstrapped.
    settle_until(cx, |cx| {
        let rows = w.read_with(cx, |w, cx| w.scheduled(cx));
        rows.len() == 2 && rows.iter().all(|r| r.conversation.as_ref() == "dm")
    });
    let rows = w.read_with(cx, |w, cx| w.scheduled(cx));
    let ids: Vec<&str> = rows.iter().map(|r| &*r.workspace.key.id.0).collect();
    assert_eq!(ids, ["w1", "w3"]);
    assert!(rows.iter().all(|r| r.local));
    assert_eq!(rows[0].conversation.as_ref(), "dm");
}

#[gpui::test]
async fn mark_all_read_marks_each_conversation_at_its_latest(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    seed(&e);
    pin_english(cx);
    let w = workspaces(cx, &e);
    // Every workspace must have loaded its unread conversation before the
    // bulk action can see it.
    settle_until(cx, |cx| w.read_with(cx, |w, cx| w.unreads(cx).len() == 3));
    w.update(cx, |w, cx| w.mark_all_read(cx));
    let marked = |e: &Env| {
        e.providers.iter().enumerate().all(|(i, p)| {
            let want = ts(&format!("{}.000001", 5 + i));
            let c = cid(&format!("C{}", i + 1));
            p.calls().iter().any(|call| {
                matches!(call,
                Call::MarkRead { conversation, ts } if *conversation == c && *ts == want)
            })
        })
    };
    // A counts answer that left the provider before the mark can still land
    // after it and show the conversation unread again, as it can for a user;
    // marking once more (idempotent) settles it, since counts now say read.
    settle_until(cx, |cx| {
        if !marked(&e) {
            return false;
        }
        let empty = w.read_with(cx, |w, cx| w.unreads(cx).is_empty());
        if !empty {
            w.update(cx, |w, cx| w.mark_all_read(cx));
        }
        empty
    });
    assert!(marked(&e), "every workspace marks its conversation read");
    assert!(w.read_with(cx, |w, cx| w.unreads(cx)).is_empty());
}

fn view<'a>(
    cx: &'a mut TestAppContext,
    e: &Env,
) -> (gpui::Entity<RootView>, &'a mut gpui::VisualTestContext) {
    let dirs = AppDirs {
        data: e.data.clone(),
        cache: e.cache.clone(),
    };
    cx.update(|cx| cx.set_global(dirs));
    pin_english(cx);
    let r = e.registry.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, vcx) = cx.add_window_view(|window, cx| RootView::new(&r, st, window, cx));
    // The accounts start on their own threads, and nothing opens (a dispatch
    // included) until the window shows the active one.
    let shown = view.clone();
    settle_visual_until(vcx, |vcx| {
        shown.read_with(vcx, |v, _| v.session().is_some())
    });
    (view, vcx)
}

fn settle_visual(cx: &mut gpui::VisualTestContext) {
    for _ in 0..30 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[gpui::test]
async fn row_opens_in_its_workspace(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    seed(&e);
    let (view, cx) = view(cx, &e);
    cx.dispatch_action(AllUnreads);
    // The rows come from every workspace's bootstrap, which runs on its own
    // threads: opening before they are listed would open nothing.
    let v = view.clone();
    settle_visual_until(cx, |cx| {
        v.read_with(cx, |v, cx| {
            v.aggregate_view()
                .is_some_and(|a| a.read(cx).rows().len() == 3)
        })
    });
    let agg = view
        .read_with(cx, |v, _| v.aggregate_view().cloned())
        .expect("All unreads opens");
    // Rows: w2 (mentions), w3, w1. Open the last.
    let order: Vec<String> = agg.read_with(cx, |a, _| {
        a.rows()
            .iter()
            .map(|e| e.workspace.key.id.0.to_string())
            .collect()
    });
    assert_eq!(order, ["w2", "w3", "w1"], "mentions first, then newest");
    agg.update(cx, |a, cx| a.open(2, cx));
    let ws = cx.update(|_, cx| cx.global::<WorkspacesHandle>().0.clone());
    {
        let (v, w) = (view.clone(), ws.clone());
        settle_visual_until(cx, move |cx| {
            w.read_with(cx, |w, _| w.active().cloned()) == Some(key("w1"))
                && v.read_with(cx, |v, _| v.shown_conversation().cloned()) == Some(cid("C1"))
                && v.read_with(cx, |v, _| v.aggregate_view().is_none())
        });
    }
    assert_eq!(
        ws.read_with(cx, |w, _| w.active().cloned()),
        Some(key("w1"))
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("C1"))
    );
    assert!(view.read_with(cx, |v, _| v.aggregate_view().is_none()));
    // Again, a row of another workspace. Opening the first one may have marked
    // its conversation read, so find w3 by what the row points at.
    cx.dispatch_action(AllUnreads);
    let v = view.clone();
    settle_visual_until(cx, |cx| {
        v.read_with(cx, |v, _| v.aggregate_view().is_some())
    });
    let agg = view
        .read_with(cx, |v, _| v.aggregate_view().cloned())
        .unwrap();
    let idx = agg
        .read_with(cx, |a, _| {
            a.rows().iter().position(|e| e.workspace.key.id.0 == "w3")
        })
        .expect("w3 is still unread");
    agg.update(cx, |a, cx| a.open(idx, cx));
    {
        let (v, w) = (view.clone(), ws.clone());
        settle_visual_until(cx, move |cx| {
            w.read_with(cx, |w, _| w.active().cloned()) == Some(key("w3"))
                && v.read_with(cx, |v, _| v.shown_conversation().cloned()) == Some(cid("C3"))
        });
    }
    assert_eq!(
        ws.read_with(cx, |w, _| w.active().cloned()),
        Some(key("w3"))
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.shown_conversation().cloned()),
        Some(cid("C3"))
    );
}

#[gpui::test]
async fn scheduled_row_opens_that_workspaces_scheduled_view(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let (view, cx) = view(cx, &e);
    view.update_in(cx, |v, window, cx| {
        v.open_aggregate(AggregateKind::Scheduled, window, cx)
    });
    settle_visual(cx);
    let agg = view
        .read_with(cx, |v, _| v.aggregate_view().cloned())
        .unwrap();
    agg.update(cx, |_, cx| {
        cx.emit(AggregateEvent::Open {
            workspace: key("w2"),
            target: AggregateTarget::Schedule(ScheduleId(Arc::from("x"))),
        })
    });
    settle_visual(cx);
    let ws = cx.update(|_, cx| cx.global::<WorkspacesHandle>().0.clone());
    assert_eq!(
        ws.read_with(cx, |w, _| w.active().cloned()),
        Some(key("w2"))
    );
    assert!(view.read_with(cx, |v, _| v.scheduled_view().is_some()));
    assert!(view.read_with(cx, |v, _| v.aggregate_view().is_none()));
}

#[test]
fn default_keymaps_bind_all_unreads() {
    for (name, text, k) in [
        (
            "macos",
            include_str!("../../../assets/keymaps/default-macos.json"),
            "cmd-shift-a",
        ),
        (
            "linux",
            include_str!("../../../assets/keymaps/default-linux.json"),
            "ctrl-shift-a",
        ),
        (
            "windows",
            include_str!("../../../assets/keymaps/default-windows.json"),
            "ctrl-shift-a",
        ),
    ] {
        let blocks = settings::parse_keymap(std::path::Path::new(name), text).unwrap();
        let (bindings, warnings) =
            settings::resolve_bindings(&blocks, &[], &parla::actions::is_known);
        assert!(warnings.is_empty(), "{name}: {warnings:?}");
        assert!(
            bindings
                .iter()
                .any(|b| b.keystrokes == k && b.action == "workspace::AllUnreads"),
            "{name}"
        );
    }
}

#[gpui::test]
async fn rail_entries_open_each_view_and_toggle(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    let (view, cx) = view(cx, &e);
    for (sel, kind) in [
        ("rail-all-unreads", AggregateKind::Unreads),
        ("rail-all-mentions", AggregateKind::Mentions),
        ("rail-all-scheduled", AggregateKind::Scheduled),
    ] {
        let b = cx
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} is drawn"));
        cx.simulate_click(b.center(), gpui::Modifiers::none());
        settle_visual(cx);
        assert_eq!(view.read_with(cx, |v, _| v.aggregate_kind()), Some(kind));
    }
    // Clicking the open one again closes it.
    let b = cx.debug_bounds("rail-all-scheduled").unwrap();
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    settle_visual(cx);
    assert_eq!(view.read_with(cx, |v, _| v.aggregate_kind()), None);
}

#[gpui::test]
async fn mark_read_marks_one_conversation_at_its_latest(cx: &mut TestAppContext) {
    let e = env(cx, "{}");
    seed(&e);
    pin_english(cx);
    let w = workspaces(cx, &e);
    settle_until(cx, |cx| w.read_with(cx, |w, cx| w.unreads(cx).len() == 3));
    // The row showed 4.000001 as its newest: that is where it is marked,
    // even though the conversation has moved on since.
    w.update(cx, |w, cx| {
        w.mark_read(&key("w1"), &cid("C1"), ts("4.000001"), cx)
    });
    let marks = |p: &domain::test_support::FakeProvider| {
        p.calls()
            .iter()
            .filter(|c| matches!(c, Call::MarkRead { .. }))
            .count()
    };
    settle_until(cx, |_| marks(&e.providers[0]) > 0);
    assert!(e.providers[0].calls().iter().any(|c| matches!(c,
        Call::MarkRead { conversation, ts: t } if *conversation == cid("C1") && *t == ts("4.000001"))));
    assert_eq!(
        marks(&e.providers[1]) + marks(&e.providers[2]),
        0,
        "only that row"
    );
}
