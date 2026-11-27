//! The provider-neutral sign-in screen against a fake `SignIn`; no webview, no network.

use std::cell::RefCell;
use std::rc::Rc;

use components::sign_in::{
    Candidate, SignIn, SignInError, SignInEvent, SignInField, SignInOption, SignInResult,
    SignInScreen, SignInText, Step, WorkspacePickerSession,
};
use components::workspace_picker::{
    AddStep, RefreshOutcome, RefreshStep, WorkspaceAdder, WorkspaceChoice,
};
use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Task, TestAppContext, Window};
use settings::WorkspaceKey;

type Calls = Rc<RefCell<Vec<(String, Vec<(String, String)>)>>>;

struct Fake {
    calls: Calls,
    result: RefCell<Option<Result<Vec<Candidate>, SignInError>>>,
    session: RefCell<Option<WorkspacePickerSession>>,
    /// Progress to report, then the run never completes.
    hang_after_progress: RefCell<Vec<RefreshStep>>,
}

impl Fake {
    fn new(result: Result<Vec<Candidate>, SignInError>) -> (Rc<Fake>, Calls) {
        let calls: Calls = Rc::default();
        (
            Rc::new(Fake {
                calls: calls.clone(),
                result: RefCell::new(Some(result)),
                session: RefCell::new(None),
                hang_after_progress: RefCell::new(Vec::new()),
            }),
            calls,
        )
    }
}

impl SignIn for Fake {
    fn provider_name(&self) -> String {
        "Fake".into()
    }
    fn notice(&self) -> Vec<SignInText> {
        vec!["Unofficial client.".into(), "Uses your own session.".into()]
    }
    fn options(&self) -> Vec<SignInOption> {
        vec![
            SignInOption {
                id: "window".into(),
                label: "Sign in with browser window".into(),
                fields: vec![],
                help: None,
                copy: None,
            },
            SignInOption {
                id: "paste".into(),
                label: "Paste token and cookie".into(),
                fields: vec![
                    SignInField {
                        id: "cookie".into(),
                        label: "Cookie".into(),
                        secret: true,
                    },
                    SignInField {
                        id: "tokens".into(),
                        label: "Tokens".into(),
                        secret: true,
                    },
                ],
                help: Some("Run this in the console".into()),
                copy: Some("copy(thing)".into()),
            },
        ]
    }
    fn run(
        &self,
        option: &str,
        values: Vec<(String, String)>,
        progress: UnboundedSender<RefreshStep>,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        self.calls.borrow_mut().push((option.to_string(), values));
        let reported = std::mem::take(&mut *self.hang_after_progress.borrow_mut());
        if !reported.is_empty() {
            for step in reported {
                let _ = progress.unbounded_send(step);
            }
            return cx.spawn(|cx: &mut gpui::AsyncApp| {
                let executor = cx.background_executor().clone();
                async move {
                    loop {
                        executor.timer(std::time::Duration::from_secs(3600)).await;
                    }
                }
            });
        }
        if let Some(session) = self.session.borrow_mut().take() {
            return Task::ready(Ok(SignInResult::Pick(session)));
        }
        Task::ready(
            self.result
                .borrow_mut()
                .take()
                .expect("one run")
                .map(SignInResult::Workspaces),
        )
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        Task::ready(Ok(()))
    }
}

fn cand(id: &str, ready: bool) -> Candidate {
    Candidate {
        key: WorkspaceKey::new("fake", id),
        name: id.into(),
        ready,
    }
}

fn setup(
    cx: &mut TestAppContext,
    fake: Rc<Fake>,
) -> (
    gpui::Entity<SignInScreen>,
    Rc<RefCell<Vec<SignInEvent>>>,
    &mut gpui::VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (view, cx) = cx.add_window_view(|_, cx| SignInScreen::new(fake, cx));
    let events: Rc<RefCell<Vec<SignInEvent>>> = Rc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &SignInEvent, _| {
            sink.borrow_mut().push(ev.clone())
        })
        .detach()
    });
    (view, events, cx)
}

#[gpui::test]
async fn capture_blocked_until_consent(cx: &mut TestAppContext) {
    let (fake, calls) = Fake::new(Ok(vec![cand("w1", true)]));
    let (view, events, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| v.choose("window", window, cx));
    cx.run_until_parked();
    assert!(calls.borrow().is_empty());
    assert_eq!(view.read_with(cx, |v, _| v.step().clone()), Step::Options);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(events.borrow().len(), 1);
}

#[gpui::test]
async fn consent_shows_notice_verbatim(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Ok(vec![]));
    let (view, _, cx) = setup(cx, fake);
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.notice(),
            [
                SignInText::from("Unofficial client."),
                SignInText::from("Uses your own session.")
            ]
        );
        assert!(!v.consented());
    });
}

