//! First-run screens around the sign-in: Welcome, the provider picker, the
//! first-sync progress and the tour, against a fake `SignIn`.

use std::cell::RefCell;
use std::rc::Rc;

use components::onboarding::{
    Onboarding, OnboardingEvent, OnboardingStep, ProviderCard, SyncProgress, WorkspaceSync,
};
use components::sign_in::{
    SignIn, SignInError, SignInOption, SignInResult, SignInScreen, SignInText, Step,
};
use components::tour::{Tour, TourEvent, TourLayout};
use components::workspace_picker::RefreshStep;
use futures::channel::mpsc::UnboundedSender;
use gpui::{App, AppContext as _, Modifiers, Task, TestAppContext, VisualTestContext, Window};
use settings::WorkspaceKey;

struct Fake;

impl SignIn for Fake {
    fn provider_name(&self) -> String {
        "Example".into()
    }
    fn notice(&self) -> Vec<SignInText> {
        vec!["Unofficial client.".into()]
    }
    fn options(&self) -> Vec<SignInOption> {
        vec![SignInOption {
            id: "window".into(),
            label: "Sign in".into(),
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
        Task::ready(Ok(SignInResult::Workspaces(Vec::new())))
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        Task::ready(Ok(()))
    }
}

type Events = Rc<RefCell<Vec<OnboardingEvent>>>;

fn setup(cx: &mut TestAppContext) -> (gpui::Entity<Onboarding>, Events, &mut VisualTestContext) {
    setup_with(cx, Rc::new(Fake), false)
}

/// A provider whose sign-in never answers: the screen stays in `Step::Waiting`.
struct Waiting;

impl SignIn for Waiting {
    fn provider_name(&self) -> String {
        "Example".into()
    }
    fn notice(&self) -> Vec<SignInText> {
        vec![]
    }
    fn options(&self) -> Vec<SignInOption> {
        vec![SignInOption {
            id: "window".into(),
            label: "Sign in".into(),
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
        cx: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        cx.spawn(async move |_| futures::future::pending().await)
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        Task::ready(Ok(()))
    }
}

fn setup_with(
    cx: &mut TestAppContext,
    signin: Rc<dyn SignIn>,
    added: bool,
) -> (gpui::Entity<Onboarding>, Events, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_motion(cx, components::Motion::Frozen);
    });
    let (view, cx) = cx.add_window_view(|window, cx| {
        let screen = cx.new(|cx| SignInScreen::new(signin, cx));
        let providers = vec![ProviderCard {
            name: "Example".into(),
        }];
        let mut ob = Onboarding::new(providers, screen, cx);
        if added {
            ob = ob.for_added_workspace();
        }
        window.focus(&ob.focus_handle_for(cx), cx);
        ob
    });
    let events: Events = Rc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &OnboardingEvent, _| {
            sink.borrow_mut().push(ev.clone())
        })
        .detach()
    });
    (view, events, cx)
}

fn step(view: &gpui::Entity<Onboarding>, cx: &mut VisualTestContext) -> OnboardingStep {
    view.read_with(cx, |o, _| o.step())
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let at = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} painted"))
        .center();
    cx.simulate_click(at, Modifiers::none());
}

#[gpui::test]
async fn screens_come_in_order_welcome_provider_then_sign_in(cx: &mut TestAppContext) {
    let (view, events, cx) = setup(cx);
    assert_eq!(step(&view, cx), OnboardingStep::Welcome);
    click(cx, "onboarding-next");
    assert_eq!(step(&view, cx), OnboardingStep::Provider);
    click(cx, "onboarding-next");
    assert_eq!(step(&view, cx), OnboardingStep::SignIn);
    assert_eq!(*events.borrow(), vec![OnboardingEvent::ProviderChosen(0)]);
    // The consent screen of the provider's sign-in is what shows now.
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("sign-in-opt-window").is_some());
}

#[gpui::test]
async fn back_returns_one_screen(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    click(cx, "onboarding-next");
    click(cx, "onboarding-next");
    assert_eq!(step(&view, cx), OnboardingStep::SignIn);
    click(cx, "onboarding-back");
    assert_eq!(step(&view, cx), OnboardingStep::Provider);
    click(cx, "onboarding-back");
    assert_eq!(step(&view, cx), OnboardingStep::Welcome);
}

