//! Starred conversations: Parla's own, per workspace, persisted, no provider call.

use std::collections::HashSet;

use domain::test_support::cid;
use domain::{Changed, Effect, Input, PersistOp, WorkspaceState, reduce};
use settings::WorkspaceKey;

fn state() -> WorkspaceState {
    WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 100)
}

fn changed(fx: &[Effect]) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Changed(Changed::Conversations)))
}

#[test]
fn star_toggle_applies_persists_and_sends_no_call() {
    let mut st = state();
    let on = Input::SetStarred {
        conversation: cid("c1"),
        starred: true,
    };
    let fx = reduce(&mut st, on.clone());
    assert!(st.starred.contains(&cid("c1")));
    assert!(changed(&fx));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Persist(PersistOp::LocalStarredPut { starred: true, .. })
    )));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Call(_))));
    // The same write again changes nothing.
    let fx = reduce(&mut st, on);
    assert!(fx.is_empty());
    // Unstar removes it and persists.
    let fx = reduce(
        &mut st,
        Input::SetStarred {
            conversation: cid("c1"),
            starred: false,
        },
    );
    assert!(st.starred.is_empty());
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Persist(PersistOp::LocalStarredPut { starred: false, .. })
    )));
}

#[test]
fn loaded_stars_fill_state() {
    let mut st = state();
    let all = HashSet::from([cid("c1"), cid("c2")]);
    let fx = reduce(&mut st, Input::StarredLoaded(all.clone()));
    assert_eq!(st.starred, all);
    assert!(changed(&fx));
    // Loading the same set again is silent.
    assert!(reduce(&mut st, Input::StarredLoaded(all)).is_empty());
}

#[test]
fn stars_changed_before_the_load_survive_it() {
    let mut st = state();
    // The user stars c1 and unstars c2 before the stored set arrives.
    reduce(
        &mut st,
        Input::SetStarred {
            conversation: cid("c1"),
            starred: true,
        },
    );
    reduce(
        &mut st,
        Input::SetStarred {
            conversation: cid("c2"),
            starred: false,
        },
    );
    reduce(
        &mut st,
        Input::StarredLoaded(HashSet::from([cid("c2"), cid("c3")])),
    );
    assert_eq!(st.starred, HashSet::from([cid("c1"), cid("c3")]));
}
