//! The email-code sign-in: the GPUI-free model (`EmailCodeFlow`) and the email and
//! code steps of the sign-in screen against a `FakeCodeSignIn`. No network.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use components::code_sign_in::{
    CodeAlphabet, CodeError, CodeSent, CodeSignIn, CodeSpec, EmailCodeFlow, FlowState, mask_email,
};
use components::sign_in::{
    SignIn, SignInError, SignInEvent, SignInOption, SignInResult, SignInScreen, SignInText, Step,
};
use components::workspace_picker::RefreshStep;
use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Task, TestAppContext, Window};
use settings::WorkspaceKey;

fn spec() -> CodeSpec {
    CodeSpec {
        length: 6,
        alphabet: CodeAlphabet::Digits,
        group: Some(3),
    }
}

fn sent(after: u64) -> CodeSent {
    CodeSent {
        spec: spec(),
        resend_after: Duration::from_secs(after),
    }
}

fn flow_on_code(now: Instant) -> EmailCodeFlow {
    let mut f = EmailCodeFlow::new();
    f.set_email("a@b.example");
    f.start_sending();
    f.sent(sent(30), now);
    f
}

// ---- model ----

#[test]
fn email_check_is_syntactic() {
    let mut f = EmailCodeFlow::new();
    for (s, ok) in [
        ("a@b.example", true),
        ("", false),
        ("a b@c.example", false),
        ("ab.example", false),
        ("a@bexample", false),
        ("@b.example", false),
        ("a@b.", false),
    ] {
        f.set_email(s);
        assert_eq!(f.email_ok(), ok, "{s:?}");
    }
    f.set_email("  a@b.example ");
    assert!(f.email_ok(), "surrounding spaces are trimmed");
    assert_eq!(f.email(), "a@b.example");
}

#[test]
fn sent_makes_one_cell_per_code_char() {
    let now = Instant::now();
    let f = flow_on_code(now);
    assert_eq!(f.state(), FlowState::Code);
    assert_eq!(f.cells().len(), 6);
    assert!(f.cells().iter().all(Option::is_none));
    assert_eq!(f.spec().unwrap().group, Some(3));
}

#[test]
fn typing_ignores_chars_outside_the_alphabet_and_backspace_clears_the_last() {
    let mut f = flow_on_code(Instant::now());
    assert!(!f.type_char('a'));
    assert!(!f.type_char(' '));
    assert!(f.type_char('1'));
    assert!(f.type_char('2'));
    assert_eq!(f.filled(), 2);
    f.backspace();
    assert_eq!(f.filled(), 1);
    assert_eq!(f.cells()[0], Some('1'));
    assert_eq!(f.cells()[1], None);
}

#[test]
fn alphanumeric_codes_take_letters_upper_cased() {
    let mut f = EmailCodeFlow::new();
    f.set_email("a@b.example");
    f.sent(
        CodeSent {
            spec: CodeSpec {
                length: 4,
                alphabet: CodeAlphabet::Alphanumeric,
                group: None,
            },
            resend_after: Duration::ZERO,
        },
        Instant::now(),
    );
    assert!(f.paste("ab-1c"));
    assert_eq!(f.code().unwrap().expose(), "AB1C");
}

#[test]
fn paste_strips_spaces_and_hyphens_or_changes_nothing() {
    for s in ["123456", "123 456", "123-456", " 12-34 56 "] {
        let mut f = flow_on_code(Instant::now());
        assert!(f.paste(s), "{s:?}");
        assert_eq!(f.code().unwrap().expose(), "123456");
    }
    for s in ["12345", "1234567", "12a456", ""] {
        let mut f = flow_on_code(Instant::now());
        f.type_char('9');
        assert!(!f.paste(s), "{s:?}");
        assert_eq!(f.filled(), 1, "unchanged by {s:?}");
    }
}

#[test]
fn code_is_some_only_when_every_cell_is_filled() {
    let mut f = flow_on_code(Instant::now());
    for c in "12345".chars() {
        f.type_char(c);
        assert!(f.code().is_none());
    }
    f.type_char('6');
    assert_eq!(f.code().unwrap().expose(), "123456");
    assert!(!f.type_char('7'), "full");
    assert!(!format!("{:?}", f.code().unwrap()).contains("123456"));
}

