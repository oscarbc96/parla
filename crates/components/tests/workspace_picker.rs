//! The native workspace picker's state: which rows start ticked, what can be
//! ticked, and how each row's progress is tracked while workspaces are added.

use components::workspace_picker::{AddState, Phase, WorkspaceChoice, WorkspacePickerModel};

fn choice(
    id: &str,
    added: bool,
    signed_out: bool,
    sso: bool,
    two_factor: bool,
    needs_sign_in: bool,
) -> WorkspaceChoice {
    WorkspaceChoice {
        id: id.into(),
        name: format!("{id} Workspace"),
        domain: format!("{id}.example"),
        icon: None,
        sso,
        two_factor,
        signed_out,
        added,
        updated: false,
        updating: false,
        update_error: false,
        needs_sign_in,
    }
}

fn model() -> WorkspacePickerModel {
    WorkspacePickerModel::new(vec![
        choice("T1", false, false, false, false, false),
        choice("T2", true, false, false, false, false),
        choice("T3", false, true, false, false, false),
        choice("T4", false, false, true, true, false),
    ])
}

#[test]
fn starts_with_the_workspaces_that_are_not_added_and_not_signed_out() {
    let m = model();
    assert_eq!(m.selected_ids(), ["T1", "T4"]);
    assert!(m.is_selected("T1"));
    assert!(!m.is_selected("T2"));
    assert!(!m.is_selected("T3"));
    assert_eq!(m.phase(), Phase::Choosing);
}

#[test]
fn an_added_workspace_cannot_be_ticked_but_the_others_can() {
    let mut m = model();
    m.toggle("T2");
    assert!(!m.is_selected("T2"), "an added workspace stays unticked");
    assert!(!m.selectable("T2"));
    m.toggle("T3");
    assert!(m.is_selected("T3"));
    m.toggle("T3");
    assert!(!m.is_selected("T3"));
}

#[test]
fn add_starts_only_the_ticked_rows_and_tracks_each_state() {
    let mut m = model();
    assert_eq!(m.begin_add(), ["T1", "T4"]);
    assert_eq!(m.phase(), Phase::Adding);
    assert_eq!(m.state("T1"), Some(AddState::Waiting));
    assert_eq!(m.state("T4"), Some(AddState::Waiting));

    m.progress("T1", AddState::SigningIn);
    m.progress("T3", AddState::Done);
    assert_eq!(
        m.state("T1"),
        Some(AddState::SigningIn),
        "an unticked row ignores progress"
    );
    assert_eq!(m.state("T3"), Some(AddState::Waiting));

    m.progress("T1", AddState::Done);
    m.progress("T4", AddState::NeedsUser);
    assert_eq!(m.state("T4"), Some(AddState::NeedsUser));
    m.progress("T4", AddState::Done);
    m.finish_add(&["T1".into(), "T4".into()]);
    assert_eq!(m.phase(), Phase::Choosing);
    assert_eq!(m.state("T1"), Some(AddState::Done));
    assert_eq!(m.state("T4"), Some(AddState::Done));
}

#[test]
fn rows_left_unfinished_are_failed_when_the_run_ends_early() {
    let mut m = model();
    m.begin_add();
    m.progress("T1", AddState::Done);
    m.finish_add(&["T1".into()]);
    assert_eq!(m.state("T1"), Some(AddState::Done));
    assert_eq!(m.state("T4"), Some(AddState::Failed));
}

#[test]
fn nothing_ticked_means_no_add() {
    let mut m = model();
    m.set_selected("T1", false);
    m.set_selected("T4", false);
    assert!(m.begin_add().is_empty());
    assert_eq!(m.phase(), Phase::Choosing);
}

#[test]
fn progress_is_ignored_while_choosing() {
    let mut m = model();
    m.progress("T1", AddState::Done);
    assert_eq!(m.state("T1"), Some(AddState::Waiting));
}

