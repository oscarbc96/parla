//! The "New" divider shows when a conversation with unread messages is opened
//! through the session (opening marks it read, the divider uses the marker
//! the conversation had before).

mod support;

use std::sync::Arc;

use domain::test_support::{cid, conv, msg, ts, uid};
use gpui::TestAppContext;
use parla::removal::AppDirs;
use parla::root_view::RootView;
use store::{MemoryStore, Store};
use support::*;

fn open_with(cx: &mut TestAppContext, read: Option<&str>) -> bool {
    let e = env(cx, "{}");
    let p = &e.providers[0];
    let mut c = conv("C1", "chan1");
    c.last_read = read.map(ts);
    c.latest = Some(ts("1700000300.000001"));
    c.unread = read.is_none_or(|r| r < "1700000300.000001");
    p.add_conversation(c);
    for (t, who) in [
        ("1700000100.000001", "u2"),
        ("1700000200.000001", "u2"),
        ("1700000300.000001", "u2"),
    ] {
        let mut m = msg("C1", t, "hello");
        m.author = Some(uid(who));
        p.add_message(m);
    }
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
    // The stores and providers answer from their own threads: wait for the
    // workspaces to be loaded, then for the newest message to be drawn (the
    // divider is part of the same frame), not for a fixed time.
    let ws = vcx.update(|_, cx| cx.global::<parla::workspaces::WorkspacesHandle>().0.clone());
    settle_visual_until(vcx, |cx| ws.read_with(cx, |w, cx| w.ready_to_lock(cx)));
    view.update_in(vcx, |v, window, cx| {
        v.navigate(key("w1"), cid("C1"), None, window, cx)
    });
    settle_visual_until(vcx, |cx| {
        cx.debug_bounds("timeline-row-1700000300.000001").is_some()
    });
    // The read position loads from the store on its own thread and can land a
    // frame after the rows, so give the divider a bounded moment to appear.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        vcx.run_until_parked();
        if vcx.debug_bounds("timeline-new-divider").is_some() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[gpui::test]
async fn opening_an_unread_conversation_shows_the_new_divider(cx: &mut TestAppContext) {
    assert!(open_with(cx, Some("1700000100.000001")));
}

#[gpui::test]
async fn opening_a_read_conversation_shows_none(cx: &mut TestAppContext) {
    assert!(!open_with(cx, Some("1700000300.000001")));
}