#[test]
fn resend_waits_for_resend_after() {
    let now = Instant::now();
    let f = flow_on_code(now);
    assert!(!f.can_resend(now));
    assert!(!f.can_resend(now + Duration::from_secs(29)));
    assert!(f.can_resend(now + Duration::from_secs(30)));
    assert_eq!(f.resend_at(), Some(now + Duration::from_secs(30)));
}

#[test]
fn wrong_code_clears_the_cells_and_keeps_the_email() {
    let now = Instant::now();
    let mut f = flow_on_code(now);
    f.paste("123456");
    f.start_verifying();
    assert_eq!(f.state(), FlowState::Verifying);
    f.fail(
        CodeError::WrongCode {
            attempts_left: Some(2),
        },
        now,
    );
    assert_eq!(f.state(), FlowState::Code);
    assert_eq!(f.filled(), 0);
    assert_eq!(f.email(), "a@b.example");
    assert!(matches!(f.error(), Some(CodeError::WrongCode { .. })));
}

#[test]
fn expired_keeps_the_code_step_and_offers_resend_at_once() {
    let now = Instant::now();
    let mut f = flow_on_code(now);
    f.paste("123456");
    f.start_verifying();
    f.fail(CodeError::Expired, now);
    assert_eq!(f.state(), FlowState::Code);
    assert!(f.can_resend(now));
}

#[test]
fn rate_limited_sets_resend_at_from_retry_after() {
    let now = Instant::now();
    let mut f = flow_on_code(now);
    f.fail(
        CodeError::RateLimited {
            retry_after: Some(Duration::from_secs(90)),
        },
        now,
    );
    assert_eq!(f.resend_at(), Some(now + Duration::from_secs(90)));
    let mut f = flow_on_code(now);
    f.fail(CodeError::RateLimited { retry_after: None }, now);
    assert_eq!(f.resend_at(), Some(now + Duration::from_secs(60)));
}

#[test]
fn a_failed_send_returns_to_the_email_step() {
    let mut f = EmailCodeFlow::new();
    f.set_email("a@b.example");
    f.start_sending();
    assert_eq!(f.state(), FlowState::Sending);
    f.fail(CodeError::Offline, Instant::now());
    assert_eq!(f.state(), FlowState::Email);
    assert_eq!(f.error(), Some(&CodeError::Offline));
}

#[test]
fn debug_shows_neither_the_email_nor_the_code() {
    let mut f = flow_on_code(Instant::now());
    f.paste("987654");
    let dbg = format!("{f:?}");
    assert!(!dbg.contains("a@b.example"), "{dbg}");
    assert!(!dbg.contains("987654") && !dbg.contains("'9'"), "{dbg}");
    assert!(dbg.contains("filled: 6"), "{dbg}");
}

#[test]
fn back_to_email_wipes_the_cells_and_keeps_the_address() {
    let mut f = flow_on_code(Instant::now());
    f.paste("123456");
    f.back_to_email();
    assert!(f.cells().is_empty());
    assert!(f.code().is_none());
    assert_eq!(f.email(), "a@b.example");
    assert_eq!(f.state(), FlowState::Email);
}

#[test]
fn masks_all_but_the_first_letter_and_the_domain() {
    assert_eq!(
        mask_email("person@example.test"),
        "p\u{2022}\u{2022}\u{2022}@example.test"
    );
    assert_eq!(mask_email("x"), "\u{2022}\u{2022}\u{2022}");
}

// ---- view ----

type Log = Rc<RefCell<Vec<String>>>;

/// Scripted code sign-in: each call takes the next answer; `None` never answers.
#[derive(Default)]
struct FakeCodeSignIn {
    log: Log,
    sends: RefCell<VecDeque<Option<Result<CodeSent, CodeError>>>>,
    confirms: RefCell<VecDeque<Option<Result<SignInResult, CodeError>>>>,
}

fn hang<T: 'static>(cx: &mut App) -> Task<T> {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_secs(3600))
                .await;
        }
    })
}

