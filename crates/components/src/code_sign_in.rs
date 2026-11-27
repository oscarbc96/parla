//! Provider-neutral email-code sign-in: the user types
//! an email address, the provider emails a code, the user types or pastes it. The
//! model here has no GPUI state; the sign-in screen draws it. Codes are
//! [`domain::SecretText`] and are never logged.

use std::fmt;
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Task, Window};
use zeroize::{Zeroize, Zeroizing};

use crate::sign_in::{SignInResult, SignInText};
use crate::workspace_picker::RefreshStep;

/// The characters a code is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeAlphabet {
    Digits,
    /// ASCII letters and digits; letters are shown upper-cased.
    Alphanumeric,
}

impl CodeAlphabet {
    /// The ASCII byte stored for `c`, or `None` outside the alphabet.
    fn accept(self, c: char) -> Option<u8> {
        let ok = match self {
            CodeAlphabet::Digits => c.is_ascii_digit(),
            CodeAlphabet::Alphanumeric => c.is_ascii_alphanumeric(),
        };
        ok.then(|| c.to_ascii_uppercase() as u8)
    }
}

/// The shape of a code: `length` cells, drawn in groups of `group` when set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeSpec {
    pub length: u8,
    pub alphabet: CodeAlphabet,
    pub group: Option<u8>,
}

/// A code is on its way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeSent {
    pub spec: CodeSpec,
    /// How long before "Resend code" is offered.
    pub resend_after: Duration,
}

/// Why sending or confirming a code failed. Messages never hold a secret.
#[derive(Clone, Debug, PartialEq)]
pub enum CodeError {
    InvalidEmail,
    /// The account cannot use codes (SSO or password only): offer `option`.
    NeedsBrowser {
        option: String,
        reason: SignInText,
    },
    WrongCode {
        attempts_left: Option<u8>,
    },
    Expired,
    TooManyAttempts,
    RateLimited {
        retry_after: Option<Duration>,
    },
    Offline,
    Failed(SignInText),
}

/// What a provider implements to offer email-code sign-in.
pub trait CodeSignIn: 'static {
    fn send_code(
        &self,
        email: &str,
        resend: bool,
        cx: &mut App,
    ) -> Task<Result<CodeSent, CodeError>>;
    /// On success returns the same `SignInResult` the other options return
    /// (usually `Pick` with a `WorkspacePickerSession`).
    fn confirm_code(
        &self,
        email: &str,
        code: domain::SecretText,
        progress: UnboundedSender<RefreshStep>,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<SignInResult, CodeError>>;
    /// Drops any confirmation material held for this attempt (Back, Cancel, close).
    fn forget(&self);
}

/// Where the flow is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowState {
    Email,
    Sending,
    Code,
    Verifying,
    Done,
}

/// Wait when a rate limit gives no `Retry-After`.
pub const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);

/// The email-code flow: the address, the code cells, the state and the last error.
/// The cells hold ASCII bytes (0 is empty) and are wiped when they are cleared,
/// when the flow goes back to the email step and on drop. `Debug` shows neither
/// the address nor the code.
pub struct EmailCodeFlow {
    email: String,
    spec: Option<CodeSpec>,
    cells: Vec<u8>,
    state: FlowState,
    error: Option<CodeError>,
    resend_at: Option<Instant>,
}

impl fmt::Debug for EmailCodeFlow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(std::any::type_name::<Self>())
            .field("email", &"<redacted>")
            .field("spec", &self.spec)
            .field("filled", &self.filled())
            .field("state", &self.state)
            .field("error", &self.error)
            .finish()
    }
}

impl Drop for EmailCodeFlow {
    fn drop(&mut self) {
        self.cells.zeroize();
    }
}

impl Default for EmailCodeFlow {
    fn default() -> Self {
        Self::new()
    }
}

impl EmailCodeFlow {
    pub fn new() -> Self {
        Self {
            email: String::new(),
            spec: None,
            cells: Vec::new(),
            state: FlowState::Email,
            error: None,
            resend_at: None,
        }
    }