#[gpui::test]
async fn enter_goes_on_and_escape_goes_back(cx: &mut TestAppContext) {
    let (view, events, cx) = setup(cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(step(&view, cx), OnboardingStep::Provider);
    cx.simulate_keystrokes("escape");
    assert_eq!(step(&view, cx), OnboardingStep::Welcome);
    cx.simulate_keystrokes("enter enter");
    assert_eq!(step(&view, cx), OnboardingStep::SignIn);
    assert_eq!(*events.borrow(), vec![OnboardingEvent::ProviderChosen(0)]);
    // Escape on the consent screen goes back to the provider picker.
    cx.simulate_keystrokes("escape");
    assert_eq!(step(&view, cx), OnboardingStep::Provider);
}

#[gpui::test]
async fn first_sync_finishes_only_when_every_workspace_is_done(cx: &mut TestAppContext) {
    let (view, events, cx) = setup(cx);
    view.update(cx, |o, cx| o.start_first_sync(vec!["One".into()], cx));
    assert_eq!(step(&view, cx), OnboardingStep::FirstSync);
    let started = view.read_with(cx, |o, _| o.workspaces()[0].progress.percent());
    assert_eq!(started, 0);
    // Open Parla does nothing while the sync runs.
    cx.simulate_keystrokes("enter");
    assert!(events.borrow().is_empty());

    let ws = |progress: SyncProgress| {
        vec![WorkspaceSync {
            name: "One".into(),
            progress,
        }]
    };
    view.update(cx, |o, cx| {
        o.set_sync(
            ws(SyncProgress {
                store_ready: true,
                directory: Some((3, 2)),
                catch_up: Some((1, 4)),
                failed: false,
            }),
            cx,
        )
    });
    let mid = view.read_with(cx, |o, _| o.workspaces()[0].progress.percent());
    assert!(mid > started && mid < 100, "{mid}");
    assert!(!view.read_with(cx, |o, _| o.can_finish()));

    view.update(cx, |o, cx| {
        o.set_sync(
            ws(SyncProgress {
                store_ready: true,
                directory: Some((3, 2)),
                catch_up: Some((4, 4)),
                failed: false,
            }),
            cx,
        )
    });
    assert_eq!(
        view.read_with(cx, |o, _| o.workspaces()[0].progress.percent()),
        100
    );
    assert!(view.read_with(cx, |o, _| o.can_finish()));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("first-sync-open").is_some());
    cx.simulate_keystrokes("enter");
    assert_eq!(*events.borrow(), vec![OnboardingEvent::Finished]);
}

fn tour(cx: &mut TestAppContext) -> (gpui::Entity<Tour>, Rc<RefCell<u32>>, &mut VisualTestContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (view, cx) = cx.add_window_view(|window, cx| {
        let t = Tour::new(TourLayout::default(), cx);
        window.focus(&t.focus_handle_for(), cx);
        t
    });
    let dismissed = Rc::new(RefCell::new(0));
    let sink = dismissed.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &TourEvent, _| match ev {
            TourEvent::Dismissed => *sink.borrow_mut() += 1,
        })
        .detach()
    });
    (view, dismissed, cx)
}

#[gpui::test]
async fn tour_steps_through_its_tips_and_finishes(cx: &mut TestAppContext) {
    let (view, dismissed, cx) = tour(cx);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("tour-coach").is_some());
    let total = view.read_with(cx, |t, _| t.tips());
    assert!(total >= 2);
    click(cx, "tour-next");
    assert_eq!(view.read_with(cx, |t, _| t.tip()), 1);
    click(cx, "tour-back");
    assert_eq!(view.read_with(cx, |t, _| t.tip()), 0);
    for _ in 0..total {
        cx.simulate_keystrokes("enter");
    }
    assert_eq!(*dismissed.borrow(), 1, "the last Next ends the tour");
}

#[gpui::test]
async fn tour_can_be_skipped(cx: &mut TestAppContext) {
    let (_, dismissed, cx) = tour(cx);
    click(cx, "tour-skip");
    assert_eq!(*dismissed.borrow(), 1);
}

#[gpui::test]
async fn escape_skips_the_tour(cx: &mut TestAppContext) {
    let (_, dismissed, cx) = tour(cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(*dismissed.borrow(), 1);
}

#[gpui::test]
async fn language_select_sets_the_language_on_welcome(cx: &mut TestAppContext) {
    let (view, events, cx) = setup(cx);
    // Closed: only the trigger.
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("onboarding-language").is_some());
    assert!(cx.debug_bounds("onboarding-language-list").is_none());
    click(cx, "onboarding-language");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("onboarding-language-list").is_some());
    assert!(cx.debug_bounds("onboarding-language-system").is_some());
    click(cx, "onboarding-language-es");
    assert_eq!(
        *events.borrow(),
        vec![OnboardingEvent::LanguageChosen(
            settings::LocaleSetting::Code("es".into())
        )]
    );
    assert_eq!(
        view.read_with(cx, |o, _| o.language().clone()),
        settings::LocaleSetting::Code("es".into())
    );
    // The list closed on the pick, and the step did not move.
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("onboarding-language-list").is_none());
    assert_eq!(step(&view, cx), OnboardingStep::Welcome);
}