impl CodeSignIn for FakeCodeSignIn {
    fn send_code(
        &self,
        _email: &str,
        resend: bool,
        cx: &mut App,
    ) -> Task<Result<CodeSent, CodeError>> {
        self.log
            .borrow_mut()
            .push(if resend { "resend" } else { "send" }.into());
        match self.sends.borrow_mut().pop_front().flatten() {
            Some(r) => Task::ready(r),
            None => hang(cx),
        }
    }
    fn confirm_code(
        &self,
        _email: &str,
        code: domain::SecretText,
        _progress: UnboundedSender<RefreshStep>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SignInResult, CodeError>> {
        self.log
            .borrow_mut()
            .push(format!("confirm:{}", code.expose().len()));
        match self.confirms.borrow_mut().pop_front().flatten() {
            Some(r) => Task::ready(r),
            None => hang(cx),
        }
    }
    fn forget(&self) {
        self.log.borrow_mut().push("forget".into());
    }
}

struct Provider {
    code: Rc<FakeCodeSignIn>,
}

impl SignIn for Provider {
    fn provider_name(&self) -> String {
        "Fake".into()
    }
    fn notice(&self) -> Vec<SignInText> {
        vec!["Unofficial client.".into()]
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
    fn email_code(&self) -> Option<Rc<dyn CodeSignIn>> {
        Some(self.code.clone())
    }
    fn run(
        &self,
        option: &str,
        _values: Vec<(String, String)>,
        _progress: UnboundedSender<RefreshStep>,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SignInResult, SignInError>> {
        self.code.log.borrow_mut().push(format!("run:{option}"));
        hang(cx)
    }
    fn sign_out(&self, _: &WorkspaceKey, _: &mut App) -> Task<Result<(), String>> {
        Task::ready(Ok(()))
    }
}

struct View<'a> {
    screen: gpui::Entity<SignInScreen>,
    events: Rc<RefCell<Vec<SignInEvent>>>,
    log: Log,
    cx: &'a mut gpui::VisualTestContext,
}

fn setup(cx: &mut TestAppContext, code: FakeCodeSignIn) -> View<'_> {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let code = Rc::new(code);
    let log = code.log.clone();
    let (screen, cx) =
        cx.add_window_view(|_, cx| SignInScreen::new(Rc::new(Provider { code }), cx));
    let events: Rc<RefCell<Vec<SignInEvent>>> = Rc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&screen, move |_, ev: &SignInEvent, _| {
            sink.borrow_mut().push(ev.clone())
        })
        .detach();
    });
    View {
        screen,
        events,
        log,
        cx,
    }
}

impl View<'_> {
    fn consent(&mut self) {
        self.screen.update_in(self.cx, |s, window, cx| {
            s.set_consent(true, cx);
            s.start_email(window, cx);
        });
        self.cx.run_until_parked();
    }
    fn step(&mut self) -> Step {
        self.screen.read_with(self.cx, |s, _| s.step().clone())
    }
    fn flow_state(&mut self) -> FlowState {
        self.screen.read_with(self.cx, |s, _| s.code_flow().state())
    }
    fn keys(&mut self, keys: &str) {
        self.cx.simulate_keystrokes(keys);
        self.cx.run_until_parked();
    }
    fn on_code_step(&mut self) {
        self.consent();
        self.cx.simulate_input("person@example.test");
        self.keys("enter");
        assert_eq!(self.step(), Step::Code);
    }
    fn text(&mut self) -> String {
        self.screen.read_with(self.cx, |s, cx| s.visible_text(cx))
    }
}

fn answers_sent(after: u64) -> FakeCodeSignIn {
    let f = FakeCodeSignIn::default();
    f.sends.borrow_mut().push_back(Some(Ok(sent(after))));
    f
}

#[gpui::test]
fn after_consent_the_email_step_shows_with_its_field_focused(cx: &mut TestAppContext) {
    let mut v = setup(cx, FakeCodeSignIn::default());
    assert_eq!(v.step(), Step::Options, "consent first");
    v.consent();
    assert_eq!(v.step(), Step::Email);
    let focused = v
        .screen
        .update_in(v.cx, |s, window, cx| s.email_focused(window, cx));
    assert!(focused);
}

#[gpui::test]
fn enter_submits_the_email_and_sending_disables_continue(cx: &mut TestAppContext) {
    let mut v = setup(cx, FakeCodeSignIn::default());
    v.consent();
    v.cx.simulate_input("person@example.test");
    v.keys("enter");
    assert_eq!(*v.log.borrow(), vec!["send".to_string()]);
    assert_eq!(v.flow_state(), FlowState::Sending);
    assert_eq!(v.step(), Step::Email);
    assert!(v.text().contains("Sending"));
    // A second Enter while sending sends nothing.
    v.keys("enter");
    assert_eq!(v.log.borrow().len(), 1);
}

