//! Provider-neutral sign-in screen. A provider describes how to sign in through
//! the [`SignIn`] trait (consent notice, options, input fields); this view
//! renders it and reports the workspace the user ended up with. Values typed
//! into fields are never logged or echoed.

use std::collections::HashMap;
use std::rc::Rc;

use futures::channel::mpsc::UnboundedSender;
use gpui::{
    App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, Render, Subscription, Task, Window, div, prelude::*,
};
use settings::WorkspaceKey;
use themes::ActiveTheme;

use crate::code_sign_in::CodeSignIn;
use crate::controls::{
    ButtonKind, button, button_frame, card, check_box, error_banner, field_frame, focus_style,
    on_activate,
};
use crate::text_input::TextInput;
use crate::workspace_picker::{
    RefreshOutcome, RefreshStep, WorkspaceAdder, WorkspaceChoice, WorkspacePicker,
    WorkspacePickerEvent, WorkspacePickerModel,
};

mod code_view;

/// Height of the strip kept for the Cancel button while a provider window covers
/// the rest of the window.
pub const OVERLAY_TOP: gpui::Pixels = themes::metrics::LAYOUT_SIGN_IN_TOP;

/// Where a provider window starts, in window coordinates: below the host's
/// top inset (for example a transparent titlebar strip) and the header row.
pub fn overlay_top(top_inset: f32) -> f32 {
    top_inset.max(0.0) + OVERLAY_TOP.as_f32()
}

/// Text a provider shows on the sign-in screen: catalog text rendered in the
/// current language on every frame, or a literal (names, test text).
#[derive(Clone, Debug, PartialEq)]
pub enum SignInText {
    Plain(String),
    Localized(i18n::LocalizedText),
}

impl SignInText {
    /// The text in the language of `l`.
    pub fn resolve(&self, l: &i18n::Localizer) -> String {
        match self {
            SignInText::Plain(s) => s.clone(),
            SignInText::Localized(t) => l.text(t).into_owned(),
        }
    }
}

impl From<i18n::LocalizedText> for SignInText {
    fn from(t: i18n::LocalizedText) -> Self {
        SignInText::Localized(t)
    }
}

impl From<String> for SignInText {
    fn from(s: String) -> Self {
        SignInText::Plain(s)
    }
}

impl From<&str> for SignInText {
    fn from(s: &str) -> Self {
        SignInText::Plain(s.to_string())
    }
}

/// One input the user fills in for a sign-in option.
#[derive(Clone, Debug, PartialEq)]
pub struct SignInField {
    pub id: String,
    pub label: SignInText,
    /// Shown masked.
    pub secret: bool,
}

/// One way to sign in, as a button (and a form when it has fields).
#[derive(Clone, Debug, PartialEq)]
pub struct SignInOption {
    pub id: String,
    pub label: SignInText,
    pub fields: Vec<SignInField>,
    /// Instructions shown above the fields.
    pub help: Option<SignInText>,
    /// Text the "Copy" button puts on the clipboard (a console snippet).
    pub copy: Option<String>,
}

/// A workspace found by a sign-in. `ready` is true once its credentials were
/// verified and saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub key: WorkspaceKey,
    pub name: String,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SignInError {
    /// The option cannot work here: show `reason` and open another option.
    SwitchTo { option: String, reason: SignInText },
    /// Plain failure; the message never contains a secret.
    Failed(SignInText),
}

/// A live session that can add several workspaces, with the rows to show.
pub struct WorkspacePickerSession {
    pub choices: Vec<WorkspaceChoice>,
    pub adder: Rc<dyn WorkspaceAdder>,
    /// Workspaces already in Parla whose credentials this sign-in refreshed:
    /// the host restarts them whatever the user does with the list.
    pub refreshed: Vec<WorkspaceKey>,
    /// Added workspaces to refresh in the background in the picker session.
    pub refresh: Vec<String>,
}