    pub fn set_email(&mut self, s: &str) {
        self.email = s.trim().to_string();
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    /// Syntactic only: something, one `@`, a dotted domain, no spaces.
    pub fn email_ok(&self) -> bool {
        let e = &self.email;
        if e.chars().any(char::is_whitespace) {
            return false;
        }
        let Some((local, domain)) = e.split_once('@') else {
            return false;
        };
        !local.is_empty()
            && !domain.contains('@')
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    }

    pub fn start_sending(&mut self) {
        self.state = FlowState::Sending;
        self.error = None;
    }

    /// A code is on its way: empty cells of its shape, resend offered after its wait.
    pub fn sent(&mut self, sent: CodeSent, now: Instant) {
        self.spec = Some(sent.spec);
        self.cells.zeroize();
        self.cells = vec![0; usize::from(sent.spec.length)];
        self.state = FlowState::Code;
        self.error = None;
        self.resend_at = Some(now + sent.resend_after);
    }

    pub fn start_verifying(&mut self) {
        self.state = FlowState::Verifying;
        self.error = None;
    }

    pub fn finish(&mut self) {
        self.state = FlowState::Done;
        self.cells.iter_mut().for_each(Zeroize::zeroize);
    }

    /// Back to the email step: cells, error and wait dropped, the address kept.
    pub fn back_to_email(&mut self) {
        self.state = FlowState::Email;
        self.spec = None;
        // `Vec::zeroize` wipes the bytes and empties the vector.
        self.cells.zeroize();
        self.error = None;
        self.resend_at = None;
    }

    pub fn state(&self) -> FlowState {
        self.state
    }

    pub fn spec(&self) -> Option<CodeSpec> {
        self.spec
    }

    /// Each cell's character, `None` while empty.
    pub fn cells(&self) -> Vec<Option<char>> {
        self.cells
            .iter()
            .map(|&b| (b != 0).then_some(char::from(b)))
            .collect()
    }

    pub fn filled(&self) -> usize {
        self.cells.iter().take_while(|&&b| b != 0).count()
    }

    pub fn error(&self) -> Option<&CodeError> {
        self.error.as_ref()
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    /// Ignores chars outside the alphabet and a full code.
    pub fn type_char(&mut self, c: char) -> bool {
        let Some(spec) = self.spec else {
            return false;
        };
        let Some(c) = spec.alphabet.accept(c) else {
            return false;
        };
        match self.cells.iter_mut().find(|b| **b == 0) {
            Some(cell) => {
                *cell = c;
                true
            }
            None => false,
        }
    }

    pub fn backspace(&mut self) {
        if let Some(cell) = self.cells.iter_mut().rev().find(|b| **b != 0) {
            cell.zeroize();
        }
    }

    /// Strips spaces and hyphens; a paste of the wrong length or alphabet changes nothing.
    pub fn paste(&mut self, s: &str) -> bool {
        let Some(spec) = self.spec else {
            return false;
        };
        let mut chars = Zeroizing::new(Vec::with_capacity(self.cells.len()));
        for c in s.chars().filter(|c| !c.is_whitespace() && *c != '-') {
            match spec.alphabet.accept(c) {
                Some(c) => chars.push(c),
                None => return false,
            }
        }
        if chars.len() != self.cells.len() || chars.is_empty() {
            return false;
        }
        for (cell, c) in self.cells.iter_mut().zip(chars.iter()) {
            *cell = *c;
        }
        true
    }

    /// The code, when every cell is filled.
    pub fn code(&self) -> Option<domain::SecretText> {
        if self.cells.is_empty() || self.cells.contains(&0) {
            return None;
        }
        // Built in a zeroizing buffer, wiped once `SecretText` holds its copy.
        let mut buf = Zeroizing::new(String::with_capacity(self.cells.len()));
        buf.extend(self.cells.iter().map(|&b| char::from(b)));
        Some(domain::SecretText::new(buf.as_str()))
    }

    pub fn can_resend(&self, now: Instant) -> bool {
        self.resend_at.is_none_or(|at| now >= at)
    }

    pub fn resend_at(&self) -> Option<Instant> {
        self.resend_at
    }

    /// Records a failure: a failed send goes back to the email step, a failed
    /// confirm stays on the code step with the cells cleared.
    pub fn fail(&mut self, e: CodeError, now: Instant) {
        self.state = match self.state {
            FlowState::Email | FlowState::Sending => FlowState::Email,
            _ => FlowState::Code,
        };
        self.cells.iter_mut().for_each(Zeroize::zeroize);
        match &e {
            CodeError::Expired | CodeError::TooManyAttempts => self.resend_at = Some(now),
            CodeError::RateLimited { retry_after } => {
                self.resend_at = Some(now + retry_after.unwrap_or(DEFAULT_RETRY_AFTER));
            }
            _ => {}
        }
        self.error = Some(e);
    }
}

/// `p•••@example.test`: the first letter and the domain only.
pub fn mask_email(email: &str) -> String {
    const DOTS: &str = "\u{2022}\u{2022}\u{2022}";
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}{DOTS}@{domain}")
        }
        None => DOTS.to_string(),
    }
}