#[test]
fn skipping_the_waiting_row_marks_it_skipped_and_leaves_the_rest() {
    let mut m = model();
    m.begin_add();
    assert!(!m.skip_waiting(), "nobody waits yet");
    m.progress("T1", AddState::NeedsUser);
    assert_eq!(m.needs_user(), Some("T1 Workspace"));
    assert!(m.skip_waiting());
    assert_eq!(m.state("T1"), Some(AddState::Skipped));
    assert_eq!(m.needs_user(), None);
    m.progress("T4", AddState::Done);
    assert!(m.has_unfinished(), "the skipped row did not finish");
    m.finish_add(&["T4".to_string()]);
    assert_eq!(m.state("T1"), Some(AddState::Skipped));
}

#[test]
fn all_done_has_nothing_unfinished() {
    let mut m = WorkspacePickerModel::new(vec![choice("T1", false, false, false, false, false)]);
    m.begin_add();
    m.progress("T1", AddState::Done);
    assert!(!m.has_unfinished());
}

#[test]
fn an_added_workspace_that_needs_sign_in_can_be_ticked() {
    let mut m = WorkspacePickerModel::new(vec![choice("T1", true, false, false, false, true)]);
    assert!(m.selectable("T1"));
    assert!(!m.is_selected("T1"));
    m.toggle("T1");
    assert!(m.is_selected("T1"));
    assert_eq!(m.begin_add(), ["T1"]);
    m.progress("T1", AddState::SigningIn);
    m.progress("T1", AddState::NeedsUser);
    assert_eq!(m.state("T1"), Some(AddState::NeedsUser));
    assert_eq!(m.needs_user(), Some("T1 Workspace"));
}

#[test]
fn finishing_add_clears_needs_sign_in_on_an_added_row() {
    let mut m = WorkspacePickerModel::new(vec![choice("T1", true, false, false, false, true)]);
    assert!(m.selectable("T1"));
    m.toggle("T1");
    assert!(m.is_selected("T1"));
    assert_eq!(m.begin_add(), ["T1"]);
    m.progress("T1", AddState::SigningIn);
    m.progress("T1", AddState::NeedsUser);
    m.finish_add(&["T1".into()]);
    assert!(!m.workspaces()[0].needs_sign_in);
    assert!(m.workspaces()[0].updated);
    assert_eq!(m.state("T1"), Some(AddState::Done));
}

#[test]
fn updating_state_transitions_for_an_added_workspace() {
    let mut row = choice("T1", true, false, false, false, false);
    row.updating = true;
    let mut m = WorkspacePickerModel::new(vec![row]);
    assert!(m.workspaces()[0].updating);
    m.refresh_updated("T1");
    assert!(!m.workspaces()[0].updating);
    assert!(m.workspaces()[0].updated);
    assert!(!m.workspaces()[0].needs_sign_in);
    assert!(!m.workspaces()[0].update_error);

    let mut row = choice("T2", true, false, false, false, false);
    row.updating = true;
    let mut m = WorkspacePickerModel::new(vec![row]);
    m.refresh_needs_user("T2");
    assert!(!m.workspaces()[0].updating);
    assert!(m.workspaces()[0].needs_sign_in);
    assert!(!m.workspaces()[0].updated);
    assert!(m.selectable("T2"));

    let mut row = choice("T3", true, false, false, false, false);
    row.updating = true;
    let mut m = WorkspacePickerModel::new(vec![row]);
    m.refresh_failed("T3");
    assert!(!m.workspaces()[0].updating);
    assert!(m.workspaces()[0].update_error);
    assert!(!m.workspaces()[0].updated);
}

