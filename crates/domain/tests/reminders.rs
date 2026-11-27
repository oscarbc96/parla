use std::sync::Arc;

use domain::test_support::uid;
use domain::{
    Bootstrap, CallOutput, Changed, Effect, Input, Load, Loaded, ProviderCall, ProviderError,
    ProviderEvent, Reminder, ReminderId, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![],
            members: vec![],
        }),
    );
    // These tests cover the provider path; without the capability Parla keeps
    // reminders itself (`reminders_local.rs`).
    s.caps.reminders = true;
    s
}

fn reminder(id: &str, completed: bool) -> Reminder {
    Reminder {
        id: ReminderId(Arc::from(id)),
        text: Arc::from(id),
        due: None,
        recurring: false,
        completed,
        fired: false,
        interval_secs: 0,
        for_user: uid("me"),
        link: None,
        snoozed_until: None,
    }
}

fn call(fx: &[Effect]) -> Option<ProviderCall> {
    fx.iter().find_map(|e| match e {
        Effect::Call(c) => Some(c.clone()),
        _ => None,
    })
}

fn changed(fx: &[Effect]) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Changed(Changed::Reminders)))
}

#[test]
fn add_emits_call() {
    let mut s = state();
    let text: Arc<str> = Arc::from("stretch");
    let when: Arc<str> = Arc::from("in 20 minutes");
    let fx = reduce(
        &mut s,
        Input::AddReminder {
            text: text.clone(),
            when: when.clone(),
            user: Some(uid("U1")),
        },
    );
    match call(&fx).expect("call") {
        ProviderCall::AddReminder {
            text: t,
            when: w,
            user,
        } => {
            assert_eq!(t, text);
            assert_eq!(w, when);
            assert_eq!(user, Some(uid("U1")));
        }
        other => panic!("{other:?}"),
    }
    assert!(s.reminders.is_none());

    // The provider answers with the reminder it stored, which is now listed.
    let r = reminder("rem-1", false);
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::AddReminder {
                text,
                when,
                user: Some(uid("U1")),
            },
            result: Ok(CallOutput::Reminder(r.clone())),
        },
    );
    assert_eq!(s.reminders.as_deref(), Some(std::slice::from_ref(&r)));
    assert!(changed(&fx));
}

#[test]
fn complete_optimistic_and_rollback() {
    let mut s = state();
    let r = reminder("rem-1", false);
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Reminders(vec![r.clone()]))),
    );

    let fx = reduce(&mut s, Input::CompleteReminder(r.id.clone()));
    assert!(s.reminders.as_ref().unwrap()[0].completed);
    assert!(changed(&fx));
    assert!(matches!(
        call(&fx),
        Some(ProviderCall::CompleteReminder(id)) if id == r.id
    ));

    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::CompleteReminder(r.id),
            result: Err(ProviderError::Network("down".into())),
        },
    );
    assert!(!s.reminders.as_ref().unwrap()[0].completed);
    assert!(changed(&fx));
}

#[test]
fn delete_optimistic_and_rollback() {
    let mut s = state();
    let one = reminder("rem-1", false);
    let two = reminder("rem-2", false);
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Reminders(vec![one.clone(), two.clone()]))),
    );

    let fx = reduce(&mut s, Input::DeleteReminder(two.id.clone()));
    assert_eq!(
        s.reminders
            .as_ref()
            .unwrap()
            .iter()
            .map(|r| &r.id)
            .collect::<Vec<_>>(),
        vec![&one.id]
    );
    assert!(changed(&fx));
    assert!(matches!(
        call(&fx),
        Some(ProviderCall::DeleteReminder(id)) if id == two.id
    ));

    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::DeleteReminder(two.id.clone()),
            result: Err(ProviderError::Network("down".into())),
        },
    );
    assert_eq!(
        s.reminders.as_ref().unwrap(),
        &vec![one.clone(), two.clone()]
    );
    assert!(changed(&fx));

    // An accepted delete keeps nothing for undo: a later refusal of the same
    // id (a stale answer) must not resurrect it.
    reduce(&mut s, Input::DeleteReminder(two.id.clone()));
    reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::DeleteReminder(two.id.clone()),
            result: Ok(CallOutput::None),
        },
    );
    reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::DeleteReminder(two.id),
            result: Err(ProviderError::Network("late".into())),
        },
    );
    assert_eq!(s.reminders.as_ref().unwrap(), &vec![one]);
}

#[test]
fn changed_event_refetches() {
    let mut s = state();
    let fx = reduce(&mut s, Input::Provider(ProviderEvent::RemindersChanged));
    assert!(matches!(fx.as_slice(), [Effect::Load(Load::Reminders)]));

    // Opening the list loads it once; later opens use what is held.
    let mut s = state();
    let fx = reduce(&mut s, Input::OpenReminders);
    assert!(matches!(fx.as_slice(), [Effect::Load(Load::Reminders)]));

    let r = reminder("rem-1", false);
    let fx = reduce(&mut s, Input::Loaded(Ok(Loaded::Reminders(vec![r]))));
    assert!(changed(&fx));
    assert!(s.reminders.is_some());
    assert!(reduce(&mut s, Input::OpenReminders).is_empty());
}

fn reloads(fx: &[Effect]) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Load(Load::Reminders)))
}

#[test]
fn delete_of_a_gone_reminder_stays_deleted_and_reloads() {
    let mut s = state();
    let one = reminder("rem-1", false);
    let two = reminder("rem-2", false);
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Reminders(vec![one.clone(), two.clone()]))),
    );
    reduce(&mut s, Input::DeleteReminder(two.id.clone()));
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::DeleteReminder(two.id.clone()),
            result: Err(ProviderError::NotAllowed("not_found".into())),
        },
    );
    // Not resurrected, and the server's list is fetched to settle.
    assert_eq!(s.reminders.as_ref().unwrap(), &vec![one]);
    assert!(reloads(&fx));
    // The undo entry is gone: a stale refusal cannot bring it back either.
    reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::DeleteReminder(two.id),
            result: Err(ProviderError::Network("late".into())),
        },
    );
    assert_eq!(s.reminders.as_ref().unwrap().len(), 1);
}

#[test]
fn complete_of_a_gone_reminder_reloads() {
    let mut s = state();
    let r = reminder("rem-1", false);
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Reminders(vec![r.clone()]))),
    );
    reduce(&mut s, Input::CompleteReminder(r.id.clone()));
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: ProviderCall::CompleteReminder(r.id),
            result: Err(ProviderError::NotAllowed("not_found".into())),
        },
    );
    assert!(reloads(&fx));
}

#[test]
fn failed_add_reloads_once_so_a_stored_reminder_shows() {
    let mut s = state();
    let add = ProviderCall::AddReminder {
        text: Arc::from("x"),
        when: Arc::from("in 1 minute"),
        user: None,
    };
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: add,
            result: Err(ProviderError::Other(
                "reminders.add failed: no_reminder_in_response".into(),
            )),
        },
    );
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::Load(Load::Reminders)))
            .count(),
        1
    );
}
