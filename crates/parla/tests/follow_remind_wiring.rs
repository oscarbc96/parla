//! Following a thread and "Remind me about this", wired from the views to the
//! provider and the local reminder list (FakeProvider).
use std::sync::Arc;

use components::{MessageAction, RemindersSource as _, ThreadSource as _, TimelineSource as _};
use domain::test_support::{Call, FakeFactory, FakeProvider, cid, conv, member, msg, uid};
use domain::{Capabilities, MessageId};
use gpui::TestAppContext;
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use settings::ConfigPaths;
use store::{MemoryStore, Store};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

fn root() -> MessageId {
    msg("C1", "1700000100.000000", "").id
}

type View<'a> = (
    gpui::Entity<RootView>,
    Arc<FakeProvider>,
    &'a mut gpui::VisualTestContext,
);

fn window(cx: &mut TestAppContext, caps: Capabilities) -> View<'_> {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")).with_capabilities(caps));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Bob"));
    let mut m = msg("C1", "1700000100.000000", "can you check the contrast?");
    m.author = Some(uid("u2"));
    provider.add_message(m);
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider.clone()));
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, vcx) = cx.add_window_view(|w, cx| RootView::new(&registry, st, w, cx));
    vcx.run_until_parked();
    let session = view.read_with(vcx, |v, _| v.session().unwrap().clone());
    session.update(vcx, |s, cx| s.open(cid("C1"), cx));
    vcx.run_until_parked();
    (view, provider, vcx)
}

#[gpui::test]
async fn the_bell_follows_through_the_provider(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let caps = Capabilities {
        threads: true,
        thread_follow: true,
        ..Capabilities::default()
    };
    let (view, provider, cx) = window(cx, caps);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    assert_eq!(
        session.read_with(cx, |s, _| s.thread_follow(&root())),
        Some(false)
    );
    session.update(cx, |s, cx| s.set_thread_followed(root(), true, cx));
    cx.run_until_parked();
    assert_eq!(
        session.read_with(cx, |s, _| s.thread_follow(&root())),
        Some(true)
    );
    assert!(provider.calls().iter().any(|c| matches!(
        c,
        Call::SetThreadFollowed { root: r, follow: true } if *r == root()
    )));
}

#[gpui::test]
async fn no_bell_without_the_capability(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let caps = Capabilities {
        threads: true,
        ..Capabilities::default()
    };
    let (view, _p, cx) = window(cx, caps);
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    assert_eq!(session.read_with(cx, |s, _| s.thread_follow(&root())), None);
}

#[gpui::test]
async fn remind_me_keeps_a_local_reminder_with_its_message(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let (view, _p, cx) = window(cx, Capabilities::default());
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    let tl = view.read_with(cx, |v, _| v.timeline().unwrap().clone());
    let acts = tl.read_with(cx, |t, cx| t.actions(&root(), cx));
    assert!(acts.contains(&MessageAction::RemindMe));
    session.update(cx, |s, cx| s.remind_about(root(), 1_900_000_000, cx));
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        let all = s.reminders().expect("loaded");
        let r = all
            .iter()
            .find(|r| r.link.is_some())
            .expect("linked reminder");
        assert_eq!(r.due, Some(1_900_000_000));
        assert!(r.about_message(), "the label is not stored as text");
        let link = r.link.as_ref().unwrap();
        assert_eq!(link.message, root());
        assert_eq!(link.author, Some(uid("u2")));
        assert_eq!(&*link.excerpt, "can you check the contrast?");
    });
}

#[gpui::test]
async fn a_provider_that_keeps_reminders_offers_no_remind_me(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let caps = Capabilities {
        reminders: true,
        ..Capabilities::default()
    };
    let (view, _p, cx) = window(cx, caps);
    let tl = view.read_with(cx, |v, _| v.timeline().unwrap().clone());
    let acts = tl.read_with(cx, |t, cx| t.actions(&root(), cx));
    assert!(!acts.contains(&MessageAction::RemindMe));
}
