//! The write confirmation prompt: what an MCP client or a plugin wants to do, to which
//! destination, with the exact text. Pure view; the shell owns the window and
//! turns the emitted event into a [`write_confirm::Confirmation`].
//!
//! The text is shown verbatim: no markup parsing, no link or mention
//! resolution, and control or invisible characters are shown as `\u{..}`.
//! The answers are Deny, Always for this session and Allow once (board
//! McpConfirm). Deny holds the focus, so Enter and Escape deny; Allow once is
//! the primary button but only a click, or Enter or Space on it after Tab,
//! allows. A plugin's prompt has no session button; instead, unless the write
//! is a delete, an "Allow without asking" check box sits before Allow. Both
//! allowing buttons are disabled for [`ARM_DELAY`] after the
//! window gains focus and again after any keystroke other than Tab, so stray
//! typing can never complete an allow.

use std::time::Duration;

use domain::ConversationKind;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyBinding, KeyDownEvent,
    Render, Subscription, Task, Window, actions, div, prelude::*,
};
use themes::{ActiveTheme, metrics};
use write_confirm::{AllowOffer, PluginSource, ReminderKind, Requester, WriteAction, WriteRequest};

use crate::reactions::glyph;

actions!(mcp, [ApproveWrite, ApproveWriteForSession, DeclineWrite]);

/// Key context of the prompt.
pub const KEY_CONTEXT: &str = "ConfirmWrite";

/// How long the allowing buttons stay disabled after the window gains focus.
pub const ARM_DELAY: Duration = Duration::from_millis(1000);

/// Registers the default prompt key bindings (no key approves).
pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", DeclineWrite, Some(KEY_CONTEXT))]);
}

/// The user's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmWriteEvent {
    /// Allow once.
    Approved,
    /// Always for this session: this tool to this destination.
    ApprovedForSession,
    /// Allow, and remember the answer the prompt offered (plugins only).
    ApprovedAndRemember,
    Declined,
}

/// The prompt's focusable controls, in tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmButton {
    Details,
    Deny,
    /// The "Allow without asking" check box (plugins only).
    Remember,
    AllowSession,
    AllowOnce,
}

/// How a key-value row draws its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowMark {
    Plain,
    /// The note is a chip after the value (the client's transport).
    Chip,
    /// The value is code (the tool name).
    Code,
    /// A conversation: hash glyph, the name, the workspace as a subtle note.
    Conversation,
    /// A private channel: lock glyph, the name, the workspace note.
    PrivateConversation,
    /// A group direct message: people glyph, the names, the workspace note.
    GroupConversation,
    /// A direct message: person glyph, the name, the workspace note.
    DirectConversation,
    /// The user, with an avatar for this name.
    Avatar(String),
}

/// One key-value row of the prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub value: String,
    pub note: Option<String>,
    pub mark: RowMark,
}

impl Row {
    fn new(label: String, value: String) -> Self {
        Self {
            label,
            value,
            note: None,
            mark: RowMark::Plain,
        }
    }

    fn note(mut self, note: String) -> Self {
        self.note = Some(note);
        self
    }

    fn mark(mut self, mark: RowMark) -> Self {
        self.mark = mark;
        self
    }
}