#[gpui::test]
fn an_invalid_email_is_not_sent(cx: &mut TestAppContext) {
    let mut v = setup(cx, FakeCodeSignIn::default());
    v.consent();
    v.cx.simulate_input("not-an-email");
    v.keys("enter");
    assert!(v.log.borrow().is_empty());
    assert!(matches!(
        v.screen
            .read_with(v.cx, |s, _| s.code_flow().error().cloned()),
        Some(CodeError::InvalidEmail)
    ));
}

#[gpui::test]
fn the_code_step_masks_the_email_and_fills_cells_as_typed(cx: &mut TestAppContext) {
    let mut v = setup(cx, answers_sent(30));
    v.on_code_step();
    let text = v.text();
    assert!(text.contains("We sent a code to"), "{text}");
    assert!(
        text.contains("p\u{2022}\u{2022}\u{2022}@example.test"),
        "{text}"
    );
    assert!(!text.contains("person@"), "{text}");
    assert!(
        v.screen
            .update_in(v.cx, |s, window, cx| s.cells_focused(window, cx))
    );
    assert_eq!(v.screen.read_with(v.cx, |s, _| s.active_cell()), 0);
    v.keys("1 x 2");
    assert_eq!(v.screen.read_with(v.cx, |s, _| s.active_cell()), 2);
    v.keys("backspace");
    assert_eq!(v.screen.read_with(v.cx, |s, _| s.active_cell()), 1);
}

#[gpui::test]
fn the_last_char_submits_the_code(cx: &mut TestAppContext) {
    let mut v = setup(cx, answers_sent(30));
    v.on_code_step();
    v.keys("1 2 3 4 5 6");
    assert_eq!(v.log.borrow().last().unwrap(), "confirm:6");
    assert_eq!(v.flow_state(), FlowState::Verifying);
}

#[gpui::test]
fn verifying_disables_the_cells_and_their_controls(cx: &mut TestAppContext) {
    let mut v = setup(cx, answers_sent(0));
    v.on_code_step();
    v.keys("1 2 3 4 5 6");
    assert_eq!(v.flow_state(), FlowState::Verifying);
    assert!(v.screen.read_with(v.cx, |s, _| s.code_busy()));
    v.keys("backspace 7");
    assert_eq!(
        v.screen.read_with(v.cx, |s, _| s.active_cell()),
        6,
        "cells ignore keys"
    );
    v.screen.update_in(v.cx, |s, w, cx| s.resend(w, cx));
    assert!(!v.log.borrow().contains(&"resend".to_string()));
    let order: Vec<&str> = (0..3)
        .map(|_| {
            v.keys("tab");
            v.screen
                .update_in(v.cx, |s, w, _| s.focused_code_control(w))
        })
        .collect();
    assert!(
        order
            .iter()
            .all(|c| !matches!(*c, "cells" | "resend" | "different")),
        "{order:?}"
    );
}

#[gpui::test]
fn paste_fills_every_cell_and_submits(cx: &mut TestAppContext) {
    let mut v = setup(cx, answers_sent(30));
    v.on_code_step();
    v.cx.write_to_clipboard(gpui::ClipboardItem::new_string("123-456".into()));
    if cfg!(target_os = "macos") {
        v.keys("cmd-v");
    } else {
        v.keys("ctrl-v");
    }
    assert_eq!(v.log.borrow().last().unwrap(), "confirm:6");
}

#[gpui::test]
fn resend_is_disabled_until_the_one_shot_timer_fires(cx: &mut TestAppContext) {
    let code = answers_sent(30);
    code.sends.borrow_mut().push_back(Some(Ok(sent(30))));
    let mut v = setup(cx, code);
    v.on_code_step();
    assert!(!v.screen.read_with(v.cx, |s, _| s.resend_ready()));
    assert!(v.text().contains("You can ask for a new code in 30 s"));
    v.screen.update_in(v.cx, |s, w, cx| s.resend(w, cx));
    assert_eq!(v.log.borrow().len(), 1, "no resend while waiting");
    v.cx.executor().advance_clock(Duration::from_secs(31));
    v.cx.run_until_parked();
    assert!(v.screen.read_with(v.cx, |s, _| s.resend_ready()));
    v.screen.update_in(v.cx, |s, w, cx| s.resend(w, cx));
    v.cx.run_until_parked();
    assert_eq!(v.log.borrow().last().unwrap(), "resend");
    assert!(v.text().contains("We sent a new code"));
    assert!(!v.screen.read_with(v.cx, |s, _| s.resend_ready()));
}

