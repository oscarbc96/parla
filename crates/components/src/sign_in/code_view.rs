//! The email and code steps of the sign-in screen, drawn from
//! [`EmailCodeFlow`]. No polling: the resend wait is one one-shot timer owned by
//! the step and dropped when the step changes. The code is never logged.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    App, Context, Div, Entity, FocusHandle, Focusable, KeyDownEvent, Task, Window, div, prelude::*,
};
use themes::{ActiveTheme, metrics, parla_tokens};

use super::{SignInResult, SignInScreen, Step};
use crate::code_sign_in::{
    CodeAlphabet, CodeError, CodeSent, CodeSignIn, DEFAULT_RETRY_AFTER, EmailCodeFlow, FlowState,
    mask_email,
};
use crate::controls::{
    ButtonKind, button, button_frame, error_banner, field_frame, focus_style, on_activate,
};
use crate::icon::{Icon, IconName};
use crate::text_input::TextInput;

/// The email-code state of one sign-in screen.
pub(super) struct CodeScreen {
    signin: Rc<dyn CodeSignIn>,
    flow: EmailCodeFlow,
    email_input: Entity<TextInput>,
    /// The running send or confirm; dropping it cancels it.
    task: Option<Task<()>>,
    /// The one-shot timer that enables Resend.
    timer: Option<Task<()>>,
    resend_ready: bool,
    /// Seconds of the current wait, computed once when it starts.
    wait_secs: u64,
    /// Minutes of a rate limit, computed once when it is reported.
    rate_minutes: u64,
    /// A resend succeeded: "We sent a new code."
    resent: bool,
    cells_focus: FocusHandle,
    resend_focus: FocusHandle,
    different_focus: FocusHandle,
}

impl CodeScreen {
    pub(super) fn new(signin: Rc<dyn CodeSignIn>, cx: &mut Context<SignInScreen>) -> Self {
        let email_input = cx.new(|cx| {
            let mut i = TextInput::new(cx);
            i.set_single_line(true);
            i
        });
        Self {
            signin,
            flow: EmailCodeFlow::new(),
            email_input,
            task: None,
            timer: None,
            resend_ready: false,
            wait_secs: 0,
            rate_minutes: 1,
            resent: false,
            cells_focus: cx.focus_handle().tab_stop(true),
            resend_focus: cx.focus_handle().tab_stop(true),
            different_focus: cx.focus_handle().tab_stop(true),
        }
    }
}

fn ceil_secs(d: Duration) -> u64 {
    d.as_secs() + u64::from(d.subsec_nanos() > 0)
}

impl SignInScreen {
    /// The email-code model. Panics without a code sign-in (`SignIn::email_code`).
    pub fn code_flow(&self) -> &EmailCodeFlow {
        &self.code.as_ref().expect("no code sign-in").flow
    }

    /// Opens the email step (after consent). Does nothing without a code sign-in.
    pub fn start_email(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.consented {
            return;
        }
        let Some(code) = self.code.as_mut() else {
            return;
        };
        code.flow.back_to_email();
        code.task = None;
        code.timer = None;
        self.error = None;
        self.step = Step::Email;
        self.focus_active(window, cx);
        cx.notify();
    }

    pub fn email_focused(&self, window: &Window, cx: &App) -> bool {
        self.code
            .as_ref()
            .is_some_and(|c| c.email_input.read(cx).focus_handle(cx).is_focused(window))
    }

    pub fn cells_focused(&self, window: &Window, _: &App) -> bool {
        self.code
            .as_ref()
            .is_some_and(|c| c.cells_focus.is_focused(window))
    }

    /// The cell the next typed character goes to.
    pub fn active_cell(&self) -> usize {
        self.code.as_ref().map_or(0, |c| c.flow.filled())
    }

    pub fn resend_ready(&self) -> bool {
        self.code.as_ref().is_some_and(|c| c.resend_ready)
    }

    /// A code is being sent or checked.
    pub fn code_busy(&self) -> bool {
        self.code
            .as_ref()
            .is_some_and(|c| matches!(c.flow.state(), FlowState::Sending | FlowState::Verifying))
    }

