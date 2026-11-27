//! Reminders through `WorkspaceSession`: Parla keeps them when the provider has
//! none, and asks the provider when it has.

use std::sync::Arc;

use components::{ComposerSource, RemindersSource, SidebarSource};
use domain::test_support::{Call, FakeProvider, conv, member, uid};
use domain::{Capabilities, ChatProvider, FixedNow, RemindTarget, ReminderId, SlashCommand};
use gpui::{AppContext as _, Entity, TestAppContext};
use jiff::Timestamp;
use parla::session::WorkspaceSession;
use store::{MemoryStore, Store};

/// 2026-11-15 10:00:00 UTC.
const NOW: i64 = 1_794_736_800;

fn setup(
    cx: &mut TestAppContext,
    server: bool,
    store: Arc<dyn Store>,
) -> (Arc<FakeProvider>, Entity<WorkspaceSession>) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_now(cx, Arc::new(FixedNow(Timestamp::from_second(NOW).unwrap())));
    });
    let caps = Capabilities {
        reminders: server,
        ..Capabilities::default()
    };
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")).with_capabilities(caps));
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Other"));
    let p: Arc<dyn ChatProvider> = provider.clone();
    let session = cx.new(|cx| WorkspaceSession::new(p, store, cx));
    cx.run_until_parked();
    (provider, session)
}

fn texts(s: &WorkspaceSession) -> Vec<String> {
    s.reminders()
        .map(|all| all.iter().map(|r| r.text.to_string()).collect())
        .unwrap_or_default()
}

#[gpui::test]
async fn local_reminders_are_kept_and_survive_a_restart(cx: &mut TestAppContext) {
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (provider, s) = setup(cx, false, store.clone());
    // The list is loaded (empty) at startup, so the view is not stuck loading.
    s.read_with(cx, |s, _| assert_eq!(s.reminders(), Some(&[][..])));
    s.update(cx, |s, cx| {
        RemindersSource::add(s, "stretch".into(), "in 20 minutes".into(), cx)
    });
    cx.run_until_parked();
    s.read_with(cx, |s, _| {
        assert_eq!(texts(s), ["stretch"]);
        let r = &s.reminders().unwrap()[0];
        assert_eq!(r.due, Some(NOW + 20 * 60));
        assert_eq!(s.reminders_entry(), Some(1));
    });
    assert!(
        provider
            .calls()
            .iter()
            .all(|c| !matches!(c, Call::AddReminder { .. }))
    );
    // A second session over the same store sees it, and complete and delete stick.
    let (_p2, s2) = setup(cx, false, store.clone());
    s2.read_with(cx, |s, _| assert_eq!(texts(s), ["stretch"]));
    let id = s2.read_with(cx, |s, _| s.reminders().unwrap()[0].id.clone());
    s2.update(cx, |s, cx| RemindersSource::complete(s, id.clone(), cx));
    cx.run_until_parked();
    s2.read_with(cx, |s, _| {
        assert!(s.reminders().unwrap()[0].completed);
        assert_eq!(s.reminders_entry(), Some(0));
    });
    let (_p3, s3) = setup(cx, false, store.clone());
    s3.read_with(cx, |s, _| assert!(s.reminders().unwrap()[0].completed));
    s3.update(cx, |s, cx| RemindersSource::delete(s, id, cx));
    cx.run_until_parked();
    let (_p4, s4) = setup(cx, false, store);
    s4.read_with(cx, |s, _| assert!(texts(s).is_empty()));
}

#[gpui::test]
async fn local_mode_only_accepts_times_it_can_place(cx: &mut TestAppContext) {
    let (_p, s) = setup(cx, false, Arc::new(MemoryStore::default()));
    s.read_with(cx, |s, cx| {
        assert!(s.local());
        assert!(s.when_ok("tomorrow at 09:00", cx));
        assert!(!s.when_ok("every other tuesday", cx));
        assert!(!s.when_ok("today at 09:00", cx), "already past");
    });
}

#[gpui::test]
async fn slash_remind_me_is_kept_locally(cx: &mut TestAppContext) {
    let (_p, s) = setup(cx, false, Arc::new(MemoryStore::default()));
    let remind = |target: RemindTarget, when: &str| SlashCommand::Remind {
        target,
        what: "call back".into(),
        when: when.into(),
    };
    let r = s.update(cx, |s, cx| {
        ComposerSource::run_slash(s, remind(RemindTarget::Me, "in 2 hours"), cx)
    });
    assert_eq!(r, Ok(()));
    cx.run_until_parked();
    s.read_with(cx, |s, _| {
        assert_eq!(texts(s), ["call back"]);
        assert_eq!(s.reminders().unwrap()[0].due, Some(NOW + 2 * 3600));
    });
    // For someone else, or at a time Parla cannot place: refused, nothing kept.
    for (t, w) in [
        (RemindTarget::Person("u2".into()), "in 2 hours"),
        (RemindTarget::Me, "every other tuesday"),
    ] {
        let r = s.update(cx, |s, cx| ComposerSource::run_slash(s, remind(t, w), cx));
        assert!(r.is_err());
    }
    cx.run_until_parked();
    s.read_with(cx, |s, _| assert_eq!(texts(s).len(), 1));
}

#[gpui::test]
async fn provider_reminders_still_go_to_the_provider(cx: &mut TestAppContext) {
    let (provider, s) = setup(cx, true, Arc::new(MemoryStore::default()));
    assert!(s.read_with(cx, |s, _| !s.local()));
    s.update(cx, |s, cx| {
        RemindersSource::add(s, "stretch".into(), "every monday".into(), cx)
    });
    s.update(cx, |s, cx| {
        RemindersSource::complete(s, ReminderId(Arc::from("r1")), cx)
    });
    cx.run_until_parked();
    let calls = provider.calls();
    assert!(calls.iter().any(|c| matches!(
        c,
        Call::AddReminder { text, when, .. } if text == "stretch" && when == "every monday"
    )));
    // The grammar is the provider's; Parla only checks its shape.
    s.read_with(cx, |s, cx| assert!(s.when_ok("every other tuesday", cx)));
}
