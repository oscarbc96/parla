//! `Effect::OpenHuddle` opens the huddle per `calls.join_with`.

use std::sync::Arc;

use domain::test_support::{FakeProvider, cid, conv, uid};
use domain::{Capabilities, ChatProvider, Effect};
use gpui::{AppContext as _, Entity, TestAppContext};
use parla::huddles::HuddleOpen;
use parla::session::WorkspaceSession;
use settings::ConfigPaths;
use store::{MemoryStore, Store};

fn setup(
    cx: &mut TestAppContext,
    join_with: &str,
) -> (tempfile::TempDir, Entity<WorkspaceSession>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    std::fs::write(
        &paths.settings,
        format!(r#"{{ "locale": "en", "calls": {{ "join_with": "{join_with}" }} }}"#),
    )
    .unwrap();
    cx.update(|cx| parla::init_without_watcher(cx, paths));
    let provider = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            huddles: true,
            ..Default::default()
        }),
    );
    provider.add_conversation(conv("C1", "general"));
    let p: Arc<dyn ChatProvider> = provider;
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let s = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    (dir, s)
}

#[gpui::test]
async fn join_with_window_on_unsupported_platform_opens_browser(cx: &mut TestAppContext) {
    let (_d, s) = setup(cx, "window");
    let got = s.update(cx, |s, cx| s.open_huddle(&cid("C1"), cx));
    assert_eq!(
        got,
        HuddleOpen::Browser("https://fake.example.test/huddle/C1".into())
    );
}

#[gpui::test]
async fn join_with_browser_opens_browser(cx: &mut TestAppContext) {
    let (_d, s) = setup(cx, "browser");
    let got = s.update(cx, |s, cx| s.open_huddle(&cid("C1"), cx));
    assert_eq!(
        got,
        HuddleOpen::Browser("https://fake.example.test/huddle/C1".into())
    );
}

#[gpui::test]
async fn join_with_app_uses_app_url(cx: &mut TestAppContext) {
    let (_d, s) = setup(cx, "provider_app");
    let got = s.update(cx, |s, cx| s.open_huddle(&cid("C1"), cx));
    assert_eq!(got, HuddleOpen::App("fake-app://open/C1".into()));
}

#[gpui::test]
async fn no_url_shows_open_in_toast(cx: &mut TestAppContext) {
    // A provider without huddles has neither URL.
    let (_d, _s) = setup(cx, "window");
    let provider = Arc::new(FakeProvider::new("w2", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    let p: Arc<dyn ChatProvider> = provider;
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let s2 = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    let got = s2.update(cx, |s, cx| s.open_huddle(&cid("C1"), cx));
    assert_eq!(got, HuddleOpen::Toast);
    let notice = s2.read_with(cx, |s, _| s.file_notice());
    assert!(matches!(notice, Some(components::FileNotice::Error(_))));
    // And the effect itself runs the same path.
    s2.update(cx, |s, cx| {
        s.run_effect_now(
            Effect::OpenHuddle {
                conversation: cid("C1"),
            },
            cx,
        )
    });
}
