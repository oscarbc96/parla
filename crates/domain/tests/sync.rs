use std::collections::HashMap;

use domain::sync::{self, CATCH_UP_MAX_PAGES, CatchUp, CatchUpResult, Older};
use domain::test_support::{Call, FakeProvider, cid, conv, msg, ts, uid};
use domain::*;
use futures::executor::block_on;
use settings::WorkspaceKey;

fn counts(c: &str, latest: Option<&str>) -> ConversationCounts {
    ConversationCounts {
        conversation: cid(c),
        latest: latest.map(ts),
        last_read: None,
        unread: false,
        mentions: 0,
    }
}

fn stored(v: &[(&str, &str)]) -> HashMap<ConversationId, MessageTs> {
    v.iter().map(|(c, t)| (cid(c), ts(t))).collect()
}

#[test]
fn plan_skips_unchanged_and_unstored() {
    let st = stored(&[("a", "5.000000"), ("b", "5.000000")]);
    let remote = [
        counts("a", Some("9.000000")),
        counts("b", Some("5.000000")),
        counts("c", Some("9.000000")),
    ];
    let jobs = sync::plan(&st, &remote, |_| true);
    assert_eq!(
        jobs,
        vec![CatchUp {
            conversation: cid("a"),
            after: ts("5.000000")
        }]
    );
}

#[test]
fn plan_skips_non_members() {
    let st = stored(&[("a", "5.000000"), ("b", "5.000000")]);
    let remote = [counts("a", Some("9.000000")), counts("b", Some("9.000000"))];
    let jobs = sync::plan(&st, &remote, |c| *c == cid("b"));
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].conversation, cid("b"));
}

fn fake(n: u32) -> FakeProvider {
    let f = FakeProvider::new("w1", uid("me"));
    f.add_conversation(conv("c1", "general"));
    for i in 1..=n {
        f.add_message(msg("c1", &format!("{i}.000000"), "x"));
    }
    f
}

fn history_calls(f: &FakeProvider) -> usize {
    f.calls()
        .iter()
        .filter(|c| matches!(c, Call::HistoryBetween { .. }))
        .count()
}

#[test]
fn short_offline_fetches_all_without_gap() {
    let f = fake(40);
    let job = CatchUp {
        conversation: cid("c1"),
        after: ts("10.000000"),
    };
    let r = block_on(sync::run(&f, job)).unwrap();
    assert_eq!(r.messages.len(), 30);
    assert!(r.gap.is_none());
    assert_eq!(history_calls(&f), 1);
    assert!(r.messages.windows(2).all(|w| w[0].id.ts < w[1].id.ts));
}

#[test]
fn long_offline_records_gap() {
    let f = fake(2000);
    let job = CatchUp {
        conversation: cid("c1"),
        after: ts("1.000000"),
    };
    let r = block_on(sync::run(&f, job)).unwrap();
    assert_eq!(history_calls(&f), CATCH_UP_MAX_PAGES as usize);
    assert_eq!(r.messages.len(), 500);
    let gap = r.gap.unwrap();
    assert_eq!(gap.after, ts("1.000000"));
    assert_eq!(gap.before, ts("1501.000000"));
    assert_eq!(r.messages[0].id.ts, ts("1501.000000"));
}

#[test]
fn older_crosses_gap_remotely() {
    let gaps = [Gap {
        conversation: cid("c1"),
        after: ts("10.000000"),
        before: ts("50.000000"),
    }];
    assert_eq!(
        sync::next_older(&ts("50.000000"), true, &gaps),
        Older::Remote {
            after: Some(ts("10.000000")),
            before: ts("50.000000")
        }
    );
}

#[test]
fn older_from_store_when_no_gap() {
    assert_eq!(sync::next_older(&ts("50.000000"), true, &[]), Older::Store);
    assert_eq!(
        sync::next_older(&ts("50.000000"), false, &[]),
        Older::Remote {
            after: None,
            before: ts("50.000000")
        }
    );
}

fn state_with_window() -> WorkspaceState {
    let mut st = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 50);
    reduce(&mut st, Input::Open(cid("c1")));
    st
}

#[test]
fn caught_up_merges_with_live_without_duplicates() {
    let mut st = state_with_window();
    reduce(
        &mut st,
        Input::Provider(ProviderEvent::MessagePosted(msg("c1", "5.000000", "x"))),
    );
    let fx = reduce(
        &mut st,
        Input::CaughtUp(CatchUpResult {
            conversation: cid("c1"),
            messages: vec![
                msg("c1", "3.000000", "x"),
                msg("c1", "4.000000", "x"),
                msg("c1", "5.000000", "x"),
            ],
            gap: Some(Gap {
                conversation: cid("c1"),
                after: ts("1.000000"),
                before: ts("3.000000"),
            }),
        }),
    );
    let w: Vec<_> = st.windows[&cid("c1")]
        .messages()
        .iter()
        .map(|m| m.id.ts.0.to_string())
        .collect();
    assert_eq!(w, ["3.000000", "4.000000", "5.000000"]);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::AddGap(_))))
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Persist(PersistOp::Messages { .. })))
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::MaybeNotify { .. })));
}

#[test]
fn caught_up_with_a_gap_replaces_the_old_window() {
    let mut st = state_with_window();
    for t in ["1.000000", "2.000000"] {
        reduce(
            &mut st,
            Input::Provider(ProviderEvent::MessagePosted(msg("c1", t, "old"))),
        );
    }
    reduce(
        &mut st,
        Input::CaughtUp(CatchUpResult {
            conversation: cid("c1"),
            messages: vec![msg("c1", "8.000000", "new"), msg("c1", "9.000000", "new")],
            gap: Some(Gap {
                conversation: cid("c1"),
                after: ts("2.000000"),
                before: ts("8.000000"),
            }),
        }),
    );
    let w = &st.windows[&cid("c1")];
    let got: Vec<_> = w.messages().iter().map(|m| m.id.ts.0.to_string()).collect();
    assert_eq!(got, ["8.000000", "9.000000"]);
    assert!(w.has_more_before());
}

fn closed(fx: &[Effect]) -> Gap {
    fx.iter()
        .find_map(|e| match e {
            Effect::Persist(PersistOp::CloseGap(g)) => Some(g.clone()),
            _ => None,
        })
        .expect("CloseGap")
}

#[test]
fn filled_gap_is_closed() {
    let mut st = state_with_window();
    let fx = reduce(
        &mut st,
        Input::GapPageLoaded {
            conversation: cid("c1"),
            after: Some(ts("1.000000")),
            before: ts("4.000000"),
            page: HistoryPage {
                messages: vec![msg("c1", "3.000000", "x"), msg("c1", "2.000000", "x")],
                has_more: false,
            },
        },
    );
    let g = closed(&fx);
    assert_eq!((g.after, g.before), (ts("1.000000"), ts("4.000000")));
    // A page that stops short closes only what it covered.
    let fx = reduce(
        &mut st,
        Input::GapPageLoaded {
            conversation: cid("c1"),
            after: Some(ts("1.000000")),
            before: ts("9.000000"),
            page: HistoryPage {
                messages: vec![msg("c1", "6.000000", "x")],
                has_more: true,
            },
        },
    );
    let g = closed(&fx);
    assert_eq!((g.after, g.before), (ts("6.000000"), ts("9.000000")));
}