/// Shows `text` verbatim with every control or invisible character (other
/// than newline and tab) written as `\u{..}`.
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c != '\n' && c != '\t' && (c.is_control() || is_invisible(c)) {
            out.push_str(&format!("\\u{{{:04x}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// Format and separator characters that render as nothing or reorder text.
fn is_invisible(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x061C | 0x180E
        | 0x200B..=0x200F
        | 0x2028..=0x202E
        | 0x2060..=0x206F
        | 0xFEFF
        | 0xFFF9..=0xFFFB
        | 0xE0000..=0xE007F)
}

/// The sentence that names what an MCP client wants, per action.
pub fn window_title(l: &i18n::Localizer, action: &WriteAction) -> String {
    match action {
        WriteAction::Send { .. } => t!(l, "confirm-write-title-send"),
        WriteAction::Schedule { .. } => t!(l, "confirm-write-title-schedule"),
        WriteAction::React { .. } => t!(l, "confirm-write-title-react"),
        WriteAction::SetStatus { .. } => t!(l, "confirm-write-title-status"),
        // MCP has no other write tools.
        _ => t!(l, "confirm-write-title-other"),
    }
    .into_owned()
}

/// The sentence that names what the requester wants, per action.
pub fn request_title(l: &i18n::Localizer, req: &WriteRequest) -> String {
    let Requester::Plugin { name, .. } = &req.requester else {
        return window_title(l, &req.action);
    };
    let plugin = name.to_string();
    match &req.action {
        WriteAction::Send { .. } => t!(l, "confirm-write-plugin-title-send", plugin = plugin),
        WriteAction::Schedule { .. } => {
            t!(l, "confirm-write-plugin-title-schedule", plugin = plugin)
        }
        WriteAction::React { .. } => t!(l, "confirm-write-plugin-title-react", plugin = plugin),
        WriteAction::SetStatus { .. } => {
            t!(l, "confirm-write-plugin-title-status", plugin = plugin)
        }
        WriteAction::Edit { .. } => t!(l, "confirm-write-plugin-title-edit", plugin = plugin),
        WriteAction::Delete { .. } => t!(l, "confirm-write-plugin-title-delete", plugin = plugin),
        WriteAction::Upload { .. } => t!(l, "confirm-write-plugin-title-upload", plugin = plugin),
        WriteAction::Reminder { kind, .. } => match kind {
            ReminderKind::Add => t!(
                l,
                "confirm-write-plugin-title-reminder-add",
                plugin = plugin
            ),
            ReminderKind::Complete => {
                t!(
                    l,
                    "confirm-write-plugin-title-reminder-complete",
                    plugin = plugin
                )
            }
            ReminderKind::Delete => {
                t!(
                    l,
                    "confirm-write-plugin-title-reminder-delete",
                    plugin = plugin
                )
            }
        },
        WriteAction::Pin { on: true, .. } => {
            t!(l, "confirm-write-plugin-title-pin", plugin = plugin)
        }
        WriteAction::Pin { on: false, .. } => {
            t!(l, "confirm-write-plugin-title-unpin", plugin = plugin)
        }
        WriteAction::Save { on: true, .. } => {
            t!(l, "confirm-write-plugin-title-save", plugin = plugin)
        }
        WriteAction::Save { on: false, .. } => {
            t!(l, "confirm-write-plugin-title-unsave", plugin = plugin)
        }
        WriteAction::MarkRead { .. } => {
            t!(l, "confirm-write-plugin-title-mark-read", plugin = plugin)
        }
        WriteAction::Presence { .. } => {
            t!(l, "confirm-write-plugin-title-presence", plugin = plugin)
        }
        WriteAction::Snooze { minutes: Some(_) } => {
            t!(l, "confirm-write-plugin-title-snooze", plugin = plugin)
        }
        WriteAction::Snooze { minutes: None } => {
            t!(l, "confirm-write-plugin-title-end-snooze", plugin = plugin)
        }
    }
    .into_owned()
}

fn source_label(l: &i18n::Localizer, source: PluginSource) -> String {
    match source {
        PluginSource::Registry => t!(l, "confirm-write-source-registry"),
        PluginSource::File => t!(l, "confirm-write-source-file"),
        PluginSource::Dev => t!(l, "confirm-write-source-dev"),
        PluginSource::BuiltIn => t!(l, "confirm-write-source-built-in"),
    }
    .into_owned()
}

pub struct ConfirmWriteView {
    req: WriteRequest,
    focus: FocusHandle,
    details_toggle: FocusHandle,
    deny: FocusHandle,
    remember_toggle: FocusHandle,
    allow_session: FocusHandle,
    allow_once: FocusHandle,
    details_open: bool,
    /// The "Allow without asking" check box is ticked.
    remember: bool,
    armed: bool,
    /// Bumped whenever the allowing buttons arm or disarm, so a press
    /// recorded in an earlier armed period can never complete in a later one.
    arm_epoch: u64,
    /// The button and arming epoch in which the current press began (mouse
    /// down or Enter/Space key down while armed).
    press: Option<(ConfirmWriteEvent, u64)>,
    arm_timer: Option<Task<()>>,
    started: bool,
    _activation: Option<Subscription>,
}

impl EventEmitter<ConfirmWriteEvent> for ConfirmWriteView {}

impl Focusable for ConfirmWriteView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ConfirmWriteView {
    pub fn new(req: impl Into<WriteRequest>, cx: &mut Context<Self>) -> Self {
        Self {
            req: req.into(),
            focus: cx.focus_handle(),
            details_toggle: cx.focus_handle().tab_stop(true),
            deny: cx.focus_handle().tab_stop(true),
            remember_toggle: cx.focus_handle().tab_stop(true),
            allow_session: cx.focus_handle().tab_stop(true),
            allow_once: cx.focus_handle().tab_stop(true),
            details_open: false,
            remember: false,
            armed: false,
            arm_epoch: 0,
            press: None,
            arm_timer: None,
            started: false,
            _activation: None,
        }
    }

    pub fn request(&self) -> &WriteRequest {
        &self.req
    }

    /// Whether the allowing buttons are enabled yet.
    pub fn can_approve(&self) -> bool {
        self.armed
    }

    /// The visible label of an allowing button. The only place that decides
    /// it: `render` and `allow_a11y_label` both go through here, so the name
    /// assistive technology hears always matches the label on screen.
    pub fn allow_label(&self, button: ConfirmButton, l: &i18n::Localizer) -> String {
        match button {
            ConfirmButton::AllowSession => t!(l, "confirm-write-allow-session"),
            _ if self.req.requester.is_plugin() => t!(l, "confirm-write-allow"),
            _ => t!(l, "confirm-write-allow-once"),
        }
        .into_owned()
    }

    /// The accessible name of an allowing button: its label, marked
    /// unavailable until the buttons arm.
    pub fn allow_a11y_label(&self, button: ConfirmButton, l: &i18n::Localizer) -> String {
        crate::a11y::disabled_label(self.allow_label(button, l), !self.armed, l).to_string()
    }

    /// The control holding the focus, if any.
    pub fn focused(&self, window: &Window) -> Option<ConfirmButton> {
        [
            (&self.details_toggle, ConfirmButton::Details),
            (&self.deny, ConfirmButton::Deny),
            (&self.remember_toggle, ConfirmButton::Remember),
            (&self.allow_session, ConfirmButton::AllowSession),
            (&self.allow_once, ConfirmButton::AllowOnce),
        ]
        .into_iter()
        .find(|(h, _)| h.is_focused(window))
        .map(|(_, b)| b)
    }

    /// Whether the Details disclosure is open.
    pub fn details_open(&self) -> bool {
        self.details_open
    }

    /// The Details text: the write's arguments as JSON, escaped for display.
    pub fn details(&self) -> String {
        escape_text(&self.req.details())
    }

    /// The window title line, per action.
    /// The font family of the details and values: `buffer_font_family`.
    pub fn code_font(&self, cx: &gpui::App) -> gpui::SharedString {
        crate::live_settings::code_font(cx)
    }

    pub fn title(&self, l: &i18n::Localizer) -> String {
        request_title(l, &self.req)
    }

    /// The check box label, when the prompt offers to remember the answer.
    pub fn remember_label(&self, l: &i18n::Localizer) -> Option<String> {
        match self.req.remember_offer() {
            AllowOffer::None => None,
            AllowOffer::InConversation(name) => {
                let name = name.trim_start_matches('#').to_string();
                Some(
                    match self.req.conversation_kind {
                        None | Some(ConversationKind::Channel) => {
                            t!(l, "confirm-write-remember-conversation", name = name)
                        }
                        Some(ConversationKind::PrivateChannel | ConversationKind::Group) => {
                            t!(l, "confirm-write-remember-named", name = name)
                        }
                        Some(ConversationKind::Direct) => {
                            t!(l, "confirm-write-remember-direct", name = name)
                        }
                    }
                    .into_owned(),
                )
            }
            AllowOffer::ForPlugin => Some(t!(l, "confirm-write-remember-plugin").into_owned()),
        }
    }

    /// A destination conversation as the prompt names it, and how its row
    /// draws: `#name` for a channel (and for MCP, which does not say), the
    /// bare name with a lock or people glyph for a private channel or group,
    /// the name with a person glyph for a direct message.
    fn destination(&self, l: &i18n::Localizer, name: &str, thread: bool) -> (String, RowMark) {
        let name = name.to_string();
        let (value, mark) = match self.req.conversation_kind {
            None | Some(ConversationKind::Channel) => {
                if thread {
                    (
                        t!(l, "confirm-write-to-thread", name = name),
                        RowMark::Plain,
                    )
                } else {
                    (
                        t!(l, "confirm-write-to", name = name),
                        RowMark::Conversation,
                    )
                }
            }
            Some(kind @ (ConversationKind::PrivateChannel | ConversationKind::Group)) => {
                if thread {
                    (
                        t!(l, "confirm-write-to-thread-named", name = name),
                        RowMark::Plain,
                    )
                } else {
                    let mark = if kind == ConversationKind::Group {
                        RowMark::GroupConversation
                    } else {
                        RowMark::PrivateConversation
                    };
                    (name.into(), mark)
                }
            }
            Some(ConversationKind::Direct) => {
                if thread {
                    (
                        t!(l, "confirm-write-to-thread-direct", name = name),
                        RowMark::Plain,
                    )
                } else {
                    (name.into(), RowMark::DirectConversation)
                }
            }
        };
        (value.into_owned(), mark)
    }

    /// The heading over the exact text, per action.
    pub fn exact_label(&self, l: &i18n::Localizer) -> String {
        let plugin = self.req.requester.is_plugin();
        match &self.req.action {
            WriteAction::SetStatus { .. } => t!(l, "confirm-write-exact-status"),
            WriteAction::Reminder { .. } => t!(l, "confirm-write-exact-reminder"),
            _ if !plugin => t!(l, "confirm-write-exact-message"),
            WriteAction::Delete { .. } => t!(l, "confirm-write-text-delete"),
            WriteAction::Pin { on: true, .. } => t!(l, "confirm-write-text-pin"),
            WriteAction::Pin { on: false, .. } => t!(l, "confirm-write-text-unpin"),
            WriteAction::Save { on: true, .. } => t!(l, "confirm-write-text-save"),
            WriteAction::Save { on: false, .. } => t!(l, "confirm-write-text-unsave"),
            WriteAction::Edit { .. } => t!(l, "confirm-write-text-edit"),
            WriteAction::React { .. } => t!(l, "confirm-write-text-react"),
            _ => t!(l, "confirm-write-exact-message"),
        }
        .into_owned()
    }

    /// The line under the title: a plugin write that sends nothing says
    /// nothing changes; everything else (and every MCP write) nothing is sent.
    pub fn subtitle(&self, l: &i18n::Localizer) -> String {
        let sends = matches!(
            self.req.action,
            WriteAction::Send { .. } | WriteAction::Schedule { .. } | WriteAction::Upload { .. }
        );
        if self.req.requester.is_plugin() && !sends {
            t!(l, "confirm-write-subtitle-change")
        } else {
            t!(l, "confirm-write-subtitle")
        }
        .into_owned()
    }

    /// Whether Allow is drawn as the danger button (a delete cannot be undone).
    pub fn allow_is_danger(&self) -> bool {
        matches!(self.req.action, WriteAction::Delete { .. })
    }

    /// Whether the "Allow without asking" check box is ticked.
    pub fn remember(&self) -> bool {
        self.remember
    }

    fn offers_remember(&self) -> bool {
        *self.req.remember_offer() != AllowOffer::None
    }

    /// What the primary allowing button answers.
    fn once_answer(&self) -> ConfirmWriteEvent {
        if self.remember && self.offers_remember() {
            ConfirmWriteEvent::ApprovedAndRemember
        } else {
            ConfirmWriteEvent::Approved
        }
    }

    fn toggle_remember(&mut self, cx: &mut Context<Self>) {
        self.remember = !self.remember;
        cx.notify();
    }

    /// The key-value rows (Client, Tool, Post to or Workspace, As, then the
    /// action's own rows), in order.
    pub fn rows(&self, l: &i18n::Localizer, zone: &jiff::tz::TimeZone) -> Vec<Row> {
        let r = &self.req;
        let mut out = match &r.requester {
            Requester::Mcp(client) => {
                // Clients reach the app through the `parla mcp` stdio shim.
                let transport = "stdio";
                let chip = match client.peer_pid {
                    Some(pid) => t!(
                        l,
                        "confirm-write-chip-pid",
                        transport = transport,
                        pid = pid.to_string()
                    ),
                    None => t!(l, "confirm-write-chip", transport = transport),
                }
                .into_owned();
                vec![
                    Row::new(
                        t!(l, "confirm-write-label-client").into_owned(),
                        client.name.to_string(),
                    )
                    .note(chip)
                    .mark(RowMark::Chip),
                    Row::new(
                        t!(l, "confirm-write-label-tool").into_owned(),
                        r.action.tool().to_string(),
                    )
                    .mark(RowMark::Code),
                ]
            }
            Requester::Plugin { name, source, .. } => vec![
                Row::new(
                    t!(l, "confirm-write-label-plugin").into_owned(),
                    name.to_string(),
                )
                .note(source_label(l, *source))
                .mark(RowMark::Chip),
                Row::new(
                    t!(l, "confirm-write-label-action").into_owned(),
                    r.action.tool().to_string(),
                )
                .mark(RowMark::Code),
            ],
        };
        let post_to = |name: &str, thread: bool| {
            let (value, mark) = self.destination(l, name, thread);
            Row::new(t!(l, "confirm-write-label-post-to").into_owned(), value)
                .note(
                    t!(
                        l,
                        "confirm-write-in-workspace",
                        name = r.workspace.name.to_string()
                    )
                    .into_owned(),
                )
                .mark(mark)
        };
        let in_conversation = |name: &str| {
            let (value, mark) = self.destination(l, name, false);
            Row::new(
                t!(l, "confirm-write-label-conversation").into_owned(),
                value,
            )
            .note(
                t!(
                    l,
                    "confirm-write-in-workspace",
                    name = r.workspace.name.to_string()
                )
                .into_owned(),
            )
            .mark(mark)
        };
        out.push(match &r.action {
            WriteAction::Send {
                conversation_name,
                thread,
                ..
            }
            | WriteAction::Schedule {
                conversation_name,
                thread,
                ..
            } => post_to(conversation_name, thread.is_some()),
            WriteAction::React {
                conversation_name, ..
            }
            | WriteAction::Upload {
                conversation_name, ..
            } => post_to(conversation_name, false),
            WriteAction::Edit {
                conversation_name, ..
            }
            | WriteAction::Delete {
                conversation_name, ..
            }
            | WriteAction::Pin {
                conversation_name, ..
            }
            | WriteAction::Save {
                conversation_name, ..
            }
            | WriteAction::MarkRead { conversation_name } => in_conversation(conversation_name),
            WriteAction::SetStatus { .. }
            | WriteAction::Reminder { .. }
            | WriteAction::Presence { .. }
            | WriteAction::Snooze { .. } => Row::new(
                t!(l, "confirm-write-label-workspace").into_owned(),
                r.workspace.name.to_string(),
            ),
        });
        out.push(match &r.as_user {
            Some(name) => Row::new(
                t!(l, "confirm-write-label-as").into_owned(),
                t!(l, "confirm-write-as-you", name = name.to_string()).into_owned(),
            )
            .mark(RowMark::Avatar(name.to_string())),
            None => Row::new(
                t!(l, "confirm-write-label-as").into_owned(),
                t!(l, "confirm-write-as-unknown").into_owned(),
            ),
        });
        match &r.action {
            WriteAction::Send {
                thread, broadcast, ..
            } => {
                if thread.is_some() && *broadcast {
                    out.push(Row::new(
                        t!(l, "confirm-write-label-also").into_owned(),
                        t!(l, "confirm-write-also-conversation").into_owned(),
                    ));
                }
            }
            WriteAction::Schedule { at, .. } => {
                let when = l.date_time(*at, zone, i18n::HourCycle::H24);
                out.push(Row::new(t!(l, "confirm-write-label-at").into_owned(), when));
            }
            WriteAction::React { emoji, .. } => {
                out.push(Row::new(
                    t!(l, "confirm-write-label-reaction").into_owned(),
                    glyph(emoji),
                ));
            }
            WriteAction::SetStatus { status } => {
                if status.is_none() {
                    out.push(Row::new(
                        t!(l, "confirm-write-label-status").into_owned(),
                        t!(l, "confirm-write-status-clear").into_owned(),
                    ));
                }
            }
            WriteAction::Upload { name, bytes, .. } => {
                out.push(
                    Row::new(
                        t!(l, "confirm-write-label-file").into_owned(),
                        name.to_string(),
                    )
                    .note(crate::files::format_size(*bytes, l)),
                );
            }
            WriteAction::Reminder { at: Some(at), .. } => {
                let when = l.date_time(*at, zone, i18n::HourCycle::H24);
                out.push(Row::new(t!(l, "confirm-write-label-at").into_owned(), when));
            }
            WriteAction::Presence { away } => {
                let value = if *away {
                    t!(l, "confirm-write-presence-away")
                } else {
                    t!(l, "confirm-write-presence-active")
                };
                out.push(Row::new(
                    t!(l, "confirm-write-label-presence").into_owned(),
                    value.into_owned(),
                ));
            }
            WriteAction::Snooze { minutes } => {
                let value = match minutes {
                    Some(m) => t!(l, "confirm-write-snooze-for", minutes = *m),
                    None => t!(l, "confirm-write-snooze-end"),
                };
                out.push(Row::new(
                    t!(l, "confirm-write-label-notifications").into_owned(),
                    value.into_owned(),
                ));
            }
            WriteAction::Edit { .. }
            | WriteAction::Delete { .. }
            | WriteAction::Reminder { at: None, .. }
            | WriteAction::Pin { .. }
            | WriteAction::Save { .. }
            | WriteAction::MarkRead { .. } => {}
        }
        out
    }

    /// The exact text to show: the message (an edit's new text), the target
    /// message excerpt, the reminder or the status text; empty when the write
    /// has none. Already escaped for display.
    pub fn body(&self) -> String {
        match &self.req.action {
            WriteAction::Send { text, .. }
            | WriteAction::Schedule { text, .. }
            | WriteAction::Edit { text, .. }
            | WriteAction::Reminder { text, .. } => escape_text(text),
            WriteAction::React {
                message_excerpt, ..
            }
            | WriteAction::Delete {
                message_excerpt, ..
            }
            | WriteAction::Pin {
                message_excerpt, ..
            }
            | WriteAction::Save {
                message_excerpt, ..
            } => escape_text(message_excerpt),
            WriteAction::Upload { .. }
            | WriteAction::MarkRead { .. }
            | WriteAction::Presence { .. }
            | WriteAction::Snooze { .. } => String::new(),
            WriteAction::SetStatus { status } => status
                .as_ref()
                .map(|s| {
                    let emoji = s.emoji.as_ref().map(glyph).unwrap_or_default();
                    escape_text(format!("{emoji} {}", s.text).trim())
                })
                .unwrap_or_default(),
        }
    }

    fn start_arming(&mut self, cx: &mut Context<Self>) {
        if self.armed || self.arm_timer.is_some() {
            return;
        }
        self.arm_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ARM_DELAY).await;
            this.update(cx, |v, cx| {
                v.armed = true;
                v.arm_epoch += 1;
                v.press = None;
                v.arm_timer = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Emits an allowing answer, only while armed.
    fn allow(&mut self, answer: ConfirmWriteEvent, cx: &mut Context<Self>) {
        if self.armed {
            cx.emit(answer);
        }
    }

    /// Records that a press on an allowing button began while armed.
    fn begin_press(&mut self, answer: ConfirmWriteEvent) {
        if self.armed {
            self.press = Some((answer, self.arm_epoch));
        }
    }

    /// Allows only when the press and the release both happened on the same
    /// button while armed.
    fn finish_press(&mut self, answer: ConfirmWriteEvent, cx: &mut Context<Self>) {
        let pressed = self.press.take();
        if self.armed && pressed == Some((answer, self.arm_epoch)) {
            cx.emit(answer);
        }
    }

    fn decline(&mut self, cx: &mut Context<Self>) {
        cx.emit(ConfirmWriteEvent::Declined);
    }

    fn toggle_details(&mut self, cx: &mut Context<Self>) {
        self.details_open = !self.details_open;
        cx.notify();
    }

    /// Locks the allowing buttons and, while the window has focus, counts
    /// down again.
    fn restart_lock(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.armed = false;
        self.arm_epoch += 1;
        self.press = None;
        self.arm_timer = None;
        if window.is_window_active() {
            self.start_arming(cx);
        }
        cx.notify();
    }

    /// Whether `e` is a deliberate activation of a focused allowing button:
    /// plain Enter or Space, not held.
    fn activates_allow(&self, e: &KeyDownEvent, window: &Window) -> bool {
        let k = &e.keystroke;
        matches!(k.key.as_str(), "enter" | "space")
            && !e.is_held
            && !k.modifiers.modified()
            && (self.allow_once.is_focused(window) || self.allow_session.is_focused(window))
    }

    fn first_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.started = true;
        window.focus(&self.deny, cx);
        if window.is_window_active() {
            self.start_arming(cx);
        }
        self._activation = Some(cx.observe_window_activation(window, |v, window, cx| {
            if window.is_window_active() {
                v.start_arming(cx);
            } else {
                // Losing focus disarms; regaining it counts down again.
                v.armed = false;
                v.arm_epoch += 1;
                v.press = None;
                v.arm_timer = None;
                cx.notify();
            }
        }));
    }

    /// An allowing button: while unarmed it is no tab stop (the focus handle
    /// is registered without one, which `button` alone cannot do because the
    /// handle decides), is named "unavailable" for assistive technology, and
    /// begin_press/finish_press ignore input; an activation counts only if
    /// its press began armed in the same epoch.
    #[allow(clippy::too_many_arguments)]
    fn allow_button(
        &self,
        id: &'static str,
        label: String,
        kind: crate::controls::ButtonKind,
        answer: ConfirmWriteEvent,
        handle: &FocusHandle,
        colors: &themes::ThemeColors,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let armed = self.armed;
        let name = crate::a11y::disabled_label(label.clone(), !armed, l);
        crate::controls::button(id, label, kind, armed, colors)
            .debug_selector(move || id.to_string())
            // The handle, not the element, registers the tab stop.
            .track_focus(&handle.clone().tab_stop(armed))
            // gpui has no disabled state for a div: the tab stop is dropped
            // above, the name says "unavailable", and begin_press/finish_press
            // both ignore input unless armed in the current epoch.
            .role(gpui::Role::Button)
            .aria_label(name)
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |v, _, _, _| v.begin_press(answer)),
            )
            .on_key_down(cx.listener(move |v, e: &KeyDownEvent, _, cx| {
                let k = &e.keystroke;
                if matches!(k.key.as_str(), "enter" | "space") {
                    if !e.is_held && !k.modifiers.modified() {
                        v.begin_press(answer);
                    }
                    cx.stop_propagation();
                }
            }))
            .on_click(cx.listener(move |v, _, _, cx| v.finish_press(answer, cx)))
    }
}

impl Render for ConfirmWriteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.started {
            self.first_frame(window, cx);
        }
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let code_font = self.code_font(cx);
        let l = i18n::gpui::localizer(cx);
        let rows = self.rows(&l, &crate::time_zone(cx));
        let title = self.title(&l);
        let body = self.body();
        let exact = self.exact_label(&l).to_uppercase();
        let is_plugin = self.req.requester.is_plugin();
        // A plugin write with no text of its own hides the block; MCP always
        // shows it, as it always has.
        let show_exact = !is_plugin
            || !body.is_empty()
            || matches!(self.req.action, WriteAction::SetStatus { .. });
        let subtitle = self.subtitle(&l);
        let danger = self.allow_is_danger();
        let remember_label = self.remember_label(&l);

        let signature = div()
            .flex()
            .flex_none()
            .h(metrics::LAYOUT_SIGNATURE_BAR)
            .child(div().flex_1().bg(tokens.logo.accent_1))
            .child(div().flex_1().bg(tokens.logo.accent_2))
            .child(div().flex_1().bg(tokens.logo.accent_3));

        let header = div()
            .flex()
            .flex_none()
            .gap(metrics::SPACE_5)
            .px(metrics::SPACE_8)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(metrics::SPACE_10)
                    .rounded(metrics::RADIUS_LG)
                    .bg(tokens.signal_soft)
                    .child(
                        crate::icon::Icon::new(if is_plugin {
                            crate::icon::IconName::Layers
                        } else {
                            crate::icon::IconName::Server
                        })
                        .size(metrics::ICON_SIZE_XL)
                        .color(tokens.signal_text),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_XL)
                            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .mt(metrics::SPACE_2)
                            .text_size(metrics::FONT_SIZE_SM)
                            .text_color(colors.text_muted)
                            .child(subtitle),
                    ),
            );

        let mut grid = div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(metrics::SPACE_4)
            .px(metrics::SPACE_8)
            .text_size(metrics::FONT_SIZE_SM);
        for row in rows {
            grid = grid.child(kv_row(row, &theme, &colors, &tokens, code_font.clone()));
        }

        let mut text_block = div()
            .id("confirm-write-text")
            .debug_selector(|| "confirm-write-text".to_string())
            .flex_none()
            .px(metrics::SPACE_5)
            .py(metrics::SPACE_5)
            .rounded(metrics::RADIUS_MD)
            .border_1()
            .border_color(colors.border)
            .bg(tokens.bg_sunken)
            .text_size(metrics::FONT_SIZE_MD);
        for line in body.split('\n') {
            text_block = text_block.child(div().min_h(metrics::SPACE_6).child(line.to_string()));
        }

        let open = self.details_open;
        let toggle = crate::controls::on_activate(
            div()
                .id("confirm-write-details")
                .debug_selector(|| "confirm-write-details".to_string())
                .track_focus(&self.details_toggle)
                .flex()
                .items_center()
                .gap(metrics::SPACE_2)
                .px(metrics::SPACE_2)
                .rounded(metrics::RADIUS_XS)
                .cursor_pointer()
                .focus_visible(crate::controls::focus_style(&colors))
                .text_size(metrics::FONT_SIZE_SM)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(colors.text_muted)
                .child(
                    crate::icon::Icon::new(if open {
                        crate::icon::IconName::ChevDown
                    } else {
                        crate::icon::IconName::ChevRight
                    })
                    .size(metrics::ICON_SIZE_SM)
                    .color(colors.text_muted),
                )
                .child(t!(l, "confirm-write-details").into_owned()),
            cx,
            |v, _, cx| v.toggle_details(cx),
        );
        let details = div()
            .flex()
            .flex_none()
            .flex_col()
            .items_start()
            .gap(metrics::SPACE_3)
            .px(metrics::SPACE_7)
            .child(toggle)
            .when(open, |d| {
                let mut pre = div()
                    .id("confirm-write-details-body")
                    .debug_selector(|| "confirm-write-details-body".to_string())
                    .w_full()
                    .px(metrics::SPACE_5)
                    .py(metrics::SPACE_4)
                    .rounded(metrics::RADIUS_MD)
                    .border_1()
                    .border_color(tokens.code_border)
                    .bg(colors.editor_background)
                    .font_family(code_font.clone())
                    .text_size(metrics::FONT_SIZE_2XS);
                for line in self.details().split('\n') {
                    pre = pre.child(div().min_h(metrics::SPACE_6).child(line.to_string()));
                }
                d.child(pre)
            });

        let hover = colors.element_hover;
        let deny = crate::controls::on_activate(
            crate::controls::button(
                "confirm-write-deny",
                t!(l, "confirm-write-deny").into_owned(),
                crate::controls::ButtonKind::Secondary,
                true,
                &colors,
            )
            .debug_selector(|| "confirm-write-deny".to_string())
            .track_focus(&self.deny)
            // Ghost (board `btn-ghost`) in the danger colour.
            .bg(gpui::transparent_black())
            .border_color(gpui::transparent_black())
            .text_color(theme.status().error)
            .hover(move |d| d.bg(hover)),
            cx,
            |v, _, cx| v.decline(cx),
        );
        // Plugins answer with the check box instead of a session button.
        let allow_session = (!is_plugin).then(|| {
            self.allow_button(
                "confirm-write-allow-session",
                self.allow_label(ConfirmButton::AllowSession, &l),
                crate::controls::ButtonKind::Secondary,
                ConfirmWriteEvent::ApprovedForSession,
                &self.allow_session.clone(),
                &colors,
                &l,
                cx,
            )
        });
        let allow_once = self
            .allow_button(
                "confirm-write-allow-once",
                self.allow_label(ConfirmButton::AllowOnce, &l),
                crate::controls::ButtonKind::Primary,
                self.once_answer(),
                &self.allow_once.clone(),
                &colors,
                &l,
                cx,
            )
            .when(danger, |b| {
                // A delete cannot be undone: the danger button (`controls::danger_button`).
                let fill = theme.status().error;
                if self.armed {
                    b.bg(fill).border_color(fill)
                } else {
                    b.bg(crate::controls::dimmed(fill))
                        .border_color(gpui::transparent_black())
                }
                .text_color(tokens.danger_text)
            });
        let remember = remember_label.map(|label| {
            let on = self.remember;
            div()
                .id("confirm-write-remember")
                .debug_selector(|| "confirm-write-remember".to_string())
                .role(gpui::Role::CheckBox)
                .aria_label(label.clone())
                .aria_toggled(if on {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .track_focus(&self.remember_toggle)
                .max_w_full()
                .flex()
                .items_center()
                .gap(metrics::SPACE_3)
                .px(metrics::SPACE_2)
                .rounded(metrics::RADIUS_XS)
                .cursor_pointer()
                .focus_visible(crate::controls::focus_style(&colors))
                .text_size(metrics::FONT_SIZE_SM)
                .child(crate::controls::check_box(
                    on,
                    &colors,
                    tokens.border_control,
                ))
                // The label wraps rather than pushing the row wider.
                .child(div().flex_1().min_w_0().child(label))
                .on_click(cx.listener(|v, _, _, cx| v.toggle_remember(cx)))
                .on_key_down(cx.listener(|v, e: &KeyDownEvent, _, cx| {
                    // Space ticks; Enter still denies (the default answer).
                    let k = &e.keystroke;
                    if k.key == "space" {
                        if !e.is_held && !k.modifiers.modified() {
                            v.toggle_remember(cx);
                        }
                        cx.stop_propagation();
                    }
                }))
        });

        // Its own row above the footer, so a long label or channel name wraps
        // instead of crowding the buttons.
        let remember = remember.map(|r| {
            div()
                .flex()
                .flex_none()
                .flex_col()
                .items_start()
                .px(metrics::SPACE_7)
                .pb(metrics::SPACE_5)
                .child(r)
        });

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|v, _: &ApproveWrite, _, cx| {
                let answer = v.once_answer();
                v.allow(answer, cx)
            }))
            .on_action(cx.listener(|v, _: &ApproveWriteForSession, _, cx| {
                if !v.req.requester.is_plugin() {
                    v.allow(ConfirmWriteEvent::ApprovedForSession, cx)
                }
            }))
            .on_action(cx.listener(|v, _: &DeclineWrite, _, cx| v.decline(cx)))
            // Before any child: every keystroke other than Tab, or a
            // deliberate Enter or Space on a focused allowing button, locks
            // the allowing buttons again.
            .capture_key_down(cx.listener(|v, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key != "tab" && !v.activates_allow(e, window) {
                    v.restart_lock(window, cx);
                }
            }))
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, window, cx| {
                // The buttons handle their own Enter and Space; a key never
                // allows here, and Enter or Space anywhere else denies.
                match e.keystroke.key.as_str() {
                    "tab" if e.keystroke.modifiers.shift => window.focus_prev(cx),
                    "tab" => window.focus_next(cx),
                    "enter" | "space"
                        if !v.allow_once.is_focused(window)
                            && !v.allow_session.is_focused(window) =>
                    {
                        v.decline(cx)
                    }
                    _ => {}
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.elevated_surface_background)
            .text_color(colors.text)
            .child(signature)
            .child(
                div()
                    .id("confirm-write-body")
                    .debug_selector(|| "confirm-write-body".to_string())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(metrics::SPACE_6)
                    .pt(metrics::SPACE_7)
                    .pb(metrics::SPACE_6)
                    .child(header)
                    .child(grid)
                    .when(show_exact, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap(metrics::SPACE_3)
                                .px(metrics::SPACE_8)
                                .child(
                                    div()
                                        .text_size(metrics::FONT_SIZE_2XS)
                                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                        .text_color(tokens.text_subtle)
                                        .child(exact),
                                )
                                .child(text_block),
                        )
                    })
                    .child(details),
            )
            .children(remember)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(metrics::SPACE_4)
                    .px(metrics::SPACE_8)
                    .py(metrics::SPACE_6)
                    .border_t_1()
                    .border_color(colors.border)
                    .bg(colors.surface_background)
                    .child(deny)
                    .child(div().flex_1())
                    .children(allow_session)
                    .child(allow_once),
            )
    }
}