    /// Which code-step control has the focus: `cells`, `resend`, `different`,
    /// `back`, `continue`, or empty.
    pub fn focused_code_control(&self, window: &Window) -> &'static str {
        let Some(c) = &self.code else { return "" };
        if c.cells_focus.is_focused(window) {
            "cells"
        } else if c.resend_focus.is_focused(window) {
            "resend"
        } else if c.different_focus.is_focused(window) {
            "different"
        } else if self.back_focus.is_focused(window) {
            "back"
        } else if self.submit_focus.is_focused(window) {
            "continue"
        } else {
            ""
        }
    }

    /// The focus target of the email or code step.
    pub(super) fn code_focus(&self, cx: &App) -> Option<FocusHandle> {
        let c = self.code.as_ref()?;
        match self.step {
            Step::Email => Some(c.email_input.read(cx).focus_handle(cx)),
            Step::Code => Some(c.cells_focus.clone()),
            _ => None,
        }
    }

    /// Drops the attempt's confirmation material and anything running.
    pub(super) fn forget_code(&mut self) {
        if let Some(c) = self.code.as_mut() {
            c.task = None;
            c.timer = None;
            c.signin.forget();
        }
    }

    /// Sends a code to the typed address; an address that is not one shows an error.
    pub fn submit_email(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_mut() else {
            return;
        };
        if code.flow.state() == FlowState::Sending {
            return;
        }
        let typed = code.email_input.read(cx).text().to_string();
        code.flow.set_email(&typed);
        if !code.flow.email_ok() {
            code.flow.fail(CodeError::InvalidEmail, Instant::now());
            cx.notify();
            return;
        }
        code.flow.start_sending();
        code.resent = false;
        let task = code.signin.send_code(code.flow.email(), false, cx);
        code.task = Some(cx.spawn(async move |this, cx| {
            let r = task.await;
            this.update(cx, |this, cx| this.code_sent(r, false, cx))
                .ok();
        }));
        cx.notify();
    }

    /// Puts `email` in the email field.
    pub fn set_code_email(&mut self, email: &str, cx: &mut Context<Self>) {
        if let Some(code) = &self.code {
            code.email_input.update(cx, |i, cx| i.set_text(email, cx));
        }
        cx.notify();
    }

    /// Opens the code step for `email` as if a code had just been sent (staged
    /// scenarios start here without waiting for a send).
    pub fn show_code(&mut self, email: &str, sent: CodeSent, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_mut() else {
            return;
        };
        code.flow.set_email(email);
        code.flow.start_sending();
        self.code_sent(Ok(sent), false, cx);
    }

    /// Types `s` into the code cells as keystrokes would; the last cell submits.
    pub fn type_code(&mut self, s: &str, window: &mut Window, cx: &mut Context<Self>) {
        for c in s.chars() {
            let Some(code) = self.code.as_mut() else {
                return;
            };
            if code.flow.type_char(c) && code.flow.code().is_some() {
                self.submit_code(window, cx);
            }
        }
        cx.notify();
    }

    /// Asks for a new code, once the wait is over.
    pub fn resend(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_mut() else {
            return;
        };
        if !code.resend_ready || code.flow.state() != FlowState::Code {
            return;
        }
        code.resend_ready = false;
        code.flow.clear_error();
        let task = code.signin.send_code(code.flow.email(), true, cx);
        code.task = Some(cx.spawn(async move |this, cx| {
            let r = task.await;
            this.update(cx, |this, cx| this.code_sent(r, true, cx)).ok();
        }));
        cx.notify();
    }

    fn code_sent(&mut self, r: Result<CodeSent, CodeError>, resend: bool, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_mut() else {
            return;
        };
        code.task = None;
        match r {
            Ok(sent) => {
                code.flow.sent(sent, Instant::now());
                code.resent = resend;
                self.step = Step::Code;
                self.refocus = true;
                self.wait_for_resend(sent.resend_after, cx);
            }
            Err(e) => self.code_failed(e, cx),
        }
        cx.notify();
    }

    /// Starts the one one-shot timer that enables Resend (none when `d` is zero).
    fn wait_for_resend(&mut self, d: Duration, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_mut() else {
            return;
        };
        code.wait_secs = ceil_secs(d);
        if d.is_zero() {
            code.resend_ready = true;
            code.timer = None;
            return;
        }
        code.resend_ready = false;
        code.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(d).await;
            this.update(cx, |this, cx| {
                if let Some(code) = this.code.as_mut() {
                    code.resend_ready = true;
                    code.timer = None;
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn code_failed(&mut self, e: CodeError, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_mut() else {
            return;
        };
        code.task = None;
        code.resent = false;
        code.flow.fail(e.clone(), Instant::now());
        self.step = if code.flow.state() == FlowState::Email {
            Step::Email
        } else {
            Step::Code
        };
        match e {
            CodeError::Expired | CodeError::TooManyAttempts => {
                code.resend_ready = true;
                code.timer = None;
            }
            CodeError::RateLimited { retry_after } => {
                let d = retry_after.unwrap_or(DEFAULT_RETRY_AFTER);
                code.rate_minutes = ceil_secs(d).div_ceil(60).max(1);
                self.wait_for_resend(d, cx);
            }
            _ => {}
        }
        self.refocus = true;
    }

    /// Confirms the code once every cell is filled.
    pub fn submit_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_mut() else {
            return;
        };
        if code.flow.state() != FlowState::Code {
            return;
        }
        let Some(secret) = code.flow.code() else {
            return;
        };
        code.flow.start_verifying();
        code.resent = false;
        let signin = code.signin.clone();
        let email = code.flow.email().to_string();
        self.reported_refresh.clear();
        let (progress, rx) = futures::channel::mpsc::unbounded();
        let task = signin.confirm_code(&email, secret, progress, window, cx);
        let driven = self.drive(task, rx, cx, |this, r, cx| this.code_confirmed(r, cx));
        if let Some(code) = self.code.as_mut() {
            code.task = Some(driven);
        }
        cx.notify();
    }

    fn code_confirmed(&mut self, r: Result<SignInResult, CodeError>, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_mut() else {
            return;
        };
        code.task = None;
        match r {
            Ok(result) => {
                code.flow.finish();
                code.timer = None;
                self.finished(Ok(result), cx);
            }
            Err(e) => self.code_failed(e, cx),
        }
        cx.notify();
    }

    /// Back to the email step with the address kept; forgets the attempt.
    pub fn use_different_email(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forget_code();
        if let Some(code) = self.code.as_mut() {
            code.flow.back_to_email();
            code.resent = false;
        }
        self.step = Step::Email;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Leaves the email step for the options.
    fn email_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forget_code();
        if let Some(code) = self.code.as_mut() {
            code.flow.back_to_email();
        }
        self.step = Step::Options;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Switches to the option a `NeedsBrowser` answer named.
    pub fn continue_in_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let option = match self.code.as_ref().and_then(|c| c.flow.error()) {
            Some(CodeError::NeedsBrowser { option, .. }) => option.clone(),
            _ => return,
        };
        self.forget_code();
        if let Some(code) = self.code.as_mut() {
            code.flow.back_to_email();
        }
        self.step = Step::Options;
        self.choose(&option, window, cx);
        cx.notify();
    }

    /// Typing, Backspace and paste on the code cells.
    fn cells_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_mut() else {
            return;
        };
        if code.flow.state() != FlowState::Code {
            return;
        }
        let k = &ev.keystroke;
        let m = &k.modifiers;
        let paste = if cfg!(target_os = "macos") {
            m.platform
        } else {
            m.control
        };
        if paste && k.key == "v" {
            let text = cx.read_from_clipboard().and_then(|c| c.text());
            if let Some(text) = text
                && code.flow.paste(&text)
            {
                self.submit_code(window, cx);
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if m.platform || m.control || m.alt || matches!(k.key.as_str(), "tab" | "enter" | "escape")
        {
            return;
        }
        if k.key == "backspace" {
            code.flow.backspace();
        } else {
            let typed = k.key_char.as_deref().unwrap_or(k.key.as_str());
            let mut chars = typed.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return;
            };
            if code.flow.type_char(c) && code.flow.code().is_some() {
                self.submit_code(window, cx);
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Tab, Enter and Escape on the email and code steps.
    pub(super) fn code_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        match (k.key.as_str(), &self.step) {
            ("tab", _) => self.tab_code(m.shift, window, cx),
            ("enter", Step::Email) => self.submit_email(window, cx),
            ("enter", Step::Code) => self.submit_code(window, cx),
            ("escape", Step::Email) => self.email_back(window, cx),
            ("escape", Step::Code) => self.use_different_email(window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Tab order: email, Back, Continue; or cells (one stop), Resend (when
    /// offered), Use a different email, Back, Continue. Wraps.
    fn tab_code(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(code) = self.code.as_ref() else {
            return;
        };
        let mut stops: Vec<FocusHandle> = Vec::new();
        match self.step {
            Step::Email => stops.push(code.email_input.read(cx).focus_handle(cx)),
            _ if code.flow.state() == FlowState::Verifying => {}
            _ => {
                stops.push(code.cells_focus.clone());
                if code.resend_ready {
                    stops.push(code.resend_focus.clone());
                }
                stops.push(code.different_focus.clone());
            }
        }
        stops.push(self.back_focus.clone());
        stops.push(self.submit_focus.clone());
        let n = stops.len();
        let next = match stops.iter().position(|h| h.is_focused(window)) {
            None if back => n - 1,
            None => 0,
            Some(i) if back => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
        };
        window.focus(&stops[next], cx);
    }

    /// The fixed text of an error.
    fn code_error_text(&self, e: &CodeError, cx: &App) -> String {
        let l = i18n::gpui::localizer(cx);
        let minutes = self.code.as_ref().map_or(1, |c| c.rate_minutes);
        match e {
            CodeError::InvalidEmail => t!(l, "sign-in-code-error-invalid-email").into_owned(),
            CodeError::NeedsBrowser { reason, .. } => reason.resolve(&l),
            CodeError::WrongCode {
                attempts_left: Some(n),
            } => t!(l, "sign-in-code-error-wrong-left", left = *n).into_owned(),
            CodeError::WrongCode {
                attempts_left: None,
            } => t!(l, "sign-in-code-error-wrong").into_owned(),
            CodeError::Expired => t!(l, "sign-in-code-error-expired").into_owned(),
            CodeError::TooManyAttempts => t!(l, "sign-in-code-error-too-many").into_owned(),
            CodeError::RateLimited { .. } => {
                t!(l, "sign-in-code-error-rate-limited", minutes = minutes).into_owned()
            }
            CodeError::Offline => t!(l, "sign-in-code-error-offline").into_owned(),
            CodeError::Failed(m) => m.resolve(&l),
        }
    }

    /// Every text the email or code step shows, in order (tests read it).
    pub fn visible_text(&self, cx: &App) -> String {
        self.code_texts(cx).join("\n")
    }

    fn code_texts(&self, cx: &App) -> Vec<String> {
        let Some(code) = &self.code else {
            return Vec::new();
        };
        let l = i18n::gpui::localizer(cx);
        let flow = &code.flow;
        let mut out = Vec::new();
        match self.step {
            Step::Email => {
                out.push(t!(l, "sign-in-code-email-title").into_owned());
                out.push(t!(l, "sign-in-code-email-help").into_owned());
                out.push(t!(l, "sign-in-code-email-label").into_owned());
            }
            Step::Code => {
                out.push(t!(l, "sign-in-code-title").into_owned());
                out.push(
                    t!(l, "sign-in-code-sent-to", email = mask_email(flow.email())).into_owned(),
                );
                out.push(t!(l, "sign-in-code-label").into_owned());
            }
            _ => return out,
        }
        if let Some(e) = flow.error() {
            out.push(self.code_error_text(e, cx));
            if matches!(e, CodeError::NeedsBrowser { .. }) {
                out.push(t!(l, "sign-in-code-browser").into_owned());
            }
        }
        if self.step == Step::Code {
            out.push(t!(l, "sign-in-code-resend").into_owned());
            if !code.resend_ready {
                out.push(t!(l, "sign-in-code-resend-wait", seconds = code.wait_secs).into_owned());
            }
            if code.resent {
                out.push(t!(l, "sign-in-code-resent").into_owned());
            }
            out.push(t!(l, "sign-in-code-different-email").into_owned());
        }
        out.push(self.code_primary_label(cx));
        out
    }

    /// The screen title on the email and code steps.
    pub(super) fn code_title(&self, cx: &App) -> Option<String> {
        self.code.as_ref()?;
        matches!(self.step, Step::Email | Step::Code).then(|| self.code_texts(cx)[0].clone())
    }

    fn code_primary_label(&self, cx: &App) -> String {
        let l = i18n::gpui::localizer(cx);
        match self.code.as_ref().map(|c| c.flow.state()) {
            Some(FlowState::Sending) => t!(l, "sign-in-code-sending").into_owned(),
            Some(FlowState::Verifying) => t!(l, "sign-in-code-verifying").into_owned(),
            _ => t!(l, "sign-in-code-continue").into_owned(),
        }
    }

    /// The code cells: one focus stop, the next empty cell ringed while focused.
    fn code_cells(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let border = parla_tokens(cx.theme()).border_control;
        let code = self.code.as_ref().expect("code step");
        let focused = code.cells_focus.is_focused(window);
        let group = code
            .flow
            .spec()
            .and_then(|s| s.group)
            .map_or(0, usize::from);
        let active = code.flow.filled();
        let total = code.flow.cells().len();
        let busy = code.flow.state() != FlowState::Code;
        let l = i18n::gpui::localizer(cx);
        let digits = code
            .flow
            .spec()
            .is_none_or(|s| s.alphabet == CodeAlphabet::Digits);
        let name = t!(l, "sign-in-code-label").into_owned();
        let value = t!(
            l,
            "sign-in-code-cells-value",
            filled = active,
            total = total,
            kind = if digits { "digits" } else { "characters" }
        )
        .into_owned();
        let mut row = div().flex().items_center().gap(metrics::SPACE_3);
        // A busy code step dims the cells' colours, not the row: GPUI Fast
        // multiplies element opacity into every paint, so each field's border
        // drew as a darker outline over its faded fill. `faded` composites
        // each colour over the page first, which is how main's group opacity
        // drew them.
        let fade = |colour: gpui::Hsla| {
            if busy {
                themes::color::faded(colour, colors.background, 0.5)
            } else {
                colour
            }
        };
        for (i, cell) in code.flow.cells().iter().enumerate() {
            if group > 0 && i > 0 && i % group == 0 {
                row = row.child(
                    div()
                        .w(metrics::LAYOUT_CODE_DASH_W)
                        .h(metrics::LAYOUT_CODE_DASH_H)
                        .rounded_full()
                        .bg(fade(colors.border_strong)),
                );
            }
            let ring = focused && !busy && i == active;
            row = row.child(
                field_frame(ring, &colors, fade(border))
                    .when(busy, |d| {
                        d.bg(fade(colors.surface_background))
                            .text_color(fade(colors.text))
                    })
                    .justify_center()
                    .w(metrics::LAYOUT_CODE_CELL_W)
                    .h(metrics::LAYOUT_CODE_CELL_H)
                    .px(metrics::SPACE_0)
                    .text_size(metrics::FONT_SIZE_XL)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(cell.map(String::from).unwrap_or_default()),
            );
        }
        // One focus stop named by the "Confirmation code" label, its value the
        // count entered (never the code itself).
        div()
            .id("sign-in-code-cells")
            .debug_selector(|| "sign-in-code-cells".into())
            .track_focus(&code.cells_focus)
            .role(gpui::Role::TextInput)
            .aria_label(crate::a11y::disabled_label(name, busy, &l))
            .aria_value(value)
            .on_key_down(cx.listener(Self::cells_key_down))
            .when(!busy, |d| {
                d.cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(c) = &this.code {
                            window.focus(&c.cells_focus, cx);
                        }
                        cx.notify();
                    }))
            })
            .child(row)
    }

    /// The email or code step under the title.
    pub(super) fn code_body(&self, mut col: Div, window: &Window, cx: &mut Context<Self>) -> Div {
        let Some(code) = &self.code else {
            return col;
        };
        let colors = *cx.theme().colors();
        let tokens = parla_tokens(cx.theme());
        let error_colour = cx.theme().status().error;
        let border = tokens.border_control;
        let l = i18n::gpui::localizer(cx);
        let texts = self.code_texts(cx);
        let state = code.flow.state();
        let busy = matches!(state, FlowState::Sending | FlowState::Verifying);
        // The shield line sits with the intro so an error banner never pushes it
        // below the fold.
        col = col.child(
            div()
                .flex()
                .flex_col()
                .gap(metrics::SPACE_3)
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_MD)
                        .text_color(colors.text_muted)
                        .child(texts[1].clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(metrics::SPACE_3)
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(colors.text_muted)
                        .child(
                            Icon::new(IconName::Shield)
                                .size(metrics::ICON_SIZE_BASE)
                                .color(colors.text_muted),
                        )
                        .child(t!(l, "sign-in-code-shield").into_owned()),
                ),
        );
        if let Some(e) = code.flow.error() {
            col = col.child(error_banner(
                self.code_error_text(e, cx),
                error_colour,
                &colors,
            ));
            if matches!(e, CodeError::NeedsBrowser { .. }) {
                col = col.child(on_activate(
                    button(
                        "code-browser",
                        t!(l, "sign-in-code-browser").into_owned(),
                        ButtonKind::Primary,
                        true,
                        &colors,
                    )
                    .debug_selector(|| "sign-in-code-browser".into()),
                    cx,
                    |this, window, cx| this.continue_in_browser(window, cx),
                ));
            }
        }
        let label = div()
            .text_size(metrics::FONT_SIZE_XS)
            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
            .text_color(colors.text_muted)
            .child(texts[2].clone());
        if self.step == Step::Email {
            let focused = code
                .email_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window);
            col = col.child(
                div()
                    .id("field-email")
                    .debug_selector(|| "sign-in-field-email".into())
                    .flex()
                    .flex_col()
                    .gap(metrics::SPACE_3)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.focus_active(window, cx);
                        cx.notify();
                    }))
                    .child(label)
                    .child(field_frame(focused, &colors, border).child(code.email_input.clone())),
            );
        } else {
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(metrics::SPACE_3)
                    .child(label)
                    .child(self.code_cells(window, cx)),
            );
            let mut resend_row = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(metrics::SPACE_5)
                .child(on_activate(
                    button(
                        "code-resend",
                        t!(l, "sign-in-code-resend").into_owned(),
                        ButtonKind::Secondary,
                        code.resend_ready && !busy,
                        &colors,
                    )
                    .track_focus(&code.resend_focus)
                    .debug_selector(|| "sign-in-code-resend".into()),
                    cx,
                    |this, window, cx| this.resend(window, cx),
                ));
            if !code.resend_ready {
                resend_row = resend_row.child(
                    div()
                        .text_size(metrics::FONT_SIZE_SM)
                        .text_color(colors.text_muted)
                        .child(
                            t!(l, "sign-in-code-resend-wait", seconds = code.wait_secs)
                                .into_owned(),
                        ),
                );
            }
            if code.resent {
                resend_row = resend_row.child(
                    div()
                        .text_size(metrics::FONT_SIZE_SM)
                        .text_color(cx.theme().status().success)
                        .child(t!(l, "sign-in-code-resent").into_owned()),
                );
            }
            col = col.child(
                resend_row.child(on_activate(
                    div()
                        .id("code-different")
                        .debug_selector(|| "sign-in-code-different".into())
                        .track_focus(&code.different_focus)
                        .when(busy, |d| {
                            d.opacity(0.5)
                                .aria_description(crate::a11y::unavailable(&l))
                        })
                        .self_start()
                        .px(metrics::SPACE_2)
                        .rounded(metrics::RADIUS_SM)
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .focus_visible(focus_style(&colors))
                        .when(!busy, |d| d.cursor_pointer())
                        .text_size(metrics::FONT_SIZE_SM)
                        .text_color(colors.text_accent)
                        .child(t!(l, "sign-in-code-different-email").into_owned()),
                    cx,
                    |this, window, cx| {
                        if !this.code_busy() {
                            this.use_different_email(window, cx)
                        }
                    },
                )),
            );
        }
        let back = on_activate(
            button_frame("back", ButtonKind::Secondary, true, &colors)
                .gap(themes::metrics::SPACE_3)
                .child(
                    crate::icon::Icon::new(crate::icon::IconName::ArrowLeft)
                        .size(themes::metrics::ICON_SIZE_SM)
                        .color(colors.text),
                )
                .child(t!(l, "sign-in-back").into_owned())
                .track_focus(&self.back_focus),
            cx,
            |this, window, cx| match this.step {
                Step::Code => this.use_different_email(window, cx),
                _ => this.email_back(window, cx),
            },
        );
        let submit = on_activate(
            button(
                "submit",
                self.code_primary_label(cx),
                ButtonKind::Primary,
                !busy,
                &colors,
            )
            .track_focus(&self.submit_focus)
            .debug_selector(|| "sign-in-code-continue".into()),
            cx,
            |this, window, cx| match this.step {
                Step::Code => this.submit_code(window, cx),
                _ => this.submit_email(window, cx),
            },
        );
        col.child(
            div()
                .flex()
                .items_center()
                .gap(metrics::SPACE_5)
                .child(back)
                .child(div().flex_1())
                .child(submit),
        )
    }
}