#[gpui::test]
async fn language_select_is_also_on_the_provider_step(cx: &mut TestAppContext) {
    let (_view, _, cx) = setup(cx);
    click(cx, "onboarding-next");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("onboarding-language").is_some());
}

#[gpui::test]
async fn enter_pushes_the_provider_page_and_focuses_it(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    let welcome = view.read_with(cx, |o, _| o.welcome_page().clone());
    let welcome_focus = welcome.read_with(cx, |p, _| p.focus_handle_for());
    assert!(cx.update(|window, _| welcome_focus.is_focused(window)));
    cx.simulate_keystrokes("enter");
    assert_eq!(step(&view, cx), OnboardingStep::Provider);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let provider = view.read_with(cx, |o, _| o.provider_page().clone());
    let provider_focus = provider.read_with(cx, |p, _| p.focus_handle_for());
    assert!(
        cx.update(|window, _| provider_focus.is_focused(window)),
        "the pushed page takes the focus"
    );
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("onboarding-page-provider").is_some());
    assert!(cx.debug_bounds("onboarding-page-welcome").is_none());
}

#[gpui::test]
async fn escape_pops_the_provider_page(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(step(&view, cx), OnboardingStep::Provider);
    cx.simulate_keystrokes("escape");
    assert_eq!(step(&view, cx), OnboardingStep::Welcome);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let welcome = view.read_with(cx, |o, _| o.welcome_page().clone());
    let welcome_focus = welcome.read_with(cx, |p, _| p.focus_handle_for());
    assert!(cx.update(|window, _| welcome_focus.is_focused(window)));
}

#[gpui::test]
async fn escape_on_welcome_dismisses(cx: &mut TestAppContext) {
    let (_, events, cx) = setup(cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(*events.borrow(), vec![OnboardingEvent::Dismissed]);
}

#[gpui::test]
async fn escape_while_the_sign_in_runs_does_nothing(cx: &mut TestAppContext) {
    let (view, _, cx) = setup_with(cx, Rc::new(Waiting), false);
    cx.simulate_keystrokes("enter enter");
    assert_eq!(step(&view, cx), OnboardingStep::SignIn);
    let screen = view.read_with(cx, |o, _| o.sign_in().clone());
    screen.update_in(cx, |s, window, cx| s.choose("window", window, cx));
    assert!(matches!(
        screen.read_with(cx, |s, _| s.step().clone()),
        Step::Waiting
    ));
    cx.simulate_keystrokes("escape");
    assert_eq!(step(&view, cx), OnboardingStep::SignIn);
}

#[gpui::test]
async fn the_added_workspace_flow_has_four_steps(cx: &mut TestAppContext) {
    let (view, _, cx) = setup_with(cx, Rc::new(Fake), true);
    assert_eq!(view.read_with(cx, |o, _| o.steps()), 4);
    assert_eq!(components::onboarding::ADDED_STEPS, 4);
}

#[gpui::test]
async fn one_page_is_drawn_and_no_timer_runs_once_the_transition_ends(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    // A frozen-motion push is immediate: the old page is gone at once.
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("onboarding-page-provider").is_some());
    assert!(cx.debug_bounds("onboarding-page-welcome").is_none());
    // Nothing is left to animate or time: parking again does not move the clock.
    cx.run_until_parked();
    let now = cx.executor().now();
    cx.run_until_parked();
    assert_eq!(cx.executor().now(), now, "no timer is pending");
    assert_eq!(step(&view, cx), OnboardingStep::Provider);
}

#[gpui::test]
async fn an_animated_push_settles_to_one_page(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    cx.update(|_, cx| components::set_motion(cx, components::Motion::Live));
    cx.simulate_keystrokes("enter");
    // Draw to start the clock, then run the animation to its end.
    cx.update(|window, cx| window.draw(cx).clear(cx));
    for _ in 0..2 {
        cx.executor()
            .advance_clock(themes::metrics::DURATION_BASE + std::time::Duration::from_millis(1));
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    assert!(cx.debug_bounds("onboarding-page-provider").is_some());
    assert!(
        cx.debug_bounds("onboarding-page-welcome").is_none(),
        "the old page unmounts when the transition ends"
    );
    let now = cx.executor().now();
    cx.run_until_parked();
    assert_eq!(cx.executor().now(), now, "no timer after the transition");
    assert_eq!(step(&view, cx), OnboardingStep::Provider);
}