#[gpui::test]
fn cancel_mid_refresh_reports_the_keys_already_saved(cx: &mut gpui::TestAppContext) {
    use components::workspace_picker::{
        AddStep, RefreshOutcome, RefreshStep, WorkspaceAdder, WorkspacePicker, WorkspacePickerEvent,
    };
    use futures::channel::mpsc::UnboundedSender;
    use gpui::{App, Task, Window};
    use settings::WorkspaceKey;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Adder {
        held: Rc<RefCell<Option<UnboundedSender<RefreshStep>>>>,
    }
    impl WorkspaceAdder for Adder {
        fn refresh(
            &self,
            ids: Vec<String>,
            progress: UnboundedSender<RefreshStep>,
            _window: &mut Window,
            _cx: &mut App,
        ) {
            let _ = progress.unbounded_send(RefreshStep {
                id: ids[0].clone(),
                outcome: RefreshOutcome::Updated(WorkspaceKey::new("test", &ids[0])),
            });
            // Hold the sender so the refresh stays in progress for the rest.
            *self.held.borrow_mut() = Some(progress);
        }
        fn add(
            &self,
            _ids: Vec<String>,
            _progress: UnboundedSender<AddStep>,
            _window: &mut Window,
            _cx: &mut App,
        ) -> Task<Result<Vec<components::sign_in::Candidate>, components::sign_in::SignInError>>
        {
            Task::ready(Ok(Vec::new()))
        }
    }

    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });

    let held = Rc::new(RefCell::new(None));
    let adder = Rc::new(Adder { held: held.clone() });
    let rows = vec![
        WorkspaceChoice {
            id: "T1".into(),
            name: "T1 Workspace".into(),
            domain: "one".into(),
            icon: None,
            sso: false,
            two_factor: false,
            signed_out: false,
            added: true,
            updated: false,
            updating: true,
            update_error: false,
            needs_sign_in: false,
        },
        WorkspaceChoice {
            id: "T2".into(),
            name: "T2 Workspace".into(),
            domain: "two".into(),
            icon: None,
            sso: false,
            two_factor: false,
            signed_out: false,
            added: true,
            updated: false,
            updating: true,
            update_error: false,
            needs_sign_in: false,
        },
    ];
    let (view, cx) = cx.add_window_view(|_, cx| {
        WorkspacePicker::new(
            WorkspacePickerModel::new(rows),
            adder,
            Vec::new(),
            vec!["T1".into(), "T2".into()],
            cx,
        )
    });
    let events: Rc<RefCell<Vec<WorkspacePickerEvent>>> = Rc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &WorkspacePickerEvent, _| {
            sink.borrow_mut().push(e.clone());
        })
        .detach();
    });

    // Render once to start the refresh and process the first outcome.
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();

    cx.update(|_, cx| {
        view.update(cx, |picker, cx| picker.cancel(cx));
    });

    assert_eq!(
        *events.borrow(),
        vec![WorkspacePickerEvent::Added(vec![WorkspaceKey::new(
            "test", "T1"
        )])],
        "the key saved before cancel is reported"
    );
}

#[gpui::test]
fn escape_after_a_partial_add_closes_and_keeps_the_added_workspace(cx: &mut gpui::TestAppContext) {
    use components::sign_in::{Candidate, SignInError};
    use components::workspace_picker::{
        AddStep, WorkspaceAdder, WorkspacePicker, WorkspacePickerEvent,
    };
    use futures::channel::mpsc::UnboundedSender;
    use gpui::{App, Task, Window};
    use settings::WorkspaceKey;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Partial;
    impl WorkspaceAdder for Partial {
        fn add(
            &self,
            _ids: Vec<String>,
            progress: UnboundedSender<AddStep>,
            _window: &mut Window,
            _cx: &mut App,
        ) -> Task<Result<Vec<Candidate>, SignInError>> {
            let _ = progress.unbounded_send(AddStep {
                id: "T1".into(),
                state: AddState::Done,
            });
            let _ = progress.unbounded_send(AddStep {
                id: "T2".into(),
                state: AddState::Failed,
            });
            Task::ready(Ok(vec![Candidate {
                key: WorkspaceKey::new("test", "T1"),
                name: "T1 Workspace".into(),
                ready: true,
            }]))
        }
    }

    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let rows = vec![
        choice("T1", false, false, false, false, false),
        choice("T2", false, false, false, false, false),
    ];
    let (view, cx) = cx.add_window_view(|_, cx| {
        WorkspacePicker::new(
            WorkspacePickerModel::new(rows),
            Rc::new(Partial),
            Vec::new(),
            Vec::new(),
            cx,
        )
    });
    let events: Rc<RefCell<Vec<WorkspacePickerEvent>>> = Rc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &WorkspacePickerEvent, _| {
            sink.borrow_mut().push(e.clone());
        })
        .detach();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    view.update_in(cx, |p, window, cx| p.add(window, cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("workspace-picker-close").is_some(),
        "a partial add shows Close"
    );
    cx.simulate_keystrokes("escape");
    let events = events.borrow();
    assert!(
        matches!(events.as_slice(), [WorkspacePickerEvent::Added(keys)] if keys.len() == 1),
        "{events:?}"
    );
}
