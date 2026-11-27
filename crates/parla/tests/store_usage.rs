//! Free disk space is read when the stores open and not again within the
//! throttle, never on a timer.

mod support;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::TestAppContext;
use parla::store_usage::StoreUsage;
use store_sqlite::FreeSpace;
use support::{env, settle, workspaces};

/// A free-space source with a settable value that counts its reads.
struct Source {
    free: AtomicU64,
    reads: AtomicU64,
}

impl FreeSpace for Source {
    fn free_bytes(&self, _: &Path) -> Option<u64> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Some(self.free.load(Ordering::SeqCst))
    }
}

fn source(free: u64) -> Arc<Source> {
    Arc::new(Source {
        free: AtomicU64::new(free),
        reads: AtomicU64::new(0),
    })
}

const MIB: u64 = 1024 * 1024;

#[gpui::test]
async fn low_space_at_startup_shows_the_banner_with_one_read_for_all_workspaces(
    cx: &mut TestAppContext,
) {
    let e = env(cx, r#"{ "locale": "en" }"#);
    let src = source(500 * MIB);
    e.sqlite.set_free_space(src.clone());
    assert!(cx.update(|cx| cx.try_global::<StoreUsage>().is_none()));
    let _w = workspaces(cx, &e);
    settle(cx);
    assert!(cx.update(|cx| StoreUsage::banner_visible(cx)));
    assert_eq!(
        src.reads.load(Ordering::SeqCst),
        1,
        "three workspaces opened, one read: the throttle is store-wide"
    );
    settle(cx);
    assert_eq!(
        src.reads.load(Ordering::SeqCst),
        1,
        "an idle app reads nothing"
    );
}

#[gpui::test]
async fn plenty_of_space_at_startup_shows_nothing(cx: &mut TestAppContext) {
    let e = env(cx, r#"{ "locale": "en" }"#);
    e.sqlite.set_free_space(source(50 * 1024 * MIB));
    let _w = workspaces(cx, &e);
    settle(cx);
    assert!(cx.update(|cx| cx.try_global::<StoreUsage>().is_some()));
    assert!(!cx.update(|cx| StoreUsage::banner_visible(cx)));
}

#[gpui::test]
async fn the_banner_has_a_hysteresis_band(cx: &mut TestAppContext) {
    let _e = env(cx, r#"{ "locale": "en" }"#);
    let gib = 1024 * MIB;
    let visible = |cx: &mut TestAppContext| cx.update(|cx| StoreUsage::banner_visible(cx));
    cx.update(|cx| StoreUsage::record(gib - 1, cx));
    assert!(visible(cx));
    cx.update(|cx| StoreUsage::record(gib + gib / 4, cx));
    assert!(visible(cx), "between 1 and 1.5 GiB it stays");
    cx.update(|cx| StoreUsage::record(gib + gib / 2 + 1, cx));
    assert!(!visible(cx));
    cx.update(|cx| StoreUsage::record(gib + gib / 4, cx));
    assert!(!visible(cx), "and stays hidden going down into the band");
}