/// What an option found: ready workspaces, or several workspaces to pick from.
pub enum SignInResult {
    Workspaces(Vec<Candidate>),
    /// Workspaces to restart or open immediately, with no picker step.
    Restart(Vec<Candidate>),
    Pick(WorkspacePickerSession),
}

/// What a provider offers on the sign-in screen.
pub trait SignIn: 'static {
    fn provider_name(&self) -> String;
    /// Consent statements shown verbatim; empty means no consent step.
    fn notice(&self) -> Vec<SignInText>;
    fn options(&self) -> Vec<SignInOption>;
    /// Runs one option with the field values. Dropping the task cancels it.
    /// Reports work saved during the run (for example a refreshed workspace) on
    /// `progress`, so the host can restart them even if the run is cancelled.
    fn run(
        &self,
        option: &str,
        values: Vec<(String, String)>,
        progress: UnboundedSender<RefreshStep>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SignInResult, SignInError>>;
    /// Tells the provider how tall the host's top strip is (titlebar inset), so
    /// a window it opens starts below that strip and the header row.
    fn set_top_inset(&self, _inset: f32) {}
    /// Called on every render while waiting, so a provider window can follow
    /// the window size.
    fn layout_window(&self, _window: &Window) {}
    /// The email-code sign-in, when the provider has one. `None` keeps the
    /// options screen as it is.
    fn email_code(&self) -> Option<Rc<dyn CodeSignIn>> {
        None
    }
    /// Deletes the stored secrets and the account row.
    fn sign_out(&self, ws: &WorkspaceKey, cx: &mut App) -> Task<Result<(), String>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInEvent {
    SignedIn(WorkspaceKey),
    /// Several workspaces were added at once; the first one opens.
    Added(Vec<WorkspaceKey>),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Options,
    /// The email-code sign-in: the address.
    Email,
    /// The email-code sign-in: the emailed code.
    Code,
    /// The fields of one option.
    Form(String),
    /// A sign-in is running (for example the browser window).
    Waiting,
    Pick(Vec<Candidate>),
    /// Several workspaces to tick before adding them.
    PickWorkspaces(Entity<WorkspacePicker>),
}

pub struct SignInScreen {
    signin: Rc<dyn SignIn>,
    options: Vec<SignInOption>,
    notice: Vec<SignInText>,
    consented: bool,
    step: Step,
    /// One input per field of the open form; dropped (with what was typed) when the
    /// sign-in ends or is cancelled.
    inputs: HashMap<String, Entity<TextInput>>,
    active: usize,
    /// Focus the active input on the next render (no window at hand yet).
    refocus: bool,
    error: Option<SignInText>,
    running: Option<Task<()>>,
    picker: Option<Subscription>,
    /// Refreshed workspaces reported during the current run.
    reported_refresh: Vec<WorkspaceKey>,
    focus: FocusHandle,
    /// Tab stops of the form's Back and Submit buttons.
    back_focus: FocusHandle,
    submit_focus: FocusHandle,
    copy_focus: FocusHandle,
    /// The email-code steps, when the provider has a code sign-in.
    code: Option<code_view::CodeScreen>,
}

impl EventEmitter<SignInEvent> for SignInScreen {}

impl SignInScreen {
    pub fn new(signin: Rc<dyn SignIn>, cx: &mut Context<Self>) -> Self {
        let notice = signin.notice();
        let code = signin
            .email_code()
            .map(|c| code_view::CodeScreen::new(c, cx));
        let mut this = Self {
            options: signin.options(),
            consented: notice.is_empty(),
            notice,
            signin,
            step: Step::Options,
            inputs: HashMap::new(),
            active: 0,
            refocus: false,
            error: None,
            running: None,
            picker: None,
            reported_refresh: Vec::new(),
            focus: cx.focus_handle(),
            back_focus: cx.focus_handle().tab_stop(true),
            submit_focus: cx.focus_handle().tab_stop(true),
            copy_focus: cx.focus_handle().tab_stop(true),
            code,
        };
        // No consent step: the email step is the first screen.
        if this.code.is_some() && this.consented {
            this.step = Step::Email;
            this.refocus = true;
        }
        this
    }

    /// Refreshed workspaces reported so far by the running sign-in.
    pub fn reported_refresh(&self) -> &[WorkspaceKey] {
        &self.reported_refresh
    }

    /// Takes the refreshed workspaces reported by the running sign-in.
    pub fn take_reported_refresh(&mut self) -> Vec<WorkspaceKey> {
        std::mem::take(&mut self.reported_refresh)
    }

    pub fn step(&self) -> &Step {
        &self.step
    }

    pub fn notice(&self) -> &[SignInText] {
        &self.notice
    }

    pub fn consented(&self) -> bool {
        self.consented
    }

    pub fn error(&self) -> Option<&SignInText> {
        self.error.as_ref()
    }

    pub fn set_consent(&mut self, on: bool, cx: &mut Context<Self>) {
        self.consented = on;
        cx.notify();
    }

    fn option(&self, id: &str) -> Option<&SignInOption> {
        self.options.iter().find(|o| o.id == id)
    }

    /// Picks an option: opens its form, or runs it when it has no fields.
    /// Does nothing until the consent box is ticked.
    pub fn choose(&mut self, option: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.consented {
            return;
        }
        let Some(has_fields) = self.option(option).map(|o| !o.fields.is_empty()) else {
            return;
        };
        self.error = None;
        if !has_fields {
            self.start(option.to_string(), window, cx);
        } else {
            self.open_form(option, window, cx);
            cx.notify();
        }
    }

    /// Opens the form of `option` with empty inputs (masked for secret fields) and
    /// focuses the first.
    fn open_form(&mut self, option: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.make_inputs(option, cx);
        self.focus_active(window, cx);
    }

    /// Fresh empty inputs for the fields of `option`, and its form as the step.
    fn make_inputs(&mut self, option: &str, cx: &mut Context<Self>) {
        let fields = self.option(option).map(|o| o.fields.clone());
        self.inputs.clear();
        for f in fields.unwrap_or_default() {
            let secret = f.secret;
            let input = cx.new(|cx| {
                let mut i = TextInput::new(cx);
                i.set_single_line(true);
                i.set_masked(secret);
                i
            });
            self.inputs.insert(f.id, input);
        }
        self.active = 0;
        self.step = Step::Form(option.to_string());
    }

    fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.code_focus(cx) {
            window.focus(&handle, cx);
            return;
        }
        let handle = match self.active_field().and_then(|f| self.inputs.get(&f)) {
            Some(i) => i.read(cx).focus_handle(cx),
            None => self.focus.clone(),
        };
        window.focus(&handle, cx);
    }

    /// The text typed in `field` so far (empty after sign-in ends or is cancelled).
    pub fn field_value(&self, field: &str, cx: &App) -> String {
        self.inputs
            .get(field)
            .map_or_else(String::new, |i| i.read(cx).text().to_string())
    }

    pub fn set_field(&mut self, field: &str, value: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.inputs.get(field) {
            i.update(cx, |i, cx| i.set_text(value, cx));
        }
        cx.notify();
    }

    /// Runs the option whose form is open.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Step::Form(option) = self.step.clone()
            && self.consented
        {
            self.start(option, window, cx);
        }
    }

    fn start(&mut self, option: String, window: &mut Window, cx: &mut Context<Self>) {
        let fields = self.option(&option).map(|o| o.fields.clone());
        let values: Vec<(String, String)> = fields
            .unwrap_or_default()
            .iter()
            .map(|f| {
                (
                    f.id.clone(),
                    self.inputs
                        .get(&f.id)
                        .map(|i| i.read(cx).text().trim().to_string())
                        .unwrap_or_default(),
                )
            })
            .collect();
        self.reported_refresh.clear();
        let (progress, rx) = futures::channel::mpsc::unbounded();
        let task = self.signin.run(&option, values, progress, window, cx);
        self.step = Step::Waiting;
        self.running = Some(self.drive(task, rx, cx, |this, r, cx| this.finished(r, cx)));
        cx.notify();
    }

    /// Runs `task`, reporting its refresh steps as they come, then `done` with its
    /// result (after any steps still queued).
    fn drive<R: Unpin + 'static>(
        &self,
        task: Task<R>,
        rx: futures::channel::mpsc::UnboundedReceiver<RefreshStep>,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) -> Task<()> {
        crate::pump::pump(
            task,
            rx,
            cx,
            |this, step, cx| this.report_refresh(step, cx),
            done,
        )
    }

    fn report_refresh(&mut self, step: RefreshStep, cx: &mut Context<Self>) {
        if let RefreshOutcome::Updated(key) = step.outcome
            && !self.reported_refresh.iter().any(|k| k == &key)
        {
            self.reported_refresh.push(key);
        }
        cx.notify();
    }

    fn finished(&mut self, result: Result<SignInResult, SignInError>, cx: &mut Context<Self>) {
        self.running = None;
        if result.is_ok() {
            // Typed values can hold tokens or cookies; keep them only while a retry may need them.
            self.inputs.clear();
        }
        match result {
            Ok(SignInResult::Workspaces(found)) => {
                let ready: Vec<Candidate> = found.into_iter().filter(|c| c.ready).collect();
                match ready.len() {
                    0 => {
                        let msg = t!(cx, "sign-in-no-workspace").into_owned();
                        self.fail(msg.into())
                    }
                    1 => {
                        let key = ready[0].key.clone();
                        self.step = Step::Options;
                        cx.emit(SignInEvent::SignedIn(key));
                    }
                    _ => self.step = Step::Pick(ready),
                }
            }
            Ok(SignInResult::Restart(found)) => {
                let mut keys: Vec<WorkspaceKey> = found
                    .into_iter()
                    .filter(|c| c.ready)
                    .map(|c| c.key)
                    .collect();
                let reported = std::mem::take(&mut self.reported_refresh);
                for key in reported {
                    if !keys.iter().any(|k| k == &key) {
                        keys.push(key);
                    }
                }
                if keys.is_empty() {
                    let msg = t!(cx, "sign-in-no-workspace").into_owned();
                    self.fail(msg.into());
                } else {
                    self.step = Step::Options;
                    cx.emit(SignInEvent::Added(keys));
                }
            }
            Ok(SignInResult::Pick(session)) => {
                self.picker = None;
                let picker = cx.new(|cx| {
                    WorkspacePicker::new(
                        WorkspacePickerModel::new(session.choices),
                        session.adder,
                        session.refreshed,
                        session.refresh,
                        cx,
                    )
                });
                let sub = cx.subscribe(&picker, |this, _, ev: &WorkspacePickerEvent, cx| {
                    match ev {
                        WorkspacePickerEvent::Added(keys) => {
                            this.picker = None;
                            this.step = Step::Options;
                            cx.emit(SignInEvent::Added(keys.clone()));
                        }
                        WorkspacePickerEvent::Cancelled => this.cancel(cx),
                    }
                    cx.notify();
                });
                self.picker = Some(sub);
                self.step = Step::PickWorkspaces(picker);
            }
            Err(SignInError::SwitchTo { option, reason }) => {
                self.make_inputs(&option, cx);
                self.refocus = true;
                self.error = Some(reason);
            }
            Err(SignInError::Failed(m)) => self.fail(m),
        }
        cx.notify();
    }

    fn fail(&mut self, message: SignInText) {
        self.step = Step::Options;
        self.error = Some(message);
    }

    pub fn pick(&mut self, key: &WorkspaceKey, cx: &mut Context<Self>) {
        let known = matches!(&self.step, Step::Pick(c) if c.iter().any(|c| &c.key == key));
        if known {
            // Every listed workspace was saved; the host restarts them all and
            // opens the picked one.
            let mut keys: Vec<WorkspaceKey> = match &self.step {
                Step::Pick(c) => c
                    .iter()
                    .filter(|c| &c.key != key)
                    .map(|c| c.key.clone())
                    .collect(),
                _ => Vec::new(),
            };
            self.step = Step::Options;
            if keys.is_empty() {
                cx.emit(SignInEvent::SignedIn(key.clone()));
            } else {
                keys.push(key.clone());
                cx.emit(SignInEvent::Added(keys));
            }
            cx.notify();
        }
    }

    /// Abandons a running sign-in (drops its task, which closes any window).
    /// Any refreshed workspaces already reported are restarted.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.forget_code();
        self.running = None;
        self.inputs.clear();
        self.picker = None;
        self.step = Step::Options;
        self.error = None;
        let reported = std::mem::take(&mut self.reported_refresh);
        if reported.is_empty() {
            cx.emit(SignInEvent::Cancelled);
        } else {
            cx.emit(SignInEvent::Added(reported));
        }
        cx.notify();
    }

    fn active_field(&self) -> Option<String> {
        let Step::Form(option) = &self.step else {
            return None;
        };
        self.option(option)?
            .fields
            .get(self.active)
            .map(|f| f.id.clone())
    }

    /// Keys the screen handles: Tab and Shift-Tab move through the controls,
    /// Enter submits the form and Escape leaves it.
    fn on_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.step, Step::Email | Step::Code) {
            return self.code_key_down(ev, window, cx);
        }
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        match k.key.as_str() {
            "tab" => {
                if let Step::Form(option) = &self.step {
                    let (n, copy) = self
                        .option(option)
                        .map_or((0, false), |o| (o.fields.len(), o.copy.is_some()));
                    self.tab_in_form(n, copy, m.shift, window, cx);
                } else if m.shift {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
            }
            "enter" if self.active_field().is_some() => return self.submit(window, cx),
            "escape" if self.active_field().is_some() => {
                self.step = Step::Options;
                window.focus(&self.focus, cx);
            }
            _ => return,
        }
        // The field must not also insert the tab.
        cx.stop_propagation();
        cx.notify();
    }

    /// Tab order of a form: Copy (when it has one), its fields, Back, then
    /// Submit, wrapping.
    fn tab_in_form(
        &mut self,
        fields: usize,
        copy: bool,
        back: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lead = usize::from(copy);
        let total = lead + fields + 2;
        let current = if copy && self.copy_focus.is_focused(window) {
            Some(0)
        } else if self.back_focus.is_focused(window) {
            Some(lead + fields)
        } else if self.submit_focus.is_focused(window) {
            Some(lead + fields + 1)
        } else if self.active_field().is_some() {
            Some(lead + self.active.min(fields.saturating_sub(1)))
        } else {
            None
        };
        let next = match (current, back) {
            (None, false) => 0,
            (None, true) => total - 1,
            (Some(c), false) => (c + 1) % total,
            (Some(c), true) => (c + total - 1) % total,
        };
        if copy && next == 0 {
            window.focus(&self.copy_focus, cx);
        } else if next < lead + fields {
            self.active = next - lead;
            self.focus_active(window, cx);
        } else if next == lead + fields {
            window.focus(&self.back_focus, cx);
        } else {
            window.focus(&self.submit_focus, cx);
        }
    }
}