#[gpui::test]
async fn paste_form_submits_values_and_single_ready_workspace_signs_in(cx: &mut TestAppContext) {
    let (fake, calls) = Fake::new(Ok(vec![cand("w1", true), cand("w2", false)]));
    let (view, events, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("paste", window, cx);
        v.set_field("cookie", "c-value", cx);
        v.set_field("tokens", "t-value", cx);
        v.submit(window, cx);
    });
    cx.run_until_parked();
    let calls = calls.borrow();
    assert_eq!(calls[0].0, "paste");
    assert!(calls[0].1.contains(&("cookie".into(), "c-value".into())));
    assert_eq!(
        *events.borrow(),
        vec![SignInEvent::SignedIn(WorkspaceKey::new("fake", "w1"))]
    );
}

#[gpui::test]
async fn several_ready_workspaces_wait_for_a_pick(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Ok(vec![cand("w1", true), cand("w2", true)]));
    let (view, events, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    assert!(matches!(
        view.read_with(cx, |v, _| v.step().clone()),
        Step::Pick(c) if c.len() == 2
    ));
    assert!(events.borrow().is_empty());
    view.update(cx, |v, cx| v.pick(&WorkspaceKey::new("fake", "w2"), cx));
    assert_eq!(
        *events.borrow(),
        vec![SignInEvent::Added(vec![
            WorkspaceKey::new("fake", "w1"),
            WorkspaceKey::new("fake", "w2")
        ])]
    );
}

struct NoAdder;

impl WorkspaceAdder for NoAdder {
    fn add(
        &self,
        _: Vec<String>,
        _: UnboundedSender<AddStep>,
        _: &mut Window,
        _: &mut App,
    ) -> Task<Result<Vec<Candidate>, SignInError>> {
        Task::ready(Ok(Vec::new()))
    }
}

fn choice(id: &str, added: bool, updated: bool) -> WorkspaceChoice {
    WorkspaceChoice {
        id: id.into(),
        name: id.into(),
        domain: id.into(),
        icon: None,
        sso: false,
        two_factor: false,
        signed_out: false,
        added,
        updated,
        updating: false,
        update_error: false,
        needs_sign_in: false,
    }
}

#[gpui::test]
async fn cancelling_the_picker_still_restarts_the_refreshed_workspace(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Ok(vec![]));
    *fake.session.borrow_mut() = Some(WorkspacePickerSession {
        choices: vec![choice("w1", true, true), choice("w2", false, false)],
        adder: Rc::new(NoAdder),
        refreshed: vec![WorkspaceKey::new("fake", "w1")],
        refresh: Vec::new(),
    });
    let (view, events, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    let Step::PickWorkspaces(picker) = view.read_with(cx, |v, _| v.step().clone()) else {
        panic!("the picker is open");
    };
    picker.update(cx, |p, cx| p.cancel(cx));
    // The expired workspace is restarted; nothing new was added.
    assert_eq!(
        *events.borrow(),
        vec![SignInEvent::Added(vec![WorkspaceKey::new("fake", "w1")])]
    );
}

#[gpui::test]
async fn progress_of_one_key_then_cancel_emits_that_key(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Ok(vec![]));
    fake.hang_after_progress.borrow_mut().push(RefreshStep {
        id: "w1".into(),
        outcome: RefreshOutcome::Updated(WorkspaceKey::new("fake", "w1")),
    });
    let (view, events, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.step().clone()), Step::Waiting);
    view.update(cx, |v, cx| v.cancel(cx));
    assert_eq!(
        *events.borrow(),
        vec![SignInEvent::Added(vec![WorkspaceKey::new("fake", "w1")])]
    );
}

#[gpui::test]
async fn restart_with_two_or_more_keys_emits_added_without_pick(cx: &mut TestAppContext) {
    struct RestartFake;
    impl SignIn for RestartFake {
        fn provider_name(&self) -> String {
            "Fake".into()
        }
        fn notice(&self) -> Vec<SignInText> {
            vec![]
        }
        fn options(&self) -> Vec<SignInOption> {
            vec![SignInOption {
                id: "window".into(),
                label: "Sign in with browser window".into(),
                fields: vec![],
                help: None,
                copy: None,
            }]
        }
        fn run(
            &self,
            _: &str,
            _: Vec<(String, String)>,
            _: UnboundedSender<RefreshStep>,
            _: &mut Window,
            _: &mut App,
        ) -> Task<Result<SignInResult, SignInError>> {
            Task::ready(Ok(SignInResult::Restart(vec![
                cand("w1", true),
                cand("w2", true),
                cand("w3", false),
            ])))
        }
        fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
            Task::ready(Ok(()))
        }
    }
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (view, cx) = cx.add_window_view(|_, cx| SignInScreen::new(Rc::new(RestartFake), cx));
    let events: Rc<RefCell<Vec<SignInEvent>>> = Rc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &SignInEvent, _| {
            sink.borrow_mut().push(ev.clone())
        })
        .detach()
    });
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.step().clone()), Step::Options);
    assert_eq!(
        *events.borrow(),
        vec![SignInEvent::Added(vec![
            WorkspaceKey::new("fake", "w1"),
            WorkspaceKey::new("fake", "w2"),
        ])]
    );
}

