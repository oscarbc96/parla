//! `Effect::Call` and `Effect::Load` run on the provider and feed back.

mod support;

use std::sync::Arc;

use domain::test_support::{Call, FakeProvider, Op, uid};
use domain::{CallOutput, Capabilities, Effect, Input, Load, Loaded, ProviderCall, ProviderError};
use gpui::{Entity, TestAppContext};
use parla::session::WorkspaceSession;

fn setup(cx: &mut TestAppContext) -> (Arc<FakeProvider>, Entity<WorkspaceSession>) {
    let provider = Arc::new(
        FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
            dnd: true,
            status: true,
            pins: true,
            bookmarks: true,
            ..Default::default()
        }),
    );
    let session = support::memory_session(cx, provider.clone());
    (provider, session)
}

#[gpui::test]
async fn call_effect_runs_and_feeds_back(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    session.update(cx, |s, cx| {
        s.run_effect_now(Effect::Call(ProviderCall::Snooze(30)), cx)
    });
    cx.run_until_parked();
    assert!(provider.calls().contains(&Call::Snooze(30)));
    session.read_with(cx, |s, _| match s.last_feedback() {
        Some(Input::CallFinished {
            call: ProviderCall::Snooze(30),
            result: Ok(CallOutput::Dnd(d)),
        }) => assert!(d.snooze_until.is_some()),
        other => panic!("unexpected feedback {other:?}"),
    });
}

#[gpui::test]
async fn load_error_feeds_back_with_load(cx: &mut TestAppContext) {
    let (provider, session) = setup(cx);
    provider.fail_next(Op::Profile, ProviderError::RateLimited);
    let l = Load::Profile(uid("u2"));
    session.update(cx, |s, cx| s.run_effect_now(Effect::Load(l), cx));
    cx.run_until_parked();
    session.read_with(cx, |s, _| match s.last_feedback() {
        Some(Input::Loaded(Err((Load::Profile(u), ProviderError::RateLimited)))) => {
            assert_eq!(*u, uid("u2"))
        }
        other => panic!("unexpected feedback {other:?}"),
    });
}

#[gpui::test]
async fn unsupported_load_feeds_back_unsupported(cx: &mut TestAppContext) {
    let (_p, session) = setup(cx);
    session.update(cx, |s, cx| {
        s.run_effect_now(Effect::Load(Load::Reminders), cx)
    });
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        assert!(matches!(
            s.last_feedback(),
            Some(Input::Loaded(Err((
                Load::Reminders,
                ProviderError::Unsupported
            ))))
        ));
    });
    let _ = Loaded::Scheduled(vec![]);
}

#[gpui::test]
async fn pin_and_bookmark_calls_run(cx: &mut TestAppContext) {
    use domain::test_support::{cid, mid};
    let (provider, session) = setup(cx);
    let id = mid("C1", "1.000001");
    session.update(cx, |s, cx| {
        s.run_effect_now(
            Effect::Call(ProviderCall::SetPinned {
                id: id.clone(),
                pinned: true,
            }),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(
        provider
            .calls()
            .contains(&Call::SetPinned { id, pinned: true })
    );
    session.update(cx, |s, cx| {
        s.run_effect_now(
            Effect::Call(ProviderCall::AddBookmark {
                conversation: cid("C1"),
                title: Arc::from("Docs"),
                link: Arc::from("https://example.com/d"),
            }),
            cx,
        )
    });
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        assert!(matches!(
            s.last_feedback(),
            Some(Input::CallFinished {
                result: Ok(CallOutput::Bookmark(_)),
                ..
            })
        ));
    });
    session.update(cx, |s, cx| {
        s.run_effect_now(Effect::Load(Load::Bookmarks(cid("C1"))), cx)
    });
    cx.run_until_parked();
    session.read_with(cx, |s, _| match s.last_feedback() {
        Some(Input::Loaded(Ok(Loaded::Bookmarks(_, v)))) => assert_eq!(v.len(), 1),
        other => panic!("unexpected feedback {other:?}"),
    });
}