impl Focusable for SignInScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SignInScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.refocus) {
            self.focus_active(window, cx);
        }
        let colors = *cx.theme().colors();
        let error_colour = cx.theme().status().error;
        let control_border = themes::parla_tokens(cx.theme()).border_control;
        let loc = i18n::gpui::localizer(cx);
        let primary = ButtonKind::Primary;
        let secondary = ButtonKind::Secondary;
        if self.step == Step::Waiting {
            self.signin.layout_window(window);
            return div()
                .flex()
                .flex_col()
                .size_full()
                .bg(colors.background)
                .text_color(colors.text)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(themes::metrics::SPACE_5)
                        .px(themes::metrics::SPACE_6)
                        .h(OVERLAY_TOP)
                        .border_b_1()
                        .border_color(colors.border)
                        .bg(colors.surface_background)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(themes::metrics::FONT_SIZE_SM)
                                .text_color(colors.text_muted)
                                .child(t!(cx, "sign-in-waiting").into_owned()),
                        )
                        .child(on_activate(
                            button(
                                "cancel",
                                t!(cx, "sign-in-cancel").into_owned(),
                                secondary,
                                true,
                                &colors,
                            ),
                            cx,
                            |this, _, cx| this.cancel(cx),
                        )),
                );
        }
        if let Step::PickWorkspaces(picker) = &self.step {
            self.signin.layout_window(window);
            return div().flex().size_full().child(picker.clone());
        }
        let mut col = div()
            .key_context("SignIn")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .flex_col()
            .w_full()
            .gap(themes::metrics::SPACE_7)
            .p(themes::metrics::SPACE_9)
            .max_w(themes::metrics::LAYOUT_SIGN_IN_CONTENT_MAX)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(themes::metrics::SPACE_5)
                    .child(crate::logo::logo(themes::metrics::SPACE_12, cx))
                    .child(
                        div()
                            .text_size(themes::metrics::FONT_SIZE_4XL)
                            .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                            .child(t!(cx, "sign-in-wordmark").into_owned()),
                    ),
            )
            .child(
                div()
                    .text_size(themes::metrics::FONT_SIZE_3XL)
                    .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                    .child(match self.code_title(cx) {
                        // The email and code steps name themselves.
                        Some(title) => title,
                        None => t!(cx, "sign-in-title", provider = self.signin.provider_name())
                            .into_owned(),
                    }),
            );
        if let Some(e) = &self.error {
            col = col.child(error_banner(e.resolve(&loc), error_colour, &colors));
        }
        match self.step.clone() {
            Step::Options => {
                if !self.notice.is_empty() {
                    let mut notice = card(&colors)
                        .text_size(themes::metrics::FONT_SIZE_MD)
                        .text_color(colors.text_muted);
                    for line in &self.notice {
                        notice = notice.child(div().child(line.resolve(&loc)));
                    }
                    col = col.child(notice);
                    let box_ = div()
                        .id("consent")
                        .flex()
                        .items_center()
                        .gap(themes::metrics::SPACE_4)
                        .py(themes::metrics::SPACE_2)
                        .rounded(themes::metrics::RADIUS_SM)
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .cursor_pointer()
                        .tab_stop(true)
                        .focusable()
                        .focus_visible(focus_style(&colors))
                        .child(check_box(self.consented, &colors, control_border))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(themes::metrics::FONT_SIZE_MD)
                                .child(t!(cx, "sign-in-consent").into_owned()),
                        );
                    col = col.child(on_activate(box_, cx, |this, _, cx| {
                        let on = !this.consented;
                        this.set_consent(on, cx);
                    }));
                }
                let with_code = self.code.is_some();
                if with_code {
                    col = col.child(on_activate(
                        button(
                            "opt-email-code",
                            t!(cx, "sign-in-code-with-email").into_owned(),
                            primary,
                            self.consented,
                            &colors,
                        )
                        .debug_selector(|| "sign-in-opt-email-code".into()),
                        cx,
                        |this, window, cx| this.start_email(window, cx),
                    ));
                }
                for (n, o) in self.options.iter().enumerate() {
                    let id = o.id.clone();
                    let sel = format!("sign-in-opt-{}", o.id);
                    col = col.child(on_activate(
                        button(
                            format!("opt-{}", o.id),
                            o.label.resolve(&loc),
                            if n == 0 && !with_code {
                                primary
                            } else {
                                secondary
                            },
                            self.consented,
                            &colors,
                        )
                        .debug_selector(move || sel.clone()),
                        cx,
                        move |this, window, cx| this.choose(&id, window, cx),
                    ));
                }
            }
            Step::Form(option) => {
                if let Some(o) = self.option(&option).cloned() {
                    if let Some(help) = &o.help {
                        col = col.child(
                            div()
                                .text_size(themes::metrics::FONT_SIZE_SM)
                                .text_color(colors.text_muted)
                                .child(help.resolve(&loc)),
                        );
                    }
                    if let Some(copy) = o.copy.clone() {
                        col = col.child(on_activate(
                            button(
                                "copy",
                                t!(cx, "sign-in-copy").into_owned(),
                                secondary,
                                true,
                                &colors,
                            )
                            .track_focus(&self.copy_focus),
                            cx,
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            },
                        ));
                    }
                    for (i, f) in o.fields.iter().enumerate() {
                        let focused = self.inputs.get(&f.id).is_some_and(|input| {
                            input.read(cx).focus_handle(cx).is_focused(window)
                        });
                        col = col.child(
                            div()
                                .id(gpui::SharedString::from(format!("field-{}", f.id)))
                                .debug_selector(|| format!("sign-in-field-{}", f.id))
                                .flex()
                                .flex_col()
                                .gap(themes::metrics::SPACE_3)
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.active = i;
                                    this.focus_active(window, cx);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_size(themes::metrics::FONT_SIZE_XS)
                                        .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                                        .text_color(colors.text_muted)
                                        .child(f.label.resolve(&loc)),
                                )
                                .child(
                                    field_frame(focused, &colors, control_border)
                                        .children(self.inputs.get(&f.id).cloned()),
                                ),
                        );
                    }
                    let back = on_activate(
                        button_frame("back", secondary, true, &colors)
                            .gap(themes::metrics::SPACE_3)
                            .child(
                                crate::icon::Icon::new(crate::icon::IconName::ArrowLeft)
                                    .size(themes::metrics::ICON_SIZE_SM)
                                    .color(colors.text),
                            )
                            .child(t!(cx, "sign-in-back").into_owned())
                            .track_focus(&self.back_focus),
                        cx,
                        |this, window, cx| {
                            this.step = Step::Options;
                            window.focus(&this.focus, cx);
                            cx.notify();
                        },
                    );
                    let submit = on_activate(
                        button(
                            "submit",
                            t!(cx, "sign-in-submit").into_owned(),
                            primary,
                            true,
                            &colors,
                        )
                        .track_focus(&self.submit_focus),
                        cx,
                        |this, window, cx| this.submit(window, cx),
                    );
                    col = col.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(themes::metrics::SPACE_5)
                            .child(back)
                            .child(div().flex_1())
                            .child(submit),
                    );
                }
            }
            Step::Waiting => {}
            Step::PickWorkspaces(_) => {}
            Step::Email | Step::Code => col = self.code_body(col, window, cx),
            Step::Pick(found) => {
                col = col.child(
                    div()
                        .text_size(themes::metrics::FONT_SIZE_MD)
                        .text_color(colors.text_muted)
                        .child(t!(cx, "sign-in-pick").into_owned()),
                );
                for c in found {
                    let key = c.key.clone();
                    col = col.child(on_activate(
                        button(format!("pick-{}", c.key), c.name, secondary, true, &colors),
                        cx,
                        move |this, _, cx| this.pick(&key, cx),
                    ));
                }
            }
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.background)
            .text_color(colors.text)
            .child(
                // Scrolls when the card is taller than the window; `my_auto`
                // centres it while it fits.
                div()
                    .id("sign-in-scroll")
                    .debug_selector(|| "sign-in-scroll".into())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .overflow_y_scroll()
                    .child(col.flex_none().my_auto()),
            )
            .child(
                div()
                    .debug_selector(|| "sign-in-footer".into())
                    .flex_none()
                    .px(themes::metrics::SPACE_9)
                    .py(themes::metrics::SPACE_5)
                    .border_t_1()
                    .border_color(colors.border)
                    .text_size(themes::metrics::FONT_SIZE_XS)
                    .text_color(colors.text_muted)
                    .child(t!(cx, "sign-in-independent").into_owned()),
            )
    }
}