#[gpui::test]
fn a_different_email_returns_to_the_email_step_and_forgets(cx: &mut TestAppContext) {
    let mut v = setup(cx, answers_sent(30));
    v.on_code_step();
    v.screen
        .update_in(v.cx, |s, w, cx| s.use_different_email(w, cx));
    assert_eq!(v.step(), Step::Email);
    assert_eq!(v.log.borrow().last().unwrap(), "forget");
}

#[gpui::test]
fn escape_on_the_code_step_goes_back(cx: &mut TestAppContext) {
    let mut v = setup(cx, answers_sent(30));
    v.on_code_step();
    v.keys("escape");
    assert_eq!(v.step(), Step::Email);
    assert_eq!(v.log.borrow().last().unwrap(), "forget");
}

#[gpui::test]
fn cancel_forgets(cx: &mut TestAppContext) {
    let mut v = setup(cx, answers_sent(30));
    v.on_code_step();
    v.screen.update(v.cx, |s, cx| s.cancel(cx));
    assert_eq!(v.log.borrow().last().unwrap(), "forget");
    assert_eq!(*v.events.borrow(), vec![SignInEvent::Cancelled]);
}

#[gpui::test]
fn tab_walks_cells_resend_different_email_back_continue(cx: &mut TestAppContext) {
    let code = answers_sent(0);
    let mut v = setup(cx, code);
    v.on_code_step();
    let order: Vec<&str> = (0..6)
        .map(|_| {
            v.keys("tab");
            v.screen
                .update_in(v.cx, |s, w, _| s.focused_code_control(w))
        })
        .collect();
    assert_eq!(
        order,
        ["resend", "different", "back", "continue", "cells", "resend"]
    );
}

#[gpui::test]
fn errors_show_fixed_texts(cx: &mut TestAppContext) {
    let cases: Vec<(CodeError, &str)> = vec![
        (
            CodeError::WrongCode {
                attempts_left: Some(2),
            },
            "2 attempts left",
        ),
        (CodeError::Expired, "expired"),
        (CodeError::TooManyAttempts, "Too many"),
        (
            CodeError::RateLimited {
                retry_after: Some(Duration::from_secs(150)),
            },
            "Try again in 3 minutes",
        ),
        (CodeError::Offline, "offline"),
    ];
    for (err, want) in cases {
        let code = answers_sent(30);
        code.confirms.borrow_mut().push_back(Some(Err(err.clone())));
        let mut cx2 = cx.clone();
        let mut v = setup(&mut cx2, code);
        v.on_code_step();
        v.keys("1 2 3 4 5 6");
        let text = v.text();
        assert!(text.contains(want), "{err:?}: {text}");
        assert_eq!(v.step(), Step::Code);
    }
}

#[gpui::test]
fn needs_browser_offers_the_browser_window(cx: &mut TestAppContext) {
    let code = FakeCodeSignIn::default();
    code.sends
        .borrow_mut()
        .push_back(Some(Err(CodeError::NeedsBrowser {
            option: "window".into(),
            reason: "This workspace uses single sign-on.".into(),
        })));
    let mut v = setup(cx, code);
    v.consent();
    v.cx.simulate_input("person@example.test");
    v.keys("enter");
    let text = v.text();
    assert!(text.contains("single sign-on"), "{text}");
    assert!(text.contains("Continue in browser window"), "{text}");
    v.screen
        .update_in(v.cx, |s, w, cx| s.continue_in_browser(w, cx));
    v.cx.run_until_parked();
    let log = v.log.borrow().clone();
    assert!(log.contains(&"forget".to_string()), "{log:?}");
    assert_eq!(log.last().unwrap(), "run:window");
}

#[gpui::test]
fn without_a_code_sign_in_the_screen_is_unchanged(cx: &mut TestAppContext) {
    struct Plain;
    impl SignIn for Plain {
        fn provider_name(&self) -> String {
            "Plain".into()
        }
        fn notice(&self) -> Vec<SignInText> {
            Vec::new()
        }
        fn options(&self) -> Vec<SignInOption> {
            Vec::new()
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
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let (screen, cx) = cx.add_window_view(|_, cx| SignInScreen::new(Rc::new(Plain), cx));
    screen.update_in(cx, |s, w, cx| s.start_email(w, cx));
    assert_eq!(screen.read_with(cx, |s, _| s.step().clone()), Step::Options);
}