#[gpui::test]
async fn failure_shows_error_and_returns_to_options(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Err(SignInError::Failed("token was rejected".into())));
    let (view, events, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.error(), Some(&SignInText::from("token was rejected")));
        assert_eq!(*v.step(), Step::Options);
    });
    assert!(events.borrow().is_empty());
}

#[gpui::test]
async fn disallowed_user_agent_switches_to_manual(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Err(SignInError::SwitchTo {
        option: "paste".into(),
        reason: "The sign-in window was refused.".into(),
    }));
    let (view, _, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(*v.step(), Step::Form("paste".into()));
        assert_eq!(
            v.error(),
            Some(&SignInText::from("The sign-in window was refused."))
        );
    });
}

#[gpui::test]
async fn no_verified_workspace_is_an_error(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Ok(vec![cand("w1", false)]));
    let (view, events, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.error().is_some()));
    assert!(events.borrow().is_empty());
}

#[test]
fn overlay_starts_below_inset_and_header() {
    use components::sign_in::{OVERLAY_TOP, overlay_top};
    let top = OVERLAY_TOP.as_f32();
    assert_eq!(overlay_top(0.0), top);
    assert_eq!(overlay_top(32.0), 32.0 + top);
    assert_eq!(overlay_top(-4.0), top);
}

#[gpui::test]
async fn sign_in_in_catalan(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Ok(vec![cand("w1", true)]));
    let (view, _, cx) = setup(cx, fake);
    cx.update(|_, cx| {
        let ca: i18n::LanguageIdentifier = "ca".parse().unwrap();
        assert!(i18n::gpui::set_pref_with_os(
            cx,
            i18n::LocalePref::Code(ca),
            &[]
        ));
    });
    // Repaints every step in Catalan without a missing message.
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let mut text = |id: &str| {
        cx.update(|_, cx| {
            i18n::gpui::localizer(cx)
                .format("components", id, None)
                .into_owned()
        })
    };
    assert_eq!(text("sign-in-submit"), "Inicia la sessió");
    assert_eq!(text("sign-in-back"), "Enrere");
    assert_eq!(text("sign-in-waiting"), "Esperant l'inici de sessió...");
}

#[gpui::test]
async fn localized_error_follows_the_language(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Err(SignInError::Failed(
        i18n::LocalizedText::new("components", "sign-in-no-workspace").into(),
    )));
    let (view, _, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    let shown = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_, cx| {
            let l = i18n::gpui::localizer(cx);
            view.read(cx).error().unwrap().resolve(&l)
        })
    };
    let en = shown(cx);
    cx.update(|_, cx| i18n::gpui::set_pref(cx, i18n::LocalePref::Code("ca".parse().unwrap())));
    let ca = shown(cx);
    assert_eq!(en, "No workspace could be verified.");
    assert_ne!(en, ca);
}

#[gpui::test]
async fn typed_values_are_dropped_after_sign_in_and_on_cancel(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Ok(vec![cand("w1", true)]));
    let (view, _, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("paste", window, cx);
        v.set_field("cookie", "c-value", cx);
        v.submit(window, cx);
    });
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, cx| v.field_value("cookie", cx)), "");
    view.update_in(cx, |v, window, cx| {
        v.choose("paste", window, cx);
        v.set_field("cookie", "again", cx);
        v.cancel(cx);
    });
    assert_eq!(view.read_with(cx, |v, cx| v.field_value("cookie", cx)), "");
}

