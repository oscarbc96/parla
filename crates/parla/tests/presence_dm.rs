//! Presence is watched for the people on screen only (debounced, no polling),
//! and "Message" opens a direct message even when none exists yet.

use std::sync::Arc;
use std::time::Duration;

use components::{ProfileSource, SidebarSource};
use domain::test_support::{Call, FakeProvider, conv, member, uid};
use domain::{Capabilities, ChatProvider, ConversationKind};
use gpui::{AppContext as _, Entity, TestAppContext};
use parla::session::WorkspaceSession;
use store::{MemoryStore, Store};

fn dm(id: &str, peer: &str) -> domain::Conversation {
    let mut c = conv(id, "");
    c.kind = ConversationKind::Direct;
    c.peer = Some(uid(peer));
    c
}

fn setup(cx: &mut TestAppContext) -> (Arc<FakeProvider>, Entity<WorkspaceSession>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    std::mem::forget(dir);
    let provider = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            presence: true,
            presence_watch: true,
            open_direct: true,
            ..Default::default()
        }),
    );
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Ana"));
    provider.add_member(member("u3", "Bo"));
    provider.add_conversation(dm("D2", "u2"));
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    (provider, session)
}

fn watches(p: &FakeProvider) -> Vec<Vec<domain::UserId>> {
    p.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::WatchPresence(v) => Some(v),
            _ => None,
        })
        .collect()
}

#[gpui::test]
async fn visible_dm_peers_are_watched_after_a_debounce(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    // Rows are read by the sidebar when it paints.
    cx.update(|cx| session.read(cx).rows(cx));
    assert!(watches(&provider).is_empty(), "debounced, not immediate");
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(watches(&provider), [vec![uid("u2")]]);
    // Nothing changed: no timer, no further call.
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(watches(&provider).len(), 1);
}

#[gpui::test]
async fn a_new_open_profile_joins_the_watched_set_in_one_batch(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    cx.update(|cx| session.read(cx).rows(cx));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    session.update(cx, |s, cx| s.show_profile(uid("u3"), cx));
    session.update(cx, |s, cx| s.show_profile(uid("u3"), cx));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(watches(&provider).last(), Some(&vec![uid("u2"), uid("u3")]));
    assert_eq!(watches(&provider).len(), 2);
}

#[gpui::test]
async fn message_without_a_dm_opens_one_and_navigates(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    cx.update(|cx| {
        session.update(cx, |s, cx| ProfileSource::open_dm(s, &uid("u3"), cx));
    });
    cx.run_until_parked();
    assert!(provider.calls().contains(&Call::OpenDirect(uid("u3"))));
    session.read_with(cx, |s, _| {
        assert_eq!(s.selected().map(|c| &*c.0), Some("dm-u3"));
        assert!(s.has_dm(&uid("u3")));
    });
}

#[gpui::test]
async fn message_with_a_known_dm_just_navigates(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    cx.update(|cx| {
        session.update(cx, |s, cx| ProfileSource::open_dm(s, &uid("u2"), cx));
    });
    cx.run_until_parked();
    assert!(
        !provider
            .calls()
            .iter()
            .any(|c| matches!(c, Call::OpenDirect(_)))
    );
    session.read_with(cx, |s, _| {
        assert_eq!(s.selected().map(|c| &*c.0), Some("D2"))
    });
}