/// One key-value row: the label column, then the value drawn per its mark.
fn kv_row(
    row: Row,
    theme: &themes::Theme,
    colors: &themes::ThemeColors,
    tokens: &themes::ParlaTokens,
    code_font: gpui::SharedString,
) -> gpui::Div {
    let value = div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(metrics::SPACE_3);
    let value = match row.mark {
        RowMark::Plain => value.child(row.value).when_some(row.note, |d, n| {
            d.child(div().text_color(tokens.text_subtle).child(n))
        }),
        RowMark::Chip => value
            .child(
                div()
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(row.value),
            )
            .when_some(row.note, |d, n| {
                d.child(
                    div()
                        .flex_none()
                        .px(metrics::SPACE_4)
                        .rounded(metrics::RADIUS_FULL)
                        .border_1()
                        .border_color(colors.border)
                        .bg(tokens.bg_sunken)
                        .text_color(colors.text_muted)
                        .text_size(metrics::FONT_SIZE_2XS)
                        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                        .child(n),
                )
            }),
        RowMark::Code => value.child(
            div()
                .px(metrics::SPACE_2)
                .rounded(metrics::RADIUS_XS)
                .border_1()
                .border_color(tokens.code_border)
                .bg(colors.editor_background)
                .font_family(code_font)
                .text_size(metrics::FONT_SIZE_XS)
                .child(row.value),
        ),
        RowMark::Conversation
        | RowMark::PrivateConversation
        | RowMark::GroupConversation
        | RowMark::DirectConversation => value
            .child(
                crate::icon::Icon::new(match row.mark {
                    RowMark::PrivateConversation => crate::icon::IconName::Lock,
                    RowMark::GroupConversation => crate::icon::IconName::Users,
                    RowMark::DirectConversation => crate::icon::IconName::User,
                    _ => crate::icon::IconName::Hash,
                })
                .size(metrics::ICON_SIZE_SM)
                .color(colors.text_muted),
            )
            .child(div().font_weight(metrics::FONT_WEIGHT_SEMIBOLD).child(
                if row.mark == RowMark::Conversation {
                    row.value.trim_start_matches('#').to_string()
                } else {
                    row.value
                },
            ))
            .when_some(row.note, |d, n| {
                d.child(div().text_color(tokens.text_subtle).child(n))
            }),
        RowMark::Avatar(name) => value
            .child(crate::message_row::avatar_sized(
                &name,
                theme,
                metrics::LAYOUT_AVATAR_XS,
                None,
            ))
            .child(row.value),
    };
    div()
        .flex()
        .items_center()
        .gap(metrics::SPACE_5)
        .min_h(metrics::LAYOUT_AVATAR_XS)
        .child(
            div()
                .flex_none()
                .w(metrics::LAYOUT_CONFIRM_LABEL)
                .text_size(metrics::FONT_SIZE_XS)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(tokens.text_subtle)
                .child(row.label),
        )
        .child(value)
}
