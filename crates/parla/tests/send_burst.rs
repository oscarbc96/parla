//! A burst of sends (owner report: messages typed back to back stayed pending
//! for seconds): every send reaches the provider at once, without waiting for
//! the answer of the one before it, and all turn sent when the server answers,
//! even when every store write fails (a full disk) or never answers.

use std::sync::Arc;

use domain::test_support::{Call, FakeProvider, Op, cid, conv, uid};
use domain::{ChatProvider, DeliveryState, ProviderEvent};
use gpui::{AppContext as _, Entity, TestAppContext};
use parla::session::WorkspaceSession;
use store::{MemoryStore, Store, WriteFault};

const TEXTS: [&str; 5] = ["one", "two", "three", "four", "five"];

fn start(
    cx: &mut TestAppContext,
    fault: WriteFault,
) -> (Arc<FakeProvider>, Entity<WorkspaceSession>) {
    let provider = Arc::new(FakeProvider::new("w1", uid("me")));
    provider.add_conversation(conv("C1", "general"));
    let p: Arc<dyn ChatProvider> = provider.clone();
    let mem = Arc::new(MemoryStore::default());
    let st: Arc<dyn Store> = mem.clone();
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    provider.push(ProviderEvent::Connected { reconnect: false });
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    mem.set_write_fault(fault);
    (provider, session)
}

fn sends(p: &FakeProvider) -> usize {
    p.calls()
        .iter()
        .filter(|c| matches!(c, Call::Send { .. }))
        .count()
}

/// The burst's rows, in timeline order, with whether each is sent.
fn rows(s: &Entity<WorkspaceSession>, cx: &mut TestAppContext) -> Vec<(String, bool)> {
    s.read_with(cx, |s, _| {
        s.state().windows[&cid("C1")]
            .messages()
            .iter()
            .filter(|m| TEXTS.contains(&&*m.text))
            .map(|m| {
                (
                    m.text.to_string(),
                    matches!(m.state, DeliveryState::Delivered),
                )
            })
            .collect()
    })
}

fn burst(cx: &mut TestAppContext, fault: WriteFault) {
    let (provider, session) = start(cx, fault);
    provider.hold(Op::Send);
    for t in TEXTS {
        session.update(cx, |s, cx| s.send(cid("C1"), t.to_string(), cx));
    }
    cx.run_until_parked();
    assert_eq!(
        sends(&provider),
        TEXTS.len(),
        "every send is in flight at once, none waits for the one before"
    );
    assert!(rows(&session, cx).iter().all(|(_, sent)| !sent), "pending");
    provider.release();
    cx.run_until_parked();
    let all = rows(&session, cx);
    let expect: Vec<(String, bool)> = TEXTS.iter().map(|t| (t.to_string(), true)).collect();
    assert_eq!(all, expect, "all sent, in order, after one round trip");
    session.read_with(cx, |s, _| {
        assert!(s.state().outbox().is_empty());
        assert!(s.state().outbox_inflight().is_empty());
    });
}

#[gpui::test]
async fn a_burst_of_sends_goes_out_at_once_and_all_turn_sent(cx: &mut TestAppContext) {
    burst(cx, WriteFault::None);
}

#[gpui::test]
async fn a_burst_turns_sent_when_every_store_write_fails(cx: &mut TestAppContext) {
    burst(cx, WriteFault::Fail);
}

#[gpui::test]
async fn a_burst_turns_sent_when_store_writes_never_answer(cx: &mut TestAppContext) {
    burst(cx, WriteFault::Stall);
}
