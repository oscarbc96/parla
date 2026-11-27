//! Typing: the composer's typing signal reaches the provider at most once per
//! few seconds per conversation, and others' typing shows above the composer
//! until it expires on one timer (no tick).

use std::sync::Arc;
use std::time::Duration;

use components::ComposerSource;
use domain::test_support::{Call, FakeProvider, cid, conv, member, ts, uid};
use domain::{Capabilities, ChatProvider, ProviderEvent, TYPING_TTL};
use gpui::{AppContext as _, Entity, TestAppContext};
use parla::session::WorkspaceSession;
use store::{MemoryStore, Store};

fn setup(cx: &mut TestAppContext, typing: bool) -> (Arc<FakeProvider>, Entity<WorkspaceSession>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    std::mem::forget(dir);
    let provider = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            typing,
            threads: true,
            ..Default::default()
        }),
    );
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Ana"));
    provider.add_conversation(conv("C1", "general"));
    provider.add_conversation(conv("C2", "random"));
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    provider.push_event(ProviderEvent::Connected { reconnect: false });
    cx.run_until_parked();
    (provider, session)
}

fn sent(p: &FakeProvider) -> Vec<(String, Option<String>)> {
    p.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Typing {
                conversation,
                thread,
            } => Some((conversation.0.to_string(), thread.map(|t| t.0.to_string()))),
            _ => None,
        })
        .collect()
}

fn type_in(cx: &mut TestAppContext, s: &Entity<WorkspaceSession>, c: &str, thread: Option<&str>) {
    let (c, thread) = (cid(c), thread.map(ts));
    s.update(cx, |s, cx| ComposerSource::typed(s, c, thread, cx));
    cx.run_until_parked();
}

#[gpui::test]
async fn typing_is_sent_at_most_once_per_conversation_in_a_burst(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx, true);
    for _ in 0..5 {
        type_in(cx, &session, "C1", None);
    }
    type_in(cx, &session, "C1", Some("5.000000"));
    type_in(cx, &session, "C2", Some("5.000000"));
    assert_eq!(
        sent(&provider),
        [
            ("C1".to_string(), None),
            ("C2".to_string(), Some("5.000000".to_string()))
        ]
    );
}

#[gpui::test]
async fn a_provider_without_typing_is_never_told(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx, false);
    type_in(cx, &session, "C1", None);
    assert!(sent(&provider).is_empty());
}

#[gpui::test]
async fn others_typing_shows_and_expires_on_its_timer(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx, true);
    let names = |cx: &mut TestAppContext| {
        session.read_with(cx, |s, _| {
            ComposerSource::typing(s, &cid("C1"), None)
                .into_iter()
                .map(|m| m.map(|m| m.id.0.to_string()))
                .collect::<Vec<_>>()
        })
    };
    for u in ["u2", "u1"] {
        provider.push_event(ProviderEvent::Typing {
            conversation: cid("C1"),
            user: uid(u),
            thread: None,
        });
    }
    cx.run_until_parked();
    assert_eq!(
        names(cx),
        [Some("u2".to_string())],
        "own typing is not shown"
    );
    assert!(session.read_with(cx, |s, _| !s.state().typers(&cid("C1"), None).is_empty()));
    // The timer is real time in the shell: let it pass, then run it.
    std::thread::sleep(TYPING_TTL);
    cx.executor()
        .advance_clock(TYPING_TTL + Duration::from_millis(10));
    cx.run_until_parked();
    assert!(names(cx).is_empty());
    assert!(
        session.read_with(cx, |s, _| s.state().typers(&cid("C1"), None).is_empty()),
        "the expiry timer pruned the typer"
    );
}