#[gpui::test]
async fn typing_goes_to_the_active_field_and_tab_moves_on(cx: &mut TestAppContext) {
    let (fake, calls) = Fake::new(Ok(vec![cand("w1", true)]));
    let (view, _, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("paste", window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("abc");
    cx.simulate_keystrokes("left backspace tab");
    cx.simulate_input("t\u{301}x");
    cx.simulate_keystrokes("shift-tab");
    cx.simulate_input("!");
    assert_eq!(
        view.read_with(cx, |v, cx| v.field_value("cookie", cx)),
        "a!c"
    );
    assert_eq!(
        view.read_with(cx, |v, cx| v.field_value("tokens", cx)),
        "t\u{301}x"
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let calls = calls.borrow();
    assert!(calls[0].1.contains(&("cookie".into(), "a!c".into())));
}

#[gpui::test]
async fn switching_to_a_form_after_a_failure_gives_typable_fields(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Err(SignInError::SwitchTo {
        option: "paste".into(),
        reason: "refused".into(),
    }));
    let (view, _, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("window", window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("typed");
    assert_eq!(
        view.read_with(cx, |v, cx| v.field_value("cookie", cx)),
        "typed"
    );
}

async fn open_paste(
    cx: &mut TestAppContext,
) -> (
    gpui::Entity<SignInScreen>,
    Calls,
    &mut gpui::VisualTestContext,
) {
    let (fake, calls) = Fake::new(Ok(vec![cand("w1", true)]));
    let (view, _, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        v.set_consent(true, cx);
        v.choose("paste", window, cx)
    });
    cx.run_until_parked();
    (view, calls, cx)
}

#[gpui::test]
async fn form_tab_order_ends_on_back_then_submit(cx: &mut TestAppContext) {
    // cookie, tokens, Back: Enter on Back leaves the form.
    let (view, _, cx) = open_paste(cx).await;
    cx.simulate_keystrokes("tab tab enter");
    assert_eq!(view.read_with(cx, |v, _| v.step().clone()), Step::Options);
    // cookie, tokens, Back, Submit: Enter on Submit runs the sign-in.
    let (_, calls, cx) = open_paste(cx).await;
    cx.simulate_keystrokes("tab tab tab enter");
    cx.run_until_parked();
    assert_eq!(calls.borrow().len(), 1);
    // Shift-Tab from the first field wraps to Submit; Tab wraps back to the first field.
    let (_, calls, cx) = open_paste(cx).await;
    cx.simulate_keystrokes("shift-tab tab");
    cx.simulate_input("x");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(calls.borrow()[0].1.contains(&("cookie".into(), "x".into())));
}

#[gpui::test]
async fn escape_leaves_the_form(cx: &mut TestAppContext) {
    let (view, _, cx) = open_paste(cx).await;
    cx.simulate_keystrokes("escape");
    assert_eq!(view.read_with(cx, |v, _| v.step().clone()), Step::Options);
}

#[gpui::test]
async fn options_are_reachable_with_tab_and_enter(cx: &mut TestAppContext) {
    let (fake, calls) = Fake::new(Ok(vec![cand("w1", true)]));
    let (view, _, cx) = setup(cx, fake);
    view.update_in(cx, |v, window, cx| {
        let focus = gpui::Focusable::focus_handle(v, cx);
        window.focus(&focus, cx);
    });
    // The consent check box is the first stop; the options follow it.
    cx.simulate_keystrokes("tab enter");
    assert!(view.read_with(cx, |v, _| v.consented()));
    cx.simulate_keystrokes("tab enter");
    cx.run_until_parked();
    assert_eq!(calls.borrow().len(), 1);
}

#[gpui::test]
async fn copy_is_reachable_by_tab(cx: &mut TestAppContext) {
    // The form opens on the first field; Shift-Tab goes to Copy, which Enter presses.
    let (_, _, cx) = open_paste(cx).await;
    cx.simulate_keystrokes("shift-tab enter");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()),
        Some("copy(thing)".to_string())
    );
}

#[gpui::test]
async fn small_window_scrolls_the_card_and_keeps_the_footer(cx: &mut TestAppContext) {
    let (fake, _) = Fake::new(Ok(vec![]));
    let (_view, _, cx) = setup(cx, fake);
    cx.simulate_resize(gpui::size(gpui::px(440.0), gpui::px(200.0)));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let win = gpui::px(200.0);
    let footer = cx.debug_bounds("sign-in-footer").expect("footer drawn");
    let scroll = cx
        .debug_bounds("sign-in-scroll")
        .expect("scroll area drawn");
    assert!(footer.bottom() <= win, "footer cut off: {:?}", footer);
    assert!(
        scroll.bottom() <= footer.top(),
        "card area {:?} overlaps footer {:?}",
        scroll,
        footer
    );
    // Scroll to the end: the primary button is then inside the card area.
    cx.simulate_event(gpui::ScrollEvent {
        position: scroll.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-2000.0))),
        source: gpui::ScrollSource::Pointer,
        ..Default::default()
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let scroll = cx.debug_bounds("sign-in-scroll").unwrap();
    let button = cx.debug_bounds("sign-in-opt-window").expect("button drawn");
    assert!(
        button.top() >= scroll.top() && button.bottom() <= scroll.bottom(),
        "primary button {:?} not inside the card area {:?}",
        button,
        scroll
    );
    assert!(
        button.bottom() <= footer.top(),
        "button overlaps the footer"
    );
}
