//! Multi-line message composer: a [`TextInput`] (IME, selection, undo) plus send,
//! newline, edit-last and cancel actions.

use domain::{
    Capabilities, ComposedText, ConversationId, DeliveryState, EmojiRef, Inline, Member, MessageId,
    MessageTs, MessageWindow, SlashError, UserId, parse_slash,
};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyBinding, Render,
    SharedString, Subscription, Window, actions, div, prelude::*,
};
use std::rc::Rc;
use themes::{ActiveTheme, metrics};

use crate::completion::{Completion, CompletionKind, detect};
use crate::files::{StagedFile, StagedState};
use crate::format_toolbar::{Format, format_edit};
use crate::keys::binding;
use crate::send_later::SendLaterRow;
use crate::slash_menu::{self, SlashEntry};
use crate::text_input::{TextInput, TextInputEvent};
use crate::timeline::TimelineEvent;
use crate::upload::{Refusal, refusals_text};
use i18n::{Localizer, Translate as _};

actions!(
    composer,
    [
        Send,
        PlainEnter,
        Newline,
        EditLast,
        Cancel,
        ToggleBroadcast,
        Paste,
        AcceptCompletion,
        Bold,
        Italic,
        Strike,
        Code,
        CodeBlock,
        Quote,
        BulletList,
        NumberedList,
        Link,
        SendLater
    ]
);

/// Rows shown by a picker.
const MAX_ITEMS: usize = 8;

/// Why the last attach failed; rendered at paint time so a language switch applies.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Notice {
    Refused(Vec<Refusal>),
    PasteFailed(String),
    /// A paste over the snippet cap; it waits in `Composer::oversized`.
    PasteTooLarge,
}

impl Notice {
    fn text(&self, l: &Localizer) -> String {
        match self {
            Self::Refused(r) => refusals_text(r, l),
            Self::PasteFailed(e) => t!(l, "composer-paste-failed", error = e).into_owned(),
            Self::PasteTooLarge => t!(
                l,
                "snippet-too-large",
                size = crate::files::format_size(crate::snippet::MAX_SNIPPET_BYTES as u64, l)
            )
            .into_owned(),
        }
    }
}

/// Why the last slash command did not run; rendered at paint time so a language switch applies.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SlashNotice {
    /// Not a built-in command, or one the provider cannot do.
    Unsupported(String),
    /// A known command with bad arguments.
    Usage(String),
    /// A `@name` or `#name` the workspace does not know.
    UnknownName(String),
}

/// The open slash menu.
struct SlashState {
    items: Vec<SlashEntry>,
    selected: usize,
}

/// The open Send later menu.
struct SendLaterState {
    choices: Vec<crate::send_later::SendChoice>,
    selected: usize,
    /// The date and time fields, once "Custom..." was chosen.
    custom: Option<CustomFields>,
}

struct CustomFields {
    date: Entity<TextInput>,
    time: Entity<TextInput>,
    _subs: Vec<Subscription>,
}

/// The URL field shown while a link is being added.
struct LinkPrompt {
    input: Entity<TextInput>,
    /// Where the link goes: the selection when the prompt opened.
    range: std::ops::Range<usize>,
    invalid: bool,
}

/// A long paste held back from the draft while the user chooses what to do with it.
struct SnippetOffer {
    /// The draft (conversation, thread) the paste was made in; the snippet goes only there.
    key: (ConversationId, Option<MessageTs>),
    text: String,
    lines: usize,
    title: Entity<TextInput>,
    /// Type-to-filter field of the language list.
    filter: Entity<TextInput>,
    _filter_sub: Subscription,
    /// Tracks the panel: Enter shares the snippet only while the focus is inside it.
    focus: FocusHandle,
    language: &'static crate::snippet::Language,
    /// The language is the guess, not the user's choice.
    auto: bool,
    /// The language list is open.
    menu_open: bool,
    /// Highlighted row of the open list (an index into the filtered languages).
    selected: usize,
    /// The filter text the highlight was set for.
    filter_text: String,
}

/// File name of a paste too large for a snippet, attached as a text file instead.
const OVERSIZED_PASTE_NAME: &str = "paste.txt";

/// A snippet to stage: uploaded under `name` as `text`, with `language` (a provider-neutral
/// id such as `rust`) as its syntax hint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSnippet {
    pub name: String,
    pub language: String,
    pub text: String,
    /// Upload as an ordinary text file, not a snippet (a paste over
    /// [`crate::snippet::MAX_SNIPPET_BYTES`]); `language` is then ignored.
    pub as_file: bool,
}

/// `https://` is assumed for a bare address; only web and mail links are accepted.
fn normalize_url(raw: &str) -> Option<String> {
    let u = raw.trim();
    if u.is_empty() || u.contains(char::is_whitespace) {
        return None;
    }
    let lower = u.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return (u.split_once("://").is_some_and(|(_, h)| !h.is_empty())).then(|| u.to_string());
    }
    if let Some(rest) = lower.strip_prefix("mailto:") {
        return (!rest.is_empty()).then(|| u.to_string());
    }
    // `host:8080/path` is an address; any other `scheme:` is refused.
    let scheme_like = u.split_once(':').is_some_and(|(_, after)| {
        !after.starts_with(|c: char| c.is_ascii_digit()) || u.contains("://")
    });
    (!scheme_like).then(|| format!("https://{u}"))
}

/// Key context of the composer view.
pub const KEY_CONTEXT: &str = "Composer";

/// Extra key context while the send key setting is Cmd-Enter (`send_key`).
const CMD_ENTER_CONTEXT: &str = "send_cmd_enter";

/// Registers the default composer key bindings.
pub fn init(cx: &mut App) {
    let not_cmd = format!("!{CMD_ENTER_CONTEXT}");
    let enter_sends = [KEY_CONTEXT, "&&", not_cmd.as_str()].join(" ");
    let cmd_enter_sends = [KEY_CONTEXT, "&&", CMD_ENTER_CONTEXT].join(" ");
    let mod_key = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let keys = [
        KeyBinding::new("enter", Send, Some(&enter_sends)),
        KeyBinding::new("enter", PlainEnter, Some(&cmd_enter_sends)),
        binding(
            &format!("{mod_key}-enter"),
            Send,
            Some(&cmd_enter_sends),
            cx,
        ),
        KeyBinding::new("shift-enter", Newline, Some(KEY_CONTEXT)),
        KeyBinding::new("up", EditLast, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", Cancel, Some(KEY_CONTEXT)),
        KeyBinding::new("tab", AcceptCompletion, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(KEY_CONTEXT)),
    ];
    cx.bind_keys(keys);
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let format_keys = [
        binding(&format!("{m}-b"), Bold, Some(KEY_CONTEXT), cx),
        binding(&format!("{m}-i"), Italic, Some(KEY_CONTEXT), cx),
        binding(&format!("{m}-shift-x"), Strike, Some(KEY_CONTEXT), cx),
        binding(&format!("{m}-e"), Code, Some(KEY_CONTEXT), cx),
        binding(
            &format!("{m}-alt-shift-c"),
            CodeBlock,
            Some(KEY_CONTEXT),
            cx,
        ),
        binding(&format!("{m}-shift-9"), Quote, Some(KEY_CONTEXT), cx),
        binding(&format!("{m}-shift-8"), BulletList, Some(KEY_CONTEXT), cx),
        binding(&format!("{m}-shift-7"), NumberedList, Some(KEY_CONTEXT), cx),
        binding(&format!("{m}-shift-u"), Link, Some(KEY_CONTEXT), cx),
    ];
    cx.bind_keys(format_keys);
}

/// What the composer needs from the session, so `components` does not depend on `parla`.
pub trait ComposerSource {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow>;
    fn self_user(&self) -> Option<&UserId>;
    fn capabilities(&self) -> &Capabilities;
    /// The workspace session ended: the composer is read-only until the user
    /// signs in again.
    fn read_only(&self) -> bool {
        false
    }
    /// The user is not in channel `c` yet: the composer offers Join in place
    /// of the input, and sending waits for it.
    fn must_join(&self, _c: &ConversationId) -> bool {
        false
    }
    /// Joins channel `c` (the composer's Join).
    fn join_conversation(&mut self, _c: ConversationId, _cx: &mut App) {}
    /// A join of channel `c` is on its way: Join waits for it.
    fn joining(&self, _c: &ConversationId) -> bool {
        false
    }
    /// Uploads can start now (online, session signed in). When not, Send keeps the draft.
    fn can_upload(&self) -> bool {
        true
    }
    /// Comment text of a removed failed upload, handed back once to go into the draft again.
    fn take_restored_text(
        &mut self,
        _c: &ConversationId,
        _thread: Option<&MessageTs>,
    ) -> Option<String> {
        None
    }
    /// Files attached to the draft of `c` (or of its thread `thread`), oldest first.
    fn staged(&self, _c: &ConversationId, _thread: Option<&MessageTs>) -> Vec<StagedFile> {
        Vec::new()
    }
    /// The session can stage snippets ([`Self::stage_snippet`]); when not, a long paste is
    /// plain text.
    fn supports_snippets(&self) -> bool {
        false
    }
    /// Attaches a snippet to the draft as a text file. Nothing is uploaded until
    /// [`Self::send_staged`].
    fn stage_snippet(
        &mut self,
        _c: ConversationId,
        _thread: Option<MessageTs>,
        _snippet: NewSnippet,
        _cx: &mut App,
    ) {
    }
    /// Attaches local files to the draft. Nothing is uploaded until [`Self::send_staged`].
    fn stage(
        &mut self,
        _c: ConversationId,
        _thread: Option<MessageTs>,
        _paths: Vec<std::path::PathBuf>,
        _cx: &mut App,
    ) {
    }
    /// Detaches staged file `id` from the draft.
    fn unstage(&mut self, _c: ConversationId, _thread: Option<MessageTs>, _id: u64, _cx: &mut App) {
    }
    /// Cancels the upload `id` belongs to; its files stay staged as cancelled.
    fn cancel_staged(
        &mut self,
        _c: ConversationId,
        _thread: Option<MessageTs>,
        _id: u64,
        _cx: &mut App,
    ) {
    }
    /// Sends the upload `id` belonged to again (a failed or cancelled one).
    fn retry_staged(
        &mut self,
        _c: ConversationId,
        _thread: Option<MessageTs>,
        _id: u64,
        _cx: &mut App,
    ) {
    }
    /// Uploads the ready staged files in one message, `text` as its comment.
    fn send_staged(
        &mut self,
        _c: ConversationId,
        _thread: Option<MessageTs>,
        _text: Option<String>,
        _cx: &mut App,
    ) {
    }
    /// Rows for the `@`, `#` and `:` pickers matching `query`, best first.
    fn completions(&self, _kind: CompletionKind, _query: &str) -> Vec<Completion> {
        Vec::new()
    }
    /// Emoji used before in this workspace, newest first.
    fn recent_emoji(&self) -> Vec<EmojiRef> {
        Vec::new()
    }
    /// Records that `e` was picked, so it leads the Recent list.
    fn emoji_used(&mut self, _e: EmojiRef, _cx: &mut App) {}
    /// The user typed in the draft of `c` (its thread `thread` when set). A
    /// restored draft, an edit of a sent message or clearing it is not typing.
    fn typed(&mut self, _c: ConversationId, _thread: Option<MessageTs>, _cx: &mut App) {}
    /// Who is typing in `c` (its thread `thread` when set) now, oldest first,
    /// never self; `None` for a person not resolved yet.
    fn typing(&self, _c: &ConversationId, _thread: Option<&MessageTs>) -> Vec<Option<&Member>> {
        Vec::new()
    }
    /// Display name of the provider, for "Open in ...".
    fn provider_name(&self) -> &str {
        "provider"
    }
    /// Runs a built-in slash command; errors are shown under the composer.
    fn run_slash(
        &mut self,
        _cmd: domain::SlashCommand,
        _cx: &mut App,
    ) -> Result<(), domain::SlashError> {
        Ok(())
    }
    /// Opens `c` in the provider's own client.
    fn open_in_provider(&mut self, _c: &ConversationId, _cx: &mut App) {}
    /// The provider's wire markup for composed text (plain text by default).
    fn encode(&self, t: &ComposedText) -> String {
        t.text.to_string()
    }
    /// The inverse of [`Self::encode`]: a sent message's markup as composed text, its
    /// mentions and links as spans, so editing it keeps them (plain text by default).
    fn decode(&self, markup: &str) -> ComposedText {
        ComposedText::plain(markup)
    }
    /// Custom emoji of the workspace, for the composer's emoji picker.
    fn picker_custom_emoji(&self) -> Vec<domain::CustomEmoji> {
        Vec::new()
    }
    /// The workspace's image for custom emoji `shortcode`, so the input draws
    /// it in place of `:shortcode:`. `None` paints the token as text.
    fn custom_emoji_image(&self, _shortcode: &str) -> Option<domain::ImageRef> {
        None
    }
    /// Presets, zone and capability for the Send later menu.
    fn send_later(&self, _cx: &App) -> crate::send_later::SendLaterConfig {
        crate::send_later::SendLaterConfig {
            presets: Vec::new(),
            zone: jiff::tz::TimeZone::system(),
            clock: settings::Clock::H24,
            server: self.capabilities().scheduled_send,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComposerEvent {
    Submit {
        body: ComposedText,
    },
    SubmitEdit {
        id: MessageId,
        body: ComposedText,
    },
    /// A reply in reply mode; `broadcast` also sends it to the conversation.
    SubmitReply {
        root: MessageId,
        body: ComposedText,
        broadcast: bool,
    },
    /// Send at `post_at` (unix seconds) instead of now.
    Schedule {
        body: ComposedText,
        post_at: i64,
    },
    Cancel,
}

pub struct Composer<S: ComposerSource + 'static> {
    session: gpui::Entity<S>,
    conversation: ConversationId,
    input: Entity<TextInput>,
    editing: Option<MessageId>,
    /// Set in reply mode: the thread root replies go to.
    reply_root: Option<MessageId>,
    broadcast: bool,
    upload_dir: std::path::PathBuf,
    notice: Option<Notice>,
    /// The open `@`, `#` or `:` picker.
    popup: Option<Popup>,
    /// The next text change is a restore: do not open the slash menu for it.
    quiet_slash: bool,
    /// Start of a trigger word whose picker the user closed with Escape.
    dismissed: Option<usize>,
    /// The next input change is a program setting the text (a draft, an edit), not typing.
    quiet: bool,
    /// The open `/` command menu.
    slash: Option<SlashState>,
    /// The user closed the menu with Escape; it stays closed until the text stops being a command name.
    slash_dismissed: bool,
    /// Why the last slash command did not run, with the text it was typed as.
    slash_notice: Option<(SlashNotice, String)>,
    link: Option<LinkPrompt>,
    /// The long paste waiting for a choice: snippet or plain text.
    snippet_offer: Option<SnippetOffer>,
    /// A paste too large for a snippet, held until it is attached as a file or dropped.
    oversized: Option<String>,
    /// Font family of code (the snippet offer's preview).
    code_font: SharedString,
    send_key: settings::SendKey,
    /// The send-key hint, built when settings change, never per render.
    key_hint: crate::keycap::Glyphs,
    /// hint while editing a message ("esc to cancel · enter to save"), built with `key_hint`.
    edit_key_hint: crate::keycap::Glyphs,
    /// The key context, built when the send key changes, never per render.
    key_context: gpui::KeyContext,
    send_later: Option<SendLaterState>,
    /// The emoji picker opened by the emoji button.
    emoji_picker: Option<Entity<crate::emoji_picker::EmojiPicker>>,
    /// An outside click just closed the picker (so the click on its button does not reopen it).
    picker_just_closed: bool,
    /// The text the edit started from, to leave the message alone when nothing changed.
    edit_original: Option<ComposedText>,
    _picker_subs: Vec<Subscription>,
    /// Move focus back to the text field on the next frame.
    refocus: bool,
    /// The Join of a channel the user is not in; it takes the focus while
    /// the strip shows.
    join_focus: FocusHandle,
    /// The last frame showed the Join strip (a join that lands hands the
    /// focus to the input).
    join_shown: bool,
    /// The Join strip's words, built when the language changes.
    join_text: JoinText,
    _input_changed: Subscription,
    /// Tells the session about the user's own typing (never a restored draft).
    _input_typed: Subscription,
}

/// The Join strip's words in the active language.
#[derive(Clone, PartialEq)]
struct JoinText {
    prompt: SharedString,
    join: SharedString,
    joining: SharedString,
}

impl JoinText {
    fn new(cx: &App) -> Self {
        Self {
            prompt: t!(cx, "composer-join-prompt").into_owned().into(),
            join: t!(cx, "composer-join").into_owned().into(),
            joining: t!(cx, "composer-joining").into_owned().into(),
        }
    }
}

struct Popup {
    kind: CompletionKind,
    /// Byte offset of the trigger character.
    start: usize,
    query: String,
    items: Vec<Completion>,
    selected: usize,
}

impl<S: ComposerSource + 'static> EventEmitter<ComposerEvent> for Composer<S> {}

impl<S: ComposerSource + 'static> Composer<S> {
    pub fn new(
        session: gpui::Entity<S>,
        conversation: ConversationId,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(TextInput::new);
        // Custom emoji show their image in the message box, as in a sent message.
        let images = session.clone();
        input.update(cx, |i, _| {
            i.set_span_images(Rc::new(move |inline, cx| match inline {
                Inline::Emoji(e) if e.unicode.is_none() => {
                    images.read(cx).custom_emoji_image(&e.shortcode)
                }
                _ => None,
            }));
        });
        // Repaint with the input (placeholder, focus ring).
        let sub = cx.observe(&input, |this, _, cx| {
            this.refresh_popup(cx);
            cx.notify();
        });
        let typed = cx.subscribe(&input, |this, input, _: &TextInputEvent, cx| {
            if this.convert_typed_emoji(cx) {
                return;
            }
            let i = input.read(cx);
            if !i.user_edit() || this.editing.is_some() || i.text().trim().is_empty() {
                return;
            }
            let (c, thread) = (this.conversation.clone(), this.thread_ts());
            this.session.update(cx, |s, cx| s.typed(c, thread, cx));
        });
        cx.observe_global::<settings::SettingsStore>(|this, cx| {
            let font = crate::live_settings::code_font(cx);
            if font != this.code_font {
                this.code_font = font;
                cx.notify();
            }
            let key = crate::live_settings::send_key(cx);
            let hint = Self::key_hint(key, false, cx);
            let edit_hint = Self::key_hint(key, true, cx);
            if key != this.send_key || hint != this.key_hint || edit_hint != this.edit_key_hint {
                this.send_key = key;
                this.key_hint = hint;
                this.edit_key_hint = edit_hint;
                this.key_context = Self::key_context(key);
                cx.notify();
            }
            let join_text = JoinText::new(cx);
            if join_text != this.join_text {
                this.join_text = join_text;
                cx.notify();
            }
        })
        .detach();
        Self {
            session,
            conversation,
            input,
            editing: None,
            reply_root: None,
            broadcast: false,
            upload_dir: crate::upload::uploads_dir(),
            notice: None,
            popup: None,
            dismissed: None,
            quiet: false,
            quiet_slash: false,
            slash: None,
            slash_dismissed: false,
            slash_notice: None,
            link: None,
            snippet_offer: None,
            oversized: None,
            code_font: crate::live_settings::code_font(cx),
            send_key: crate::live_settings::send_key(cx),
            key_hint: Self::key_hint(crate::live_settings::send_key(cx), false, cx),
            edit_key_hint: Self::key_hint(crate::live_settings::send_key(cx), true, cx),
            key_context: Self::key_context(crate::live_settings::send_key(cx)),
            send_later: None,
            emoji_picker: None,
            picker_just_closed: false,
            edit_original: None,
            _picker_subs: Vec::new(),
            refocus: false,
            join_focus: cx.focus_handle(),
            join_shown: false,
            join_text: JoinText::new(cx),
            _input_changed: sub,
            _input_typed: typed,
        }
    }

    /// Where pasted images are written before they are uploaded.
    pub fn set_upload_dir(&mut self, dir: std::path::PathBuf) {
        self.upload_dir = dir;
    }

    /// Why the last attach was refused, if it was, in the active language.
    pub fn notice(&self, l: &Localizer) -> Option<String> {
        self.notice.as_ref().map(|n| n.text(l))
    }

    /// The thread the draft belongs to, in reply mode.
    fn thread_ts(&self) -> Option<MessageTs> {
        self.reply_root.as_ref().map(|r| r.ts.clone())
    }

    /// Files attached to this draft.
    pub fn staged_files(&self, cx: &App) -> Vec<StagedFile> {
        let thread = self.thread_ts();
        self.session
            .read(cx)
            .staged(&self.conversation, thread.as_ref())
    }

    /// Attaches `paths` to the draft (files over 1 GB are refused). They upload on Send.
    pub fn attach_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        if self.session.read(cx).read_only() {
            return;
        }
        let planned = crate::upload::plan(&paths);
        self.notice = (!planned.refused.is_empty()).then_some(Notice::Refused(planned.refused));
        if !planned.accepted.is_empty() {
            let c = self.conversation.clone();
            let thread = self.thread_ts();
            let ok = planned.accepted.into_iter().map(|u| u.path).collect();
            self.session.update(cx, |s, cx| s.stage(c, thread, ok, cx));
        }
        cx.notify();
    }

    /// Detaches a staged file.
    pub fn remove_staged(&mut self, id: u64, cx: &mut Context<Self>) {
        let (c, thread) = (self.conversation.clone(), self.thread_ts());
        self.session
            .update(cx, |s, cx| s.unstage(c, thread, id, cx));
    }

    /// Cancels the upload `id` is part of.
    pub fn cancel_staged(&mut self, id: u64, cx: &mut Context<Self>) {
        let (c, thread) = (self.conversation.clone(), self.thread_ts());
        self.session
            .update(cx, |s, cx| s.cancel_staged(c, thread, id, cx));
    }

    /// Sends a failed or cancelled upload again.
    pub fn retry_staged(&mut self, id: u64, cx: &mut Context<Self>) {
        let (c, thread) = (self.conversation.clone(), self.thread_ts());
        self.session
            .update(cx, |s, cx| s.retry_staged(c, thread, id, cx));
    }

    /// Writes a pasted image to a temp file and attaches it.
    pub fn paste_image(&mut self, img: &gpui::Image, cx: &mut Context<Self>) {
        if self.session.read(cx).read_only() {
            return;
        }
        match crate::upload::write_pasted_image(&self.upload_dir, img) {
            Ok(p) => self.attach_paths(vec![p], cx),
            Err(e) => {
                self.notice = Some(Notice::PasteFailed(e.to_string()));
                cx.notify();
            }
        }
    }

    /// Opens the file picker and attaches the chosen files.
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update(cx, |c, cx| c.attach_paths(paths, cx)).ok();
            }
        })
        .detach();
    }

    /// The link or Send-later field holding focus, if any. Composer key
    /// bindings also reach these fields (they sit inside the composer), so the
    /// message-input actions must not run while one has focus.
    fn focused_field(&self, window: &Window, cx: &App) -> Option<Entity<TextInput>> {
        let mut fields = self
            .link
            .iter()
            .map(|l| &l.input)
            .chain(
                self.snippet_offer
                    .iter()
                    .flat_map(|o| [&o.title, &o.filter]),
            )
            .chain(
                self.send_later
                    .iter()
                    .filter_map(|s| s.custom.as_ref())
                    .flat_map(|c| [&c.date, &c.time]),
            );
        fields
            .find(|f| f.read(cx).focus_handle(cx).is_focused(window))
            .cloned()
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        // The picker has the focus: the hidden input must not take its keys.
        if self.emoji_picker.is_some() {
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let snippets = self.editing.is_none() && self.session.read(cx).supports_snippets();
        // A long paste while the offer's own fields have the focus is a second long paste.
        let second_paste = snippets
            && self.snippet_offer.is_some()
            && item
                .text()
                .is_some_and(|t| crate::snippet::offers_snippet(&t));
        if let Some(field) = self.focused_field(window, cx).filter(|_| !second_paste) {
            // A single-line field takes the text only.
            if let Some(t) = item.text() {
                let t = t.replace("\r\n", " ").replace(['\r', '\n'], " ");
                field.update(cx, |i, cx| i.replace_selection(&t, cx));
            }
            return;
        }
        for e in item.into_entries() {
            match e {
                gpui::ClipboardEntry::Image(img) => self.paste_image(&img, cx),
                gpui::ClipboardEntry::ExternalPaths(p) => self.attach_paths(p.paths().to_vec(), cx),
                gpui::ClipboardEntry::String(s) => {
                    let t = s.text().replace("\r\n", "\n").replace('\r', "\n");
                    if snippets && crate::snippet::offers_snippet(&t) {
                        self.offer_snippet(t, window, cx);
                    } else {
                        self.input.update(cx, |i, cx| i.replace_selection(&t, cx));
                    }
                }
            }
        }
    }

    /// Holds a long paste back and offers to share it as a snippet. A paste over
    /// [`crate::snippet::MAX_SNIPPET_BYTES`] is never a snippet: a notice offers to attach
    /// it as a file instead. An offer already open is not lost: its text goes into the
    /// draft before the new one opens.
    pub fn offer_snippet(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        use crate::snippet::{Paste, classify_paste};
        let (lines, language) = match classify_paste(&text) {
            Paste::Short => {
                self.input
                    .update(cx, |i, cx| i.replace_selection(&text, cx));
                return;
            }
            Paste::TooLarge => {
                self.oversized = Some(text);
                self.notice = Some(Notice::PasteTooLarge);
                cx.notify();
                return;
            }
            Paste::Snippet { lines, language } => (lines, language),
        };
        self.paste_snippet_as_text(cx);
        let title = cx.new(TextInput::new);
        title.update(cx, |i, _| i.set_single_line(true));
        let filter = cx.new(TextInput::new);
        filter.update(cx, |i, _| {
            i.set_single_line(true);
            // Up and Down move through the list, not the caret.
            i.set_intercept_down(true);
        });
        // The language list follows what is typed in its filter: a new filter text
        // highlights its first match.
        let filter_sub = cx.observe(&filter, |this, filter, cx| {
            let text = filter.read(cx).text();
            if let Some(o) = &mut this.snippet_offer
                && o.filter_text != text
            {
                o.filter_text = text.to_string();
                o.selected = 0;
            }
            cx.notify();
        });
        // A paste that is plain prose is not "detected": the hint stays quiet for it.
        let auto = language.id != "text";
        window.focus(&title.read(cx).focus_handle(cx), cx);
        self.snippet_offer = Some(SnippetOffer {
            key: (self.conversation.clone(), self.thread_ts()),
            lines,
            text,
            title,
            filter,
            _filter_sub: filter_sub,
            focus: cx.focus_handle(),
            language,
            auto,
            menu_open: false,
            selected: 0,
            filter_text: String::new(),
        });
        cx.notify();
    }

    /// Lines of the long paste being offered as a snippet, if one is.
    pub fn snippet_offer_lines(&self) -> Option<usize> {
        self.snippet_offer.as_ref().map(|o| o.lines)
    }

    /// The language the offered snippet will have.
    pub fn snippet_offer_language(&self) -> Option<&'static crate::snippet::Language> {
        self.snippet_offer.as_ref().map(|o| o.language)
    }

    /// Sets the title of the offered snippet.
    pub fn set_snippet_title(&mut self, title: &str, cx: &mut Context<Self>) {
        if let Some(o) = &self.snippet_offer {
            o.title.update(cx, |i, cx| i.set_text(title, cx));
        }
    }

    /// Sets the language of the offered snippet (a [`crate::snippet::LANGUAGES`] id).
    pub fn set_snippet_language(&mut self, id: &str, cx: &mut Context<Self>) {
        if let (Some(o), Some(lang)) = (&mut self.snippet_offer, crate::snippet::language_by_id(id))
        {
            o.language = lang;
            o.auto = false;
            o.menu_open = false;
            cx.notify();
        }
    }

    /// The languages the open list shows: those matching its filter.
    fn offer_languages(o: &SnippetOffer, cx: &App) -> Vec<&'static crate::snippet::Language> {
        crate::snippet::filter_languages(o.filter.read(cx).text())
    }

    /// Opens the language list (focus in its filter) or closes it (focus back on the title).
    fn toggle_language_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(o) = &mut self.snippet_offer else {
            return;
        };
        o.menu_open = !o.menu_open;
        if o.menu_open {
            o.filter.update(cx, |i, cx| i.set_text("", cx));
            o.filter_text.clear();
            let current = o.language.id;
            o.selected = crate::snippet::LANGUAGES
                .iter()
                .position(|l| l.id == current)
                .unwrap_or(0);
            window.focus(&o.filter.read(cx).focus_handle(cx), cx);
        } else {
            window.focus(&o.title.read(cx).focus_handle(cx), cx);
        }
        cx.notify();
    }

    /// Moves the highlight of the open language list by `delta` rows. False when it is closed.
    fn step_language(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(o) = &self.snippet_offer else {
            return false;
        };
        if !o.menu_open {
            return false;
        }
        let n = Self::offer_languages(o, cx).len();
        if let Some(o) = &mut self.snippet_offer
            && n > 0
        {
            o.selected = (o.selected as isize + delta).rem_euclid(n as isize) as usize;
        }
        cx.notify();
        true
    }

    /// Picks the highlighted language of the open list. False when it is closed.
    fn pick_language(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(o) = &self.snippet_offer else {
            return false;
        };
        if !o.menu_open {
            return false;
        }
        let pick = Self::offer_languages(o, cx).get(o.selected).map(|l| l.id);
        let title = o.title.read(cx).focus_handle(cx);
        match pick {
            Some(id) => self.set_snippet_language(id, cx),
            None => {
                if let Some(o) = &mut self.snippet_offer {
                    o.menu_open = false;
                }
            }
        }
        window.focus(&title, cx);
        cx.notify();
        true
    }

    /// Stages the offered paste as a snippet of the draft it was pasted into.
    pub fn create_snippet(&mut self, cx: &mut Context<Self>) {
        let Some(o) = self.snippet_offer.take() else {
            return;
        };
        // Never stage into another draft than the paste's.
        if o.key != (self.conversation.clone(), self.thread_ts()) {
            cx.notify();
            return;
        }
        let title = o.title.read(cx).text().to_string();
        let snippet = NewSnippet {
            name: crate::snippet::file_name(&title, o.language),
            language: o.language.id.to_string(),
            text: o.text,
            as_file: false,
        };
        let (c, thread) = o.key;
        self.session
            .update(cx, |s, cx| s.stage_snippet(c, thread, snippet, cx));
        self.refocus = true;
        cx.notify();
    }

    /// Attaches the paste that was too large for a snippet as an ordinary text file.
    pub fn attach_oversized_paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.oversized.take() else {
            return;
        };
        self.notice = None;
        let file = NewSnippet {
            name: OVERSIZED_PASTE_NAME.to_string(),
            language: crate::snippet::plain().id.to_string(),
            text,
            as_file: true,
        };
        let (c, thread) = (self.conversation.clone(), self.thread_ts());
        self.session
            .update(cx, |s, cx| s.stage_snippet(c, thread, file, cx));
        self.refocus = true;
        cx.notify();
    }

    /// Puts the offered paste in the draft as ordinary text.
    pub fn paste_snippet_as_text(&mut self, cx: &mut Context<Self>) {
        let Some(o) = self.snippet_offer.take() else {
            return;
        };
        self.input
            .update(cx, |i, cx| i.replace_selection(&o.text, cx));
        self.refocus = true;
        cx.notify();
    }

    /// Drops the offered paste.
    pub fn dismiss_snippet_offer(&mut self, cx: &mut Context<Self>) {
        if self.snippet_offer.take().is_some() {
            self.refocus = true;
            cx.notify();
        }
    }

    /// What the "is typing" line above the draft says, when anyone is typing.
    pub fn typing_text(&self, cx: &App) -> Option<String> {
        self.typing_line(cx).map(|(t, _)| t)
    }

    /// The "is typing" line and the byte ranges of the names in it (one or
    /// two people are named; three or more are "several people").
    fn typing_line(&self, cx: &App) -> Option<(String, Vec<std::ops::Range<usize>>)> {
        let thread = self.thread_ts();
        let l = cx.localizer();
        let names: Vec<String> = {
            let who = self
                .session
                .read(cx)
                .typing(&self.conversation, thread.as_ref());
            if who.is_empty() {
                return None;
            }
            who.into_iter()
                .take(3)
                .map(|m| crate::message_row::user_name(m, l))
                .collect()
        };
        let first = names.first().cloned().unwrap_or_default();
        let second = names.get(1).cloned().unwrap_or_default();
        let text = t!(
            l,
            "composer-typing",
            count = names.len(),
            first = first,
            second = second
        )
        .into_owned();
        let mut ranges = Vec::new();
        if names.len() <= 2 {
            let mut from = 0;
            for n in names.iter().filter(|n| !n.is_empty()) {
                if let Some(at) = text[from..].find(n.as_str()) {
                    let start = from + at;
                    ranges.push(start..start + n.len());
                    from = start + n.len();
                }
            }
        }
        Some((text, ranges))
    }

    /// Three fading dots and the "is typing" line, above the draft.
    fn render_typing(&self, cx: &App) -> Option<gpui::AnyElement> {
        let (text, names) = self.typing_line(cx)?;
        let label = text.clone();
        let theme = cx.theme();
        let subtle = themes::parla_tokens(theme).text_subtle;
        let name_style = gpui::HighlightStyle {
            color: Some(theme.colors().text_muted),
            font_weight: Some(metrics::FONT_WEIGHT_SEMIBOLD),
            ..Default::default()
        };
        // Full, 0.6 and 0.3 opacity: the three dots of the Main board.
        let dot = |opacity: f32| {
            div()
                .size(metrics::SPACE_2)
                .rounded_full()
                .bg(subtle)
                .opacity(opacity)
        };
        Some(
            div()
                .id("composer-typing")
                .debug_selector(|| "composer-typing".into())
                // gpui has no `aria_live` and Role::Status alone leaves AccessKit's
                // `live` off: set it on the node.
                .role(gpui::Role::Status)
                .aria_label(label)
                .a11y_synthetic_children(crate::a11y::polite_live_region)
                .flex()
                .items_center()
                .gap(metrics::SPACE_4)
                // The dots start where the draft text does: past the frame's
                // hairline border and the input's own inset.
                .pl(gpui::px(1.0) + INPUT_INSET_X)
                .pb(metrics::SPACE_2)
                .text_size(metrics::FONT_SIZE_XS)
                .text_color(subtle)
                .child(
                    div()
                        .flex()
                        .gap(metrics::SPACE_1)
                        .child(dot(1.0))
                        .child(dot(0.6))
                        .child(dot(0.3)),
                )
                .child(
                    gpui::StyledText::new(text)
                        .with_highlights(names.into_iter().map(|r| (r, name_style))),
                )
                .into_any_element(),
        )
    }

    pub fn text<'a>(&self, cx: &'a App) -> &'a str {
        self.input.read(cx).text()
    }

    /// The text with the picked mentions and emoji, as typed (untrimmed).
    pub fn composed(&self, cx: &App) -> ComposedText {
        let i = self.input.read(cx);
        ComposedText::new(i.text(), i.spans().to_vec())
    }

    /// The rows of the open `@`, `#` or `:` picker, if one is open.
    pub fn completion_items(&self) -> Option<&[Completion]> {
        self.popup.as_ref().map(|p| p.items.as_slice())
    }

    /// Opens, updates or closes the picker for the word at the caret.
    fn refresh_popup(&mut self, cx: &mut Context<Self>) {
        self.refresh_slash(cx);
        let found = {
            let i = self.input.read(cx);
            match i.caret() {
                Some(caret) if !i.is_composing() => detect(i.text(), caret, i.spans()),
                _ => None,
            }
        };
        if std::mem::take(&mut self.quiet) {
            // Restored text keeps its triggers as text until the user types a new one.
            self.dismissed = found.map(|t| t.start);
            self.set_popup(None, cx);
            return;
        }
        let Some(t) = found else {
            self.dismissed = None;
            self.set_popup(None, cx);
            return;
        };
        if self.dismissed == Some(t.start) {
            self.set_popup(None, cx);
            return;
        }
        self.dismissed = None;
        if let Some(p) = &self.popup
            && p.kind == t.kind
            && p.start == t.start
            && p.query == t.query
        {
            return;
        }
        let s = self.session.read(cx);
        let mut items = s.completions(t.kind, &t.query);
        if t.kind == CompletionKind::Emoji {
            // Recently used first, in recency order; the rest keep their match order.
            let recent = s.recent_emoji();
            let rank = |c: &Completion| match &c.inline {
                domain::Inline::Emoji(e) => recent
                    .iter()
                    .position(|r| r.shortcode == e.shortcode)
                    .unwrap_or(usize::MAX),
                _ => usize::MAX,
            };
            items.sort_by_key(rank);
        }
        items.truncate(MAX_ITEMS);
        let l = cx.localizer();
        for it in &mut items {
            if let Some(n) = it.members {
                it.members_label = Some(
                    t!(&l, "composer-mention-group", count = n)
                        .into_owned()
                        .into(),
                );
            }
        }
        let popup = (!items.is_empty()).then_some(Popup {
            kind: t.kind,
            start: t.start,
            query: t.query,
            items,
            selected: 0,
        });
        self.set_popup(popup, cx);
    }

    fn set_popup(&mut self, p: Option<Popup>, cx: &mut Context<Self>) {
        self.popup = p;
        self.sync_intercept(cx);
    }

    /// The input leaves Down to the composer while any menu is open.
    fn sync_intercept(&mut self, cx: &mut Context<Self>) {
        let open = self.popup.is_some()
            || self.slash.is_some()
            || self.send_later.as_ref().is_some_and(|s| s.custom.is_none());
        self.input.update(cx, |i, _| i.set_intercept_down(open));
    }

    fn step_popup(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(st) = self.send_later.as_mut().filter(|s| s.custom.is_none()) {
            let n = st.choices.len() as isize + 1;
            st.selected = (st.selected as isize + delta).rem_euclid(n) as usize;
            cx.notify();
        } else if let Some(p) = &mut self.popup {
            let n = p.items.len() as isize;
            p.selected = (p.selected as isize + delta).rem_euclid(n) as usize;
            cx.notify();
        } else if let Some(m) = &mut self.slash {
            let n = m.items.len() as isize;
            m.selected = (m.selected as isize + delta).rem_euclid(n) as usize;
            cx.notify();
        }
    }

    /// The rows of the open `/` command menu, if one is open.
    pub fn slash_items(&self) -> Option<&[SlashEntry]> {
        self.slash.as_ref().map(|m| m.items.as_slice())
    }

    /// Opens, updates or closes the slash menu for the text, and drops a stale notice.
    fn refresh_slash(&mut self, cx: &mut Context<Self>) {
        // Only the command query is copied: the field can hold a long draft.
        let (query, stale_notice) = {
            let i = self.input.read(cx);
            let text = i.text();
            let query = (self.editing.is_none() && !i.is_composing())
                .then(|| slash_menu::query(text))
                .flatten()
                .map(str::to_string);
            let stale = self
                .slash_notice
                .as_ref()
                .is_some_and(|(_, t)| t.as_str() != text);
            (query, stale)
        };
        if stale_notice {
            self.slash_notice = None;
        }
        let restored = std::mem::take(&mut self.quiet_slash);
        let items = match &query {
            Some(q) => slash_menu::entries(self.session.read(cx).capabilities(), q),
            None => Vec::new(),
        };
        if query.is_none() || items.is_empty() {
            self.slash_dismissed = false;
            self.slash = None;
        } else if restored {
            // A restored draft keeps its `/` as text until the user types.
            self.slash_dismissed = true;
            self.slash = None;
        } else if self.slash_dismissed {
            self.slash = None;
        } else {
            let selected = self
                .slash
                .as_ref()
                .map_or(0, |m| m.selected)
                .min(items.len() - 1);
            self.slash = Some(SlashState { items, selected });
        }
        self.sync_intercept(cx);
    }

    /// Writes the selected command name into the text. False when no menu is open.
    fn accept_slash(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(m) = self.slash.take() else {
            return false;
        };
        let Some(e) = m.items.get(m.selected) else {
            return false;
        };
        let line = format!("/{} ", e.name);
        let len = self.input.read(cx).text().len();
        self.slash_dismissed = false;
        self.input
            .update(cx, |i, cx| i.replace_range(0..len, &line, cx));
        self.sync_intercept(cx);
        cx.notify();
        true
    }

    /// Inserts the selected row in place of the trigger word. False when no picker is open.
    fn accept(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(p) = self.popup.take() else {
            return false;
        };
        let Some(item) = p.items.get(p.selected).cloned() else {
            return false;
        };
        let Some(caret) = self.input.read(cx).caret() else {
            return false;
        };
        let range = p.start..caret;
        let label = item.label.to_string();
        let emoji = match &item.inline {
            domain::Inline::Emoji(e) => Some(e.clone()),
            _ => None,
        };
        self.input.update(cx, |i, cx| match &emoji {
            // A standard emoji is its glyph, plain text; only custom ones are tokens.
            Some(e) if e.unicode.is_some() => i.replace_range(range, &format!("{label} "), cx),
            _ => {
                i.insert_span(range, &label, item.inline.clone(), cx);
                let end = i.caret().unwrap_or_default();
                i.replace_range(end..end, " ", cx);
            }
        });
        if let Some(e) = emoji {
            self.session.update(cx, |s, cx| s.emoji_used(e, cx));
        }
        self.set_popup(None, cx);
        cx.notify();
        true
    }

    fn dismiss_popup(&mut self, cx: &mut Context<Self>) -> bool {
        if self.close_send_later(cx) {
            return true;
        }
        if self.slash.take().is_some() {
            self.slash_dismissed = true;
            self.sync_intercept(cx);
            cx.notify();
            return true;
        }
        let Some(p) = self.popup.take() else {
            return false;
        };
        self.dismissed = Some(p.start);
        self.set_popup(None, cx);
        cx.notify();
        true
    }

    fn accept_completion(
        &mut self,
        _: &AcceptCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The picker has the focus: the hidden input must not take its keys.
        if self.emoji_picker.is_some() {
            return;
        }
        // Tab moves between the custom date and time fields.
        if let Some(c) = self.send_later.as_ref().and_then(|s| s.custom.as_ref()) {
            let (d, t) = (
                c.date.read(cx).focus_handle(cx),
                c.time.read(cx).focus_handle(cx),
            );
            window.focus(if d.is_focused(window) { &t } else { &d }, cx);
            return;
        }
        if !self.accept(cx) && !self.accept_slash(cx) {
            cx.propagate();
        }
    }

    fn format_action(&mut self, f: Format, window: &mut Window, cx: &mut Context<Self>) {
        // The picker has the focus: the hidden input must not take its keys.
        if self.emoji_picker.is_some() {
            return;
        }
        if self.focused_field(window, cx).is_some() {
            return;
        }
        self.apply_format(f, cx);
        window.focus(&self.input.read(cx).focus_handle(cx), cx);
    }

    fn link_action(&mut self, _: &Link, window: &mut Window, cx: &mut Context<Self>) {
        // The picker has the focus: the hidden input must not take its keys.
        if self.emoji_picker.is_some() {
            return;
        }
        self.begin_link(window, cx);
    }

    fn picker_down(&mut self, _: &crate::text_input::Down, _: &mut Window, cx: &mut Context<Self>) {
        if self.step_language(1, cx) {
            return;
        }
        self.step_popup(1, cx);
    }

    /// Applies a formatting command to the selection (or the caret).
    pub fn apply_format(&mut self, f: Format, cx: &mut Context<Self>) {
        let edit = {
            let i = self.input.read(cx);
            if i.is_composing() {
                return;
            }
            format_edit(f, i.text(), i.selection().range())
        };
        self.input
            .update(cx, |i, cx| i.edit_ranges(&edit.edits, edit.select, cx));
        cx.notify();
    }

    /// Whether the URL field of the link flow is open.
    pub fn link_prompt_open(&self) -> bool {
        self.link.is_some()
    }

    /// Opens the URL field for the current selection (or the caret).
    pub fn begin_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.link.is_some() {
            return;
        }
        let range = self.input.read(cx).selection().range();
        let input = cx.new(TextInput::new);
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.link = Some(LinkPrompt {
            input,
            range,
            invalid: false,
        });
        cx.notify();
    }

    /// Turns the selection (or the URL itself, with no selection) into a link span.
    /// False, with the prompt kept open, when `url` is not a web or mail address.
    pub fn apply_link(&mut self, url: &str, cx: &mut Context<Self>) -> bool {
        let Some(url) = normalize_url(url) else {
            if let Some(l) = &mut self.link {
                l.invalid = true;
                cx.notify();
            }
            return false;
        };
        let (text_len, current) = {
            let i = self.input.read(cx);
            (i.text().len(), i.selection().range())
        };
        let range = match self.link.take() {
            Some(l) => l.range,
            None => current,
        };
        let valid = range.end <= text_len
            && self.input.read(cx).text().is_char_boundary(range.start)
            && self.input.read(cx).text().is_char_boundary(range.end);
        let range = if valid { range } else { text_len..text_len };
        let selected = self.input.read(cx).text()[range.clone()].to_string();
        let (label, shown) = if selected.is_empty() {
            (url.clone(), None)
        } else {
            (selected.clone(), Some(selected))
        };
        let inline = Inline::Link {
            url: url.as_str().into(),
            label: shown.map(|l| l.as_str().into()),
            style: Default::default(),
        };
        self.input
            .update(cx, |i, cx| i.insert_span(range, &label, inline, cx));
        self.refocus = true;
        cx.notify();
        true
    }

    fn close_link(&mut self, cx: &mut Context<Self>) -> bool {
        if self.link.take().is_none() {
            return false;
        }
        self.refocus = true;
        cx.notify();
        true
    }

    /// Why the last slash command did not run, in the active language.
    pub fn slash_notice(&self, l: &Localizer) -> Option<String> {
        self.slash_notice.as_ref().map(|(n, _)| match n {
            SlashNotice::Unsupported(name) => t!(l, "slash-unsupported", name = name).into_owned(),
            SlashNotice::Usage(name) => slash_menu::usage(l, name),
            SlashNotice::UnknownName(name) => t!(l, "slash-unknown-name", name = name).into_owned(),
        })
    }

    /// "Open in {provider}", offered when the command is not supported here.
    pub fn slash_open_label(&self, cx: &App) -> Option<String> {
        match self.slash_notice {
            Some((SlashNotice::Unsupported(_), _)) => Some(
                t!(
                    cx,
                    "rich-open-in",
                    provider = self.session.read(cx).provider_name()
                )
                .into_owned(),
            ),
            _ => None,
        }
    }

    /// Opens the conversation in the provider's own client.
    pub fn open_in_provider(&mut self, cx: &mut Context<Self>) {
        let c = self.conversation.clone();
        self.session.update(cx, |s, cx| s.open_in_provider(&c, cx));
    }

    /// Runs the slash command `text`. True when it ran (the composer then empties).
    fn run_slash(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let name = text
            .trim_start()
            .trim_start_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let notice = |n: SlashNotice| Some((n, text.to_string()));
        let supported = slash_menu::available(self.session.read(cx).capabilities(), &name);
        self.slash_notice = match parse_slash(text) {
            None => return false,
            Some(_) if !supported => notice(SlashNotice::Unsupported(name)),
            Some(Err(SlashError::Unknown(n))) => notice(SlashNotice::Unsupported(n)),
            Some(Err(SlashError::Usage(_))) => notice(SlashNotice::Usage(name)),
            Some(Err(SlashError::UnknownName(n))) => notice(SlashNotice::UnknownName(n)),
            Some(Ok(cmd)) => match self.session.update(cx, |s, cx| s.run_slash(cmd, cx)) {
                Ok(()) => None,
                Err(SlashError::Unknown(n)) => notice(SlashNotice::Unsupported(n)),
                Err(SlashError::Usage(_)) => notice(SlashNotice::Usage(name)),
                Err(SlashError::UnknownName(n)) => notice(SlashNotice::UnknownName(n)),
            },
        };
        let ran = self.slash_notice.is_none();
        if ran {
            self.reset(cx);
        }
        cx.notify();
        ran
    }

    /// The text input, for hosts that need its selection or composition state.
    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// Switches reply mode on (`Some(root)`) or off.
    pub fn set_reply_root(&mut self, root: Option<MessageId>, cx: &mut Context<Self>) {
        self.reply_root = root;
        self.broadcast = false;
        cx.notify();
    }

    pub fn reply_root(&self) -> Option<&MessageId> {
        self.reply_root.as_ref()
    }

    /// Whether the reply will also be sent to the conversation.
    pub fn broadcast(&self) -> bool {
        self.broadcast
    }

    /// Replaces the draft text and moves the caret to its end.
    pub fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.quiet = true;
        self.quiet_slash = true;
        self.input.update(cx, |i, cx| i.set_text(&text, cx));
        cx.notify();
    }

    /// Replaces the draft with `t`, its picked mentions, emoji and links as units.
    pub fn set_composed(&mut self, t: &ComposedText, cx: &mut Context<Self>) {
        self.quiet = true;
        self.quiet_slash = true;
        self.input
            .update(cx, |i, cx| i.set_composed(&t.text, &t.spans, cx));
        cx.notify();
    }

    fn toggle_broadcast(&mut self, _: &ToggleBroadcast, _: &mut Window, cx: &mut Context<Self>) {
        // The picker has the focus: the hidden input must not take its keys.
        if self.emoji_picker.is_some() {
            return;
        }
        if self.reply_root.is_some() {
            self.broadcast = !self.broadcast;
            cx.notify();
        }
    }

    /// The message being edited, if the composer is in edit mode.
    pub fn editing(&self) -> Option<&MessageId> {
        self.editing.as_ref()
    }

    pub fn set_conversation(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        if c != self.conversation {
            self.conversation = c;
            self.reset(cx);
            // Menus and notices belong to the conversation that was left.
            self.link = None;
            self.send_later = None;
            self.emoji_picker = None;
            self._picker_subs.clear();
            self.popup = None;
            self.dismissed = None;
            self.slash = None;
            self.slash_dismissed = false;
            self.slash_notice = None;
            self.notice = None;
            self.snippet_offer = None;
            self.oversized = None;
            cx.notify();
        }
    }

    /// The font family of code shown in the composer (a test hook).
    pub fn code_font(&self) -> &SharedString {
        &self.code_font
    }

    /// Sets the font family of code shown in the composer (the code font setting).
    pub fn set_code_font(&mut self, family: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.code_font = family.into();
        cx.notify();
    }

    /// The emoji picker, while the emoji button's picker is open.
    pub fn emoji_picker(&self) -> Option<&Entity<crate::emoji_picker::EmojiPicker>> {
        self.emoji_picker.as_ref()
    }

    /// Opens the emoji picker, or closes it when it is open.
    pub fn toggle_emoji_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::emoji_picker::{EmojiPicked, EmojiPicker, PickerDismissed};
        if self.emoji_picker.take().is_some() {
            self._picker_subs.clear();
            self.refocus = true;
            cx.notify();
            return;
        }
        let (custom, enabled, recent) = {
            let s = self.session.read(cx);
            (
                s.picker_custom_emoji(),
                s.capabilities().custom_emoji,
                s.recent_emoji(),
            )
        };
        let picker = cx.new(|cx| {
            let mut p = EmojiPicker::new(custom, enabled, cx);
            p.set_recent(recent, cx);
            p
        });
        self._picker_subs = vec![
            cx.subscribe(&picker, |this, _, ev: &EmojiPicked, cx| {
                this.insert_emoji(ev.0.clone(), cx);
            }),
            cx.subscribe(&picker, |this, _, _: &PickerDismissed, cx| {
                this.emoji_picker = None;
                this._picker_subs.clear();
                this.refocus = true;
                cx.notify();
            }),
        ];
        window.focus(&picker.focus_handle(cx), cx);
        self.emoji_picker = Some(picker);
        cx.notify();
    }

    /// Closes the picker for a press outside it, remembering that for the press's own handlers.
    fn close_emoji_picker_by_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.emoji_picker.take().is_none() {
            return;
        }
        self._picker_subs.clear();
        self.picker_just_closed = true;
        self.refocus = true;
        let this = cx.entity();
        // The press reaches the button's handler before this runs.
        window.defer(cx, move |_, cx| {
            this.update(cx, |c, _| c.picker_just_closed = false)
        });
        cx.notify();
    }

    /// Turns the `:name:` the user just typed (the closing colon) into the
    /// custom emoji span or the standard emoji glyph. Pasting, `10:30:` and
    /// unknown codes stay text. True when it converted something.
    fn convert_typed_emoji(&mut self, cx: &mut Context<Self>) -> bool {
        let (typed, text, spans, caret, composing) = {
            let i = self.input.read(cx);
            // Most edits are not a typed closing colon: leave before copying anything.
            match (i.typed_range(), i.caret()) {
                (Some(t), Some(c)) if t.end == c && i.text()[..c].ends_with(':') => {}
                _ => return false,
            }
            (
                i.typed_range(),
                i.text().to_string(),
                i.spans().to_vec(),
                i.caret(),
                i.is_composing(),
            )
        };
        let (Some(typed), Some(caret)) = (typed, caret) else {
            return false;
        };
        if composing || typed.end != caret || !text[..caret].ends_with(':') {
            return false;
        }
        // The opening colon starts a word, as the `:` picker's trigger rule.
        let start = text[..caret]
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let word = &text[start..caret];
        let Some(code) = word
            .strip_prefix(':')
            .and_then(|w| w.strip_suffix(':'))
            .filter(|c| {
                !c.is_empty()
                    && c.chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '+' | '-'))
            })
        else {
            return false;
        };
        // A picked span never reopens or converts: it is already a unit.
        if spans
            .iter()
            .any(|s| s.range.start < caret && start < s.range.end)
        {
            return false;
        }
        let custom = {
            let s = self.session.read(cx);
            (s.capabilities().custom_emoji && s.custom_emoji_image(code).is_some()).then(|| {
                EmojiRef {
                    shortcode: code.into(),
                    unicode: None,
                    skin_tone: None,
                }
            })
        };
        if let Some(e) = custom {
            let label = format!(":{}:", e.shortcode);
            self.input.update(cx, |i, cx| {
                i.insert_span(start..caret, &label, Inline::Emoji(e.clone()), cx)
            });
            self.session.update(cx, |s, cx| s.emoji_used(e, cx));
            cx.notify();
            return true;
        }
        let Some(glyph) = emojis::get_by_shortcode(code) else {
            return false;
        };
        let unicode: std::sync::Arc<str> = glyph.as_str().into();
        let e = EmojiRef {
            shortcode: code.into(),
            unicode: Some(unicode.clone()),
            skin_tone: None,
        };
        self.input
            .update(cx, |i, cx| i.replace_range(start..caret, &unicode, cx));
        self.session.update(cx, |s, cx| s.emoji_used(e, cx));
        cx.notify();
        true
    }

    /// Inserts `e` at the caret (or over the selection) and closes the picker: a
    /// standard emoji as its character, a custom one as a `:name:` token.
    fn insert_emoji(&mut self, e: EmojiRef, cx: &mut Context<Self>) {
        self.emoji_picker = None;
        self._picker_subs.clear();
        let range = self.input.read(cx).selection().range();
        self.input.update(cx, |i, cx| match &e.unicode {
            Some(u) => i.replace_range(range, u, cx),
            None => i.insert_span(
                range,
                &format!(":{}:", e.shortcode),
                Inline::Emoji(e.clone()),
                cx,
            ),
        });
        self.session.update(cx, |s, cx| s.emoji_used(e, cx));
        self.refocus = true;
        cx.notify();
    }

    /// Enters edit mode on `id` with its current `text`.
    pub fn begin_edit(&mut self, id: MessageId, text: String, cx: &mut Context<Self>) {
        let composed = self.session.read(cx).decode(&text);
        self.set_composed(&composed, cx);
        self.edit_original = trimmed_body(&composed);
        self.editing = Some(id);
        cx.notify();
    }

    /// Wires a timeline's `EditRequested` into edit mode (subscribe from the host view).
    pub fn handle_timeline_event(&mut self, ev: &TimelineEvent, cx: &mut Context<Self>) {
        match ev {
            TimelineEvent::EditRequested { id, text } => {
                self.begin_edit(id.clone(), text.clone(), cx);
            }
            TimelineEvent::DeleteRequested { .. }
            | TimelineEvent::DeleteCancelled
            | TimelineEvent::OpenThread(_) => {}
        }
    }

    /// Puts back the comment of a failed upload whose files were all removed, before whatever
    /// was typed since.
    fn take_restored_text(&mut self, cx: &mut Context<Self>) {
        let (c, thread) = (self.conversation.clone(), self.thread_ts());
        let Some(text) = self
            .session
            .update(cx, |s, _| s.take_restored_text(&c, thread.as_ref()))
        else {
            return;
        };
        let now = self.input.read(cx).text().to_string();
        let merged = if now.trim().is_empty() {
            text
        } else {
            [text, now].join("\n")
        };
        self.set_text(merged, cx);
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.set_text("", cx));
        self.editing = None;
        self.edit_original = None;
    }

    fn key_context(send_key: settings::SendKey) -> gpui::KeyContext {
        let mut ctx = gpui::KeyContext::default();
        ctx.add(KEY_CONTEXT);
        if send_key == settings::SendKey::CmdEnter {
            ctx.add(CMD_ENTER_CONTEXT);
        }
        ctx
    }

    /// The hint the footer shows when nothing is staged: the edit hint while
    /// editing a message, the send-key pair otherwise.
    pub fn footer_hint(&self) -> &SharedString {
        self.footer_glyphs().text()
    }

    /// The footer hint with its icons and spoken label, built when settings
    /// change, never per render.
    fn footer_glyphs(&self) -> &crate::keycap::Glyphs {
        if self.editing.is_some() {
            &self.edit_key_hint
        } else {
            &self.key_hint
        }
    }

    /// "⏎ send · ⇧⏎ new line", or the Cmd-Enter pair, per send key setting;
    /// while `editing`, "esc to cancel · enter to save" (the MessageStates board).
    fn key_hint(send_key: settings::SendKey, editing: bool, cx: &App) -> crate::keycap::Glyphs {
        use crate::keycap::{RETURN, SHIFT_RETURN};
        let cmd = if cfg!(target_os = "macos") {
            t!(cx, "composer-key-cmd-enter-mac").into_owned()
        } else {
            t!(cx, "composer-key-cmd-enter").into_owned()
        };
        if editing {
            let save = match send_key {
                settings::SendKey::Enter => t!(cx, "composer-key-enter").into_owned(),
                settings::SendKey::CmdEnter => cmd,
            };
            return crate::keycap::Glyphs::new(
                t!(cx, "composer-edit-key-hint", save = save.as_str()).into_owned(),
                cx,
            );
        }
        let (send, newline) = match send_key {
            settings::SendKey::Enter => (RETURN.to_string(), SHIFT_RETURN),
            settings::SendKey::CmdEnter => (cmd, RETURN),
        };
        crate::keycap::Glyphs::new(
            t!(
                cx,
                "composer-key-hint",
                send = send.as_str(),
                newline = newline
            )
            .into_owned(),
            cx,
        )
    }

    fn send(&mut self, _: &Send, window: &mut Window, cx: &mut Context<Self>) {
        self.submit(false, window, cx);
    }

    /// Enter while Cmd-Enter sends: it still drives an open menu or ends an IME
    /// composition, otherwise it makes a newline.
    fn plain_enter(&mut self, _: &PlainEnter, window: &mut Window, cx: &mut Context<Self>) {
        self.submit(true, window, cx);
    }

    /// Send, or with `plain_enter` the Enter-key variant (see [`Self::plain_enter`]).
    fn submit(&mut self, plain_enter: bool, window: &mut Window, cx: &mut Context<Self>) {
        let s = self.session.read(cx);
        if s.read_only() || s.must_join(&self.conversation) {
            return;
        }
        if let Some(st) = &self.send_later {
            if st.custom.is_some() {
                self.submit_custom(cx);
            } else {
                let ix = st.selected;
                self.choose_send_later(ix, window, cx);
            }
            return;
        }
        if self.accept(cx) {
            return;
        }
        // Enter in the open language list picks the highlighted language.
        if self.pick_language(window, cx) {
            return;
        }
        // Enter inside the offer shares the snippet; in the message box it sends as usual.
        if self
            .snippet_offer
            .as_ref()
            .is_some_and(|o| o.focus.contains_focused(window, cx))
        {
            self.create_snippet(cx);
            return;
        }
        if let Some(l) = &self.link {
            let url = l.input.read(cx).text().to_string();
            self.apply_link(&url, cx);
            return;
        }
        if let Some(m) = &self.slash {
            // Enter on a partly typed name completes it; on a whole name it runs the command.
            let typed = slash_menu::query(self.input.read(cx).text()).unwrap_or("");
            if m.items
                .get(m.selected)
                .is_some_and(|e| !e.name.eq_ignore_ascii_case(typed))
            {
                self.accept_slash(cx);
                return;
            }
        }
        // Enter that ends an IME composition commits it; it does not send.
        if self.input.read(cx).is_composing() {
            self.input.update(cx, |i, cx| i.commit_composition(cx));
            return;
        }
        if plain_enter {
            self.newline(&Newline, window, cx);
            return;
        }
        let body = trimmed_body(&self.composed(cx));
        // `/shrug [text]` is the text with a shrug after it: an ordinary message from here on.
        let shrug = body
            .as_ref()
            .filter(|_| self.editing.is_none())
            .and_then(slash_menu::expand_shrug);
        let is_shrug = shrug.is_some();
        let body = shrug.or(body);
        // A command is a command even with files staged; it never becomes the files' comment.
        if self.editing.is_none()
            && let Some(b) = &body
            && b.text.starts_with('/')
            && !is_shrug
        {
            let line = b.text.to_string();
            self.run_slash(&line, cx);
            return;
        }
        if self.editing.is_none() {
            let staged = self.staged_files(cx);
            // Send waits for files that are still going out.
            if staged
                .iter()
                .any(|f| matches!(f.state, StagedState::Uploading(_)))
            {
                return;
            }
            // Files ready on the draft go out with the text as their comment; the text may be empty.
            if staged.iter().any(|f| f.state == StagedState::Ready) {
                let text = body.map(|b| ComposerSource::encode(self.session.read(cx), &b));
                let accepted = self.session.read(cx).can_upload();
                let (c, thread) = (self.conversation.clone(), self.thread_ts());
                self.session
                    .update(cx, |s, cx| s.send_staged(c, thread, text, cx));
                // When the session refuses (offline) it says why and the draft stays as typed.
                if accepted {
                    // The files carry no broadcast; the toggle does not outlive the reply.
                    self.broadcast = false;
                    self.reset(cx);
                }
                cx.notify();
                return;
            }
        }
        let Some(body) = body else {
            return;
        };
        // An edit that changes nothing leaves the message alone: no update, so no token is rewritten.
        if self.editing.is_some() && self.edit_original.as_ref() == Some(&body) {
            self.reset(cx);
            cx.emit(ComposerEvent::Cancel);
            cx.notify();
            return;
        }
        let ev = match (self.editing.take(), self.reply_root.clone()) {
            (Some(id), _) => ComposerEvent::SubmitEdit { id, body },
            (None, Some(root)) => ComposerEvent::SubmitReply {
                root,
                body,
                broadcast: std::mem::take(&mut self.broadcast),
            },
            (None, None) => ComposerEvent::Submit { body },
        };
        self.reset(cx);
        cx.emit(ev);
        cx.notify();
    }

    /// Opens the Send later menu for the text in the field. Nothing opens for
    /// an empty field, an edit, a reply or a slash command.
    pub fn open_send_later(&mut self, cx: &mut Context<Self>) {
        if self.editing.is_some() || self.reply_root.is_some() {
            return;
        }
        let Some(body) = trimmed_body(&self.composed(cx)) else {
            return;
        };
        if body.text.starts_with('/') {
            return;
        }
        let cfg = self.session.read(cx).send_later(cx);
        let now = crate::now(cx).to_zoned_datetime(cfg.zone.clone());
        self.popup = None;
        self.slash = None;
        self.send_later = Some(SendLaterState {
            choices: crate::send_later::preset_choices(&cfg.presets, &now),
            selected: 0,
            custom: None,
        });
        self.sync_intercept(cx);
        cx.notify();
    }

    pub fn send_later_open(&self) -> bool {
        self.send_later.is_some()
    }

    /// The rows of the open Send later menu: the presets with their times in
    /// the schedule zone, then "Custom...".
    pub fn send_later_rows(&self, cx: &App) -> Option<Vec<SendLaterRow>> {
        let cfg = self.session.read(cx).send_later(cx);
        self.send_later_rows_with(&cfg, cx)
    }

    fn send_later_rows_with(
        &self,
        cfg: &crate::send_later::SendLaterConfig,
        cx: &App,
    ) -> Option<Vec<SendLaterRow>> {
        let st = self.send_later.as_ref()?;
        let now = crate::now(cx).as_second();
        let l = cx.localizer();
        let mut rows: Vec<SendLaterRow> = st
            .choices
            .iter()
            .map(|c| SendLaterRow {
                label: crate::send_later::time_label(c.at, cfg, l),
                at: Some(c.at),
                disabled: (c.at > now + domain::MAX_AHEAD_DAYS * 86_400).then(|| {
                    t!(l, "send-later-too-far", days = domain::MAX_AHEAD_DAYS).into_owned()
                }),
            })
            .collect();
        rows.push(SendLaterRow {
            label: t!(l, "send-later-custom").into_owned(),
            at: None,
            disabled: None,
        });
        Some(rows)
    }

    /// The "Schedule locally" note, when the provider cannot schedule this message.
    pub fn send_later_local_label(&self, cx: &App) -> Option<String> {
        let cfg = self.session.read(cx).send_later(cx);
        self.send_later_local_label_with(&cfg, cx)
    }

    fn send_later_local_label_with(
        &self,
        cfg: &crate::send_later::SendLaterConfig,
        cx: &App,
    ) -> Option<String> {
        self.send_later.as_ref()?;
        crate::send_later::needs_local(false, cfg.server)
            .then(|| t!(cx, "send-later-local").into_owned())
    }

    /// Schedules at row `ix` of the menu (the last row opens the custom fields).
    pub fn choose_send_later(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(st) = &self.send_later else {
            return;
        };
        if let Some(c) = st.choices.get(ix) {
            let at = c.at;
            let now = crate::now_secs(cx);
            if at > now + domain::MAX_AHEAD_DAYS * 86_400 {
                return;
            }
            self.emit_schedule(at, cx);
        } else {
            self.begin_custom(window, cx);
        }
    }

    fn begin_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cfg = self.session.read(cx).send_later(cx);
        let now = crate::now(cx).to_zoned_datetime(cfg.zone);
        let (d, t) = crate::datetime_picker::default_fields(&now);
        let date = cx.new(TextInput::new);
        let time = cx.new(TextInput::new);
        date.update(cx, |i, cx| i.set_text(&d, cx));
        time.update(cx, |i, cx| i.set_text(&t, cx));
        // Typing in a field repaints the check line.
        let subs = vec![
            cx.observe(&date, |_, _, cx| cx.notify()),
            cx.observe(&time, |_, _, cx| cx.notify()),
        ];
        let focus = date.read(cx).focus_handle(cx);
        if let Some(st) = &mut self.send_later {
            st.custom = Some(CustomFields {
                date,
                time,
                _subs: subs,
            });
        }
        window.focus(&focus, cx);
        cx.notify();
    }

    /// What the custom fields say, while they are open.
    pub fn custom_check(&self, cx: &App) -> Option<crate::CustomCheck> {
        let cfg = self.session.read(cx).send_later(cx);
        self.custom_check_with(&cfg, cx)
    }

    fn custom_check_with(
        &self,
        cfg: &crate::send_later::SendLaterConfig,
        cx: &App,
    ) -> Option<crate::CustomCheck> {
        let c = self.send_later.as_ref()?.custom.as_ref()?;
        let now = crate::now(cx).to_zoned_datetime(cfg.zone.clone());
        Some(crate::check_custom(
            c.date.read(cx).text(),
            c.time.read(cx).text(),
            &now,
        ))
    }

    pub fn set_custom(&mut self, date: &str, time: &str, cx: &mut Context<Self>) {
        if let Some(c) = self.send_later.as_ref().and_then(|s| s.custom.as_ref()) {
            c.date.update(cx, |i, cx| i.set_text(date, cx));
            c.time.update(cx, |i, cx| i.set_text(time, cx));
        }
    }

    /// Schedules at the custom time when it is valid; otherwise the fields
    /// stay open with the reason shown.
    pub fn submit_custom(&mut self, cx: &mut Context<Self>) {
        if let Some(crate::CustomCheck::Ok(at)) = self.custom_check(cx) {
            self.emit_schedule(at, cx);
        }
    }

    fn emit_schedule(&mut self, post_at: i64, cx: &mut Context<Self>) {
        let Some(body) = trimmed_body(&self.composed(cx)) else {
            return;
        };
        self.send_later = None;
        self.sync_intercept(cx);
        self.reset(cx);
        cx.emit(ComposerEvent::Schedule { body, post_at });
        cx.notify();
    }

    fn close_send_later(&mut self, cx: &mut Context<Self>) -> bool {
        if self.send_later.take().is_none() {
            return false;
        }
        self.sync_intercept(cx);
        self.refocus = true;
        cx.notify();
        true
    }

    fn send_later_action(&mut self, _: &SendLater, window: &mut Window, cx: &mut Context<Self>) {
        // The picker has the focus: the hidden input must not take its keys.
        if self.emoji_picker.is_some() {
            return;
        }
        if self.send_later.is_some() {
            self.close_send_later(cx);
        } else {
            self.open_send_later(cx);
            // A click on the button moved focus away: the menu's keys need the field.
            let focus = self.input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        // The picker has the focus: the hidden input must not take its keys.
        if self.emoji_picker.is_some() {
            return;
        }
        if self.focused_field(window, cx).is_some() {
            return;
        }
        self.input.update(cx, |i, cx| i.replace_selection("\n", cx));
    }

    fn edit_last(&mut self, _: &EditLast, window: &mut Window, cx: &mut Context<Self>) {
        // The picker has the focus: the hidden input must not take its keys.
        if self.emoji_picker.is_some() {
            return;
        }
        if self.step_language(-1, cx) {
            return;
        }
        if self.focused_field(window, cx).is_some() {
            return;
        }
        if self.popup.is_some()
            || self.slash.is_some()
            || self.send_later.as_ref().is_some_and(|s| s.custom.is_none())
        {
            self.step_popup(-1, cx);
            return;
        }
        if !self.text(cx).is_empty() || self.editing.is_some() || self.reply_root.is_some() {
            // Up is the caret key whenever there is nothing to edit.
            self.input.update(cx, |i, cx| i.move_up(cx));
            return;
        }
        let s = self.session.read(cx);
        if !s.capabilities().edit {
            return;
        }
        let me = s.self_user();
        let last = s.window(&self.conversation).and_then(|w| {
            w.messages().iter().rev().find(|m| {
                m.author.is_some() && m.author.as_ref() == me && m.state == DeliveryState::Delivered
            })
        });
        if let Some(m) = last {
            let (id, text) = (m.id.clone(), m.text.to_string());
            self.begin_edit(id, text, cx);
        }
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_link(cx) || self.dismiss_popup(cx) {
            return;
        }
        // Escape with the language list open closes only the list.
        if self.snippet_offer.as_ref().is_some_and(|o| o.menu_open) {
            self.toggle_language_menu(window, cx);
            return;
        }
        // Escape on the offer gives the paste back as text: it is never lost.
        if self.snippet_offer.is_some() {
            self.paste_snippet_as_text(cx);
            return;
        }
        if self.editing.is_some() {
            self.reset(cx);
            cx.emit(ComposerEvent::Cancel);
            cx.notify();
        } else {
            cx.propagate();
        }
    }
}

/// `t` without leading and trailing whitespace (spans shifted), `None` when nothing is left.
fn trimmed_body(t: &ComposedText) -> Option<ComposedText> {
    let text: &str = &t.text;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lead = text.len() - text.trim_start().len();
    let end = lead + trimmed.len();
    let spans = t
        .spans
        .iter()
        .filter(|s| s.range.start >= lead && s.range.end <= end)
        .map(|s| domain::ComposedSpan {
            range: s.range.start - lead..s.range.end - lead,
            inline: s.inline.clone(),
        })
        .collect();
    Some(ComposedText::new(trimmed, spans))
}

impl<S: ComposerSource + 'static> Focusable for Composer<S> {
    /// The input; Join while the user is not in the channel.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.session.read(cx).must_join(&self.conversation) {
            return self.join_focus.clone();
        }
        self.input.read(cx).focus_handle(cx)
    }
}

impl<S: ComposerSource + 'static> Composer<S> {
    fn render_popup(&self, p: &Popup, cx: &mut Context<Self>) -> impl IntoElement + use<S> {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let mut list = div()
            .debug_selector(|| "completion-popup".into())
            .flex()
            .flex_col()
            .w_full()
            .mb_1()
            .p_1p5()
            .rounded(themes::metrics::RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background);
        for (i, item) in p.items.iter().enumerate() {
            let image = item
                .image
                .clone()
                .filter(|_| cx.has_global::<crate::media::ImageCache>());
            let alt = item.label.clone();
            list = list.child(
                div()
                    .id(("completion", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(themes::metrics::SPACE_9)
                    .px_3()
                    .rounded_sm()
                    .text_size(themes::metrics::FONT_SIZE_SM)
                    .text_color(colors.text)
                    .when(i == p.selected, |d| {
                        d.bg(colors.accent_soft).text_color(colors.text_accent)
                    })
                    .when_some(image, |d, image| {
                        d.child(crate::media::emoji_hook(&image, alt.trim_matches(':')))
                    })
                    .child(item.label.clone())
                    .when_some(item.detail.clone(), |d, t| {
                        d.child(div().text_color(colors.text_muted).child(t))
                    })
                    .when_some(item.members_label.clone(), |d, t| {
                        d.child(div().text_color(colors.text_muted).child(t))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(p) = &mut this.popup {
                            p.selected = i;
                        }
                        this.accept(cx);
                    })),
            );
        }
        list
    }
}

impl<S: ComposerSource + 'static> Composer<S> {
    /// The Send later menu (presets, then Custom...) or its date and time fields.
    fn render_send_later(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let st = self.send_later.as_ref()?;
        let cfg = self.session.read(cx).send_later(cx);
        let rows = self.send_later_rows_with(&cfg, cx)?;
        let local = self.send_later_local_label_with(&cfg, cx);
        let check = self.custom_check_with(&cfg, cx);
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let zone = cfg.zone.iana_name().unwrap_or("UTC").to_string();
        let l = cx.localizer();
        let title = t!(l, "send-later-title", zone = zone).into_owned();
        let mut list = div()
            .debug_selector(|| "send-later-menu".into())
            .flex()
            .flex_col()
            .w_full()
            .mb_1()
            .py(themes::metrics::SPACE_4)
            .gap(themes::metrics::SPACE_2)
            .rounded(themes::metrics::RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .child(
                div()
                    .px(themes::metrics::SPACE_5)
                    .pb(themes::metrics::SPACE_2)
                    .text_sm()
                    .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                    .child(title),
            )
            .when_some(local, |d, label| {
                d.child(
                    div()
                        .px(themes::metrics::SPACE_5)
                        .pb(themes::metrics::SPACE_2)
                        .text_xs()
                        .text_color(colors.text_muted)
                        .child(label),
                )
            });
        if let Some(c) = &st.custom {
            let msg = check.and_then(|c| match c {
                crate::CustomCheck::Past => Some(t!(l, "send-later-past").into_owned()),
                crate::CustomCheck::TooFar => {
                    Some(t!(l, "send-later-too-far", days = domain::MAX_AHEAD_DAYS).into_owned())
                }
                crate::CustomCheck::Invalid => Some(t!(l, "send-later-invalid").into_owned()),
                _ => None,
            });
            let (dp, tp) = (
                t!(l, "send-later-date-placeholder").into_owned(),
                t!(l, "send-later-time-placeholder").into_owned(),
            );
            c.date.update(cx, |i, _| i.set_placeholder(dp));
            c.time.update(cx, |i, _| i.set_placeholder(tp));
            let field = |input: &Entity<TextInput>, sel: &'static str| {
                div()
                    .debug_selector(move || sel.into())
                    .px(themes::metrics::SPACE_4)
                    .py(themes::metrics::SPACE_2)
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.background)
                    .w(themes::metrics::LAYOUT_FIELD_XS)
                    .child(input.clone())
            };
            list = list
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .px(themes::metrics::SPACE_5)
                        .py_1()
                        .child(field(&c.date, "send-later-date"))
                        .child(field(&c.time, "send-later-time")),
                )
                .when_some(msg, |d, m| {
                    d.child(
                        div()
                            .debug_selector(|| "send-later-error".into())
                            .px(themes::metrics::SPACE_5)
                            .text_color(theme.status().error)
                            .text_xs()
                            .child(m),
                    )
                });
            return Some(list.into_any_element());
        }
        for (i, r) in rows.into_iter().enumerate() {
            let disabled = r.disabled.clone();
            list = list.child(
                div()
                    .id(("send-later", i))
                    .flex()
                    .gap_2()
                    .mx(themes::metrics::SPACE_3)
                    .px(themes::metrics::SPACE_4)
                    .py(themes::metrics::SPACE_2)
                    .rounded(themes::metrics::RADIUS_SM)
                    .text_sm()
                    .text_color(if disabled.is_some() {
                        colors.text_muted
                    } else {
                        colors.text
                    })
                    .when(i == st.selected, |d| d.bg(colors.accent_soft))
                    .child(r.label)
                    .when_some(disabled, |d, why| {
                        d.child(div().text_xs().text_color(colors.text_muted).child(why))
                    })
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if let Some(s) = &mut this.send_later {
                            s.selected = i;
                        }
                        this.choose_send_later(i, w, cx);
                    })),
            );
        }
        Some(list.into_any_element())
    }

    fn render_slash_menu(
        &self,
        m: &SlashState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<S> {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let typed = slash_menu::query(self.input.read(cx).text()).unwrap_or("");
        let header = if typed.is_empty() {
            t!(cx, "slash-header-all").into_owned()
        } else {
            t!(cx, "slash-header-matching", query = typed).into_owned()
        };
        let key = |sel: &'static str, k: &str, what: String| {
            div()
                .debug_selector(move || sel.into())
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .px_1()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.border)
                        .child(
                            crate::keycap::Glyphs::new(k.to_string(), cx)
                                .row(themes::metrics::ICON_SIZE_SM, colors.text),
                        ),
                )
                .child(what)
        };
        let mut list = div()
            .debug_selector(|| "slash-menu".into())
            .flex()
            .flex_col()
            .w_full()
            .mb_1()
            .p_1p5()
            .rounded(themes::metrics::RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .child(
                div()
                    .debug_selector(|| "slash-header".into())
                    .px_3()
                    .pt_1()
                    .pb_1()
                    .text_size(themes::metrics::FONT_SIZE_XS)
                    .font_weight(themes::metrics::FONT_WEIGHT_MEDIUM)
                    .text_color(tokens.text_subtle)
                    .child(header.to_uppercase()),
            );
        for (i, e) in m.items.iter().enumerate() {
            let selected = i == m.selected;
            list = list.child(
                div()
                    .id(("slash", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(themes::metrics::SPACE_9)
                    .px_3()
                    .rounded_sm()
                    .text_size(themes::metrics::FONT_SIZE_SM)
                    .text_color(colors.text)
                    .when(selected, |d| d.bg(colors.accent_soft))
                    .child(
                        div()
                            .debug_selector(move || format!("slash-name-{i}"))
                            .flex_none()
                            .min_w(themes::metrics::LAYOUT_COMMAND_NAME)
                            .font_family(self.code_font.clone())
                            .font_weight(themes::metrics::FONT_WEIGHT_MEDIUM)
                            .text_color(if selected {
                                tokens.accent_soft_text
                            } else {
                                colors.text
                            })
                            .child(format!("/{}", e.name)),
                    )
                    .child(
                        div()
                            .id(("slash-desc", i))
                            .debug_selector(move || format!("slash-desc-{i}"))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(colors.text_muted)
                            .child(slash_menu::description(cx.localizer(), e.name)),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("slash-source-{i}"))
                            .flex_none()
                            .px_2()
                            .py_0p5()
                            .rounded_full()
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.background)
                            .text_size(themes::metrics::FONT_SIZE_XS)
                            .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                            .text_color(colors.text_muted)
                            .child(e.source.label(cx.localizer())),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(m) = &mut this.slash {
                            m.selected = i;
                        }
                        this.accept_slash(cx);
                    })),
            );
        }
        list.child(
            div()
                .debug_selector(|| "slash-footer".into())
                .flex()
                .flex_wrap()
                .gap_3()
                .mt_1()
                .px_3()
                .pt_2()
                .pb_1()
                .border_t_1()
                .border_color(tokens.border_subtle)
                .text_size(themes::metrics::FONT_SIZE_XS)
                .text_color(tokens.text_subtle)
                .child(key(
                    "slash-hint-move",
                    crate::keycap::UP_DOWN,
                    t!(cx, "slash-hint-move").into_owned(),
                ))
                .child(key(
                    "slash-hint-complete",
                    crate::keycap::TAB,
                    t!(cx, "slash-hint-complete").into_owned(),
                ))
                .child(key(
                    "slash-hint-run",
                    crate::keycap::RETURN,
                    t!(cx, "slash-hint-run").into_owned(),
                ))
                .child(key(
                    "slash-hint-close",
                    "esc",
                    t!(cx, "slash-hint-close").into_owned(),
                )),
        )
    }

    /// The composer of an expired session: a dashed, read-only strip.
    fn render_read_only(&self, cx: &mut Context<Self>) -> impl IntoElement + use<S> {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        div()
            .debug_selector(|| "composer-read-only".into())
            .w_full()
            .px_3()
            .py_2()
            .bg(colors.editor_background)
            .child(
                div()
                    .id("composer-read-only")
                    .w_full()
                    .min_h(themes::metrics::LAYOUT_COMPOSER_MIN)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(themes::metrics::RADIUS_LG)
                    .border_1()
                    .border_dashed()
                    .border_color(colors.border_strong)
                    .text_size(themes::metrics::FONT_SIZE_SM)
                    .text_color(tokens.text_subtle)
                    .child(t!(cx, "composer-read-only").into_owned()),
            )
    }

    /// The composer of a channel the user is not in: a line saying so and
    /// Join, which sending needs first. The draft waits in the input.
    fn render_join(&self, cx: &mut Context<Self>) -> impl IntoElement + use<S> {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let joining = self.session.read(cx).joining(&self.conversation);
        let label = if joining {
            self.join_text.joining.clone()
        } else {
            self.join_text.join.clone()
        };
        let button = crate::controls::button(
            "composer-join-button",
            label,
            crate::controls::ButtonKind::Primary,
            !joining,
            &colors,
        )
        .track_focus(&self.join_focus)
        .debug_selector(|| "composer-join-button".into());
        let join = crate::controls::on_activate(button, cx, |this: &mut Self, _, cx| {
            let s = this.session.read(cx);
            if s.joining(&this.conversation) || !s.must_join(&this.conversation) {
                return;
            }
            let c = this.conversation.clone();
            this.session.update(cx, |s, cx| s.join_conversation(c, cx));
        });
        div()
            .debug_selector(|| "composer-join".into())
            .w_full()
            .px_3()
            .py_2()
            .bg(colors.editor_background)
            .child(
                div()
                    .id("composer-join")
                    .w_full()
                    .min_h(themes::metrics::LAYOUT_COMPOSER_MIN)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .gap(themes::metrics::SPACE_5)
                    .px(themes::metrics::SPACE_5)
                    .rounded(themes::metrics::RADIUS_LG)
                    .border_1()
                    .border_color(colors.border)
                    .child(
                        div()
                            .text_size(themes::metrics::FONT_SIZE_SM)
                            .text_color(tokens.text_subtle)
                            .child(self.join_text.prompt.clone()),
                    )
                    .child(join),
            )
    }

    /// The panel shown for a long paste: title, language and the choice between a snippet
    /// and plain text.
    fn render_snippet_offer(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let o = self.snippet_offer.as_ref()?;
        let theme = cx.theme();
        let colors = *theme.colors();
        let tokens = themes::parla_tokens(theme);
        let l = i18n::gpui::localizer(cx);
        o.title.update(cx, |i, _| {
            // The empty title is a real choice: the file is `snippet.<ext>`.
            i.set_placeholder(crate::snippet::file_name("", o.language))
        });
        let filter_hint = t!(l, "snippet-offer-filter").into_owned();
        o.filter.update(cx, |i, _| i.set_placeholder(filter_hint));
        let title_focused = o.title.read(cx).focus_handle(cx).is_focused(window);
        let label = |text: String| {
            div()
                .text_size(metrics::FONT_SIZE_2XS)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(colors.text_muted)
                .child(text)
        };
        let lang_button = crate::controls::on_activate(
            div()
                .id("composer-snippet-language")
                .debug_selector(|| "composer-snippet-language".into())
                .flex()
                .items_center()
                .gap(metrics::SPACE_4)
                .h(metrics::LAYOUT_INPUT_MIN)
                .px(metrics::SPACE_5)
                .rounded(metrics::RADIUS_MD)
                .border_1()
                .border_color(tokens.border_control)
                .bg(colors.surface_background)
                .cursor_pointer()
                .tab_stop(true)
                .focusable()
                .focus_visible(crate::controls::focus_style(&colors))
                .when(o.menu_open, |d| {
                    d.border_color(colors.border_focused)
                        .shadow(crate::controls::ring(&colors))
                })
                .child(
                    div()
                        .px(metrics::SPACE_3)
                        .rounded(metrics::RADIUS_SM)
                        .bg(colors.accent_soft)
                        .text_color(tokens.accent_soft_text)
                        .text_size(metrics::FONT_SIZE_2XS)
                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        .child(o.language.display(&l)),
                )
                .when(o.auto, |d| {
                    d.child(
                        div()
                            .text_size(metrics::FONT_SIZE_2XS)
                            .text_color(tokens.text_subtle)
                            .child(t!(l, "snippet-offer-auto").into_owned()),
                    )
                })
                .child(div().flex_1())
                .child(
                    crate::icon::Icon::new(if o.menu_open {
                        crate::icon::IconName::ChevUp
                    } else {
                        crate::icon::IconName::ChevDown
                    })
                    .size(metrics::ICON_SIZE_SM)
                    .color(tokens.text_subtle),
                ),
            cx,
            |this, window, cx| this.toggle_language_menu(window, cx),
        );
        // The language list: a popover under the button with a filter and the matches.
        let menu = o.menu_open.then(|| {
            let matches = Self::offer_languages(o, cx);
            let selected = o.selected.min(matches.len().saturating_sub(1));
            let filter_row = div()
                .debug_selector(|| "composer-snippet-filter".into())
                .flex()
                .items_center()
                .gap(metrics::SPACE_4)
                .h(metrics::LAYOUT_MENU_ROW)
                .px(metrics::SPACE_4)
                .rounded(metrics::RADIUS_MD)
                .bg(tokens.bg_sunken)
                .text_size(metrics::FONT_SIZE_XS)
                .child(
                    crate::icon::Icon::new(crate::icon::IconName::Search)
                        .size(metrics::ICON_SIZE_SM)
                        .color(tokens.text_subtle),
                )
                .child(div().flex_1().min_w_0().child(o.filter.clone()));
            let rows = matches.iter().enumerate().map(|(ix, lang)| {
                let current = lang.id == o.language.id;
                let on = ix == selected;
                let (hover, id) = (colors.element_hover, lang.id);
                crate::controls::on_activate(
                    div()
                        .id(("composer-snippet-lang", ix))
                        .debug_selector(move || format!("composer-snippet-lang-{id}"))
                        .flex()
                        .items_center()
                        .gap(metrics::SPACE_4)
                        .px(metrics::SPACE_4)
                        .h(metrics::LAYOUT_MENU_ROW)
                        .rounded(metrics::RADIUS_MD)
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(if on {
                            tokens.accent_soft_text
                        } else {
                            colors.text
                        })
                        .when(on, |d| d.bg(colors.accent_soft))
                        .when(current, |d| d.font_weight(metrics::FONT_WEIGHT_SEMIBOLD))
                        .cursor_pointer()
                        .hover(move |d| d.bg(hover))
                        .child(div().flex_1().child(lang.display(&l)))
                        .when(current, |d| {
                            d.child(
                                crate::icon::Icon::new(crate::icon::IconName::Check)
                                    .size(metrics::ICON_SIZE_SM)
                                    .color(tokens.accent_soft_text),
                            )
                        }),
                    cx,
                    move |this, window, cx| {
                        this.set_snippet_language(id, cx);
                        if let Some(o) = &this.snippet_offer {
                            window.focus(&o.title.read(cx).focus_handle(cx), cx);
                        }
                    },
                )
            });
            div()
                .debug_selector(|| "composer-snippet-languages".into())
                .absolute()
                .top_full()
                .left_0()
                .right_0()
                .mt(metrics::SPACE_2)
                .flex()
                .flex_col()
                .gap(metrics::SPACE_1)
                .p(metrics::SPACE_3)
                .rounded(metrics::RADIUS_LG)
                .border_1()
                .border_color(colors.border)
                .bg(colors.elevated_surface_background)
                .shadow_md()
                .child(filter_row)
                .children(rows)
                .when(matches.is_empty(), |d| {
                    d.child(
                        div()
                            .px(metrics::SPACE_4)
                            .h(metrics::LAYOUT_MENU_ROW)
                            .flex()
                            .items_center()
                            .text_size(metrics::FONT_SIZE_XS)
                            .text_color(tokens.text_subtle)
                            .child(t!(l, "snippet-offer-no-language").into_owned()),
                    )
                })
        });
        let preview =
            crate::snippet::render_preview(&o.text, o.language, cx.theme(), self.code_font.clone())
                .debug_selector(|| "composer-snippet-preview".into());
        Some(
            div()
                .id("composer-snippet-offer")
                .track_focus(&o.focus)
                .debug_selector(|| "composer-snippet-offer".into())
                .flex()
                .flex_col()
                .gap(metrics::SPACE_4)
                .mx(metrics::SPACE_4)
                .mt(metrics::SPACE_4)
                .p(metrics::SPACE_5)
                .rounded(metrics::RADIUS_MD)
                .border_1()
                .border_color(colors.border)
                .bg(colors.surface_background)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(metrics::SPACE_4)
                        .child(
                            crate::icon::Icon::new(crate::icon::IconName::Code)
                                .size(metrics::ICON_SIZE_SM)
                                .color(colors.text_muted),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(metrics::FONT_SIZE_SM)
                                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                .text_color(colors.text)
                                .child(
                                    t!(l, "snippet-offer-heading", count = o.lines as i64)
                                        .into_owned(),
                                ),
                        )
                        .child(
                            crate::controls::icon_button(
                                "composer-snippet-dismiss",
                                crate::icon::IconName::X,
                                t!(l, "snippet-offer-dismiss").into_owned(),
                                &colors,
                                {
                                    let this = cx.entity();
                                    move |_, cx| {
                                        this.update(cx, |c, cx| c.dismiss_snippet_offer(cx))
                                    }
                                },
                            )
                            .debug_selector(|| "composer-snippet-dismiss".into()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(metrics::SPACE_5)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(metrics::SPACE_2)
                                .flex_1()
                                .child(label(t!(l, "snippet-offer-title").into_owned()))
                                .child(
                                    div()
                                        .debug_selector(|| "composer-snippet-title".into())
                                        .h(metrics::LAYOUT_INPUT_MIN)
                                        .px(metrics::SPACE_5)
                                        .flex()
                                        .items_center()
                                        .rounded(metrics::RADIUS_MD)
                                        .border_1()
                                        .border_color(tokens.border_control)
                                        // The focus ring, as on every field.
                                        .when(title_focused, |d| {
                                            d.border_color(colors.border_focused)
                                                .shadow(crate::controls::ring(&colors))
                                        })
                                        .child(o.title.clone()),
                                ),
                        )
                        .child(
                            div()
                                .relative()
                                .flex()
                                .flex_col()
                                .gap(metrics::SPACE_2)
                                .flex_1()
                                .child(label(t!(l, "snippet-offer-language").into_owned()))
                                .child(lang_button)
                                .children(menu),
                        ),
                )
                .child(preview)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(metrics::SPACE_4)
                        .child(crate::controls::on_activate(
                            crate::controls::button(
                                "composer-snippet-paste",
                                t!(l, "snippet-offer-paste").into_owned(),
                                crate::controls::ButtonKind::Secondary,
                                true,
                                &colors,
                            )
                            .debug_selector(|| "composer-snippet-paste".into()),
                            cx,
                            |this, _, cx| this.paste_snippet_as_text(cx),
                        ))
                        .child(crate::controls::on_activate(
                            crate::controls::button(
                                "composer-snippet-share",
                                t!(l, "snippet-offer-share").into_owned(),
                                crate::controls::ButtonKind::Primary,
                                true,
                                &colors,
                            )
                            .debug_selector(|| "composer-snippet-share".into()),
                            cx,
                            |this, _, cx| this.create_snippet(cx),
                        )),
                )
                .into_any_element(),
        )
    }

    /// One chip per staged file: type badge or thumbnail, name, status, and its actions.
    fn render_staged(
        &self,
        files: &[StagedFile],
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if files.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let colors = *theme.colors();
        let error = theme.status().error;
        let ok = theme.status().success;
        let tokens = themes::parla_tokens(theme);
        let l = i18n::gpui::localizer(cx);
        let chips = files.iter().map(|f| {
            let id = f.id;
            let failed = matches!(f.state, StagedState::Failed(_) | StagedState::Cancelled);
            let size = crate::files::format_size(f.size, &l);
            let name = f.name.to_string();
            let ext = std::path::Path::new(&name)
                .extension()
                .map(|e| e.to_string_lossy().to_uppercase())
                .filter(|e| e.chars().count() <= 4);
            // Images take the signal tint, every other kind the accent tint; a failed chip
            // sits on the surface in the danger colour.
            let is_image = ext.as_deref().is_some_and(|e| {
                matches!(
                    e,
                    "PNG" | "JPG" | "JPEG" | "GIF" | "WEBP" | "SVG" | "BMP" | "HEIC"
                )
            });
            let (badge_bg, badge_fg) = if failed {
                (colors.surface_background, error)
            } else if is_image {
                (tokens.signal_soft, tokens.signal_text)
            } else {
                (colors.accent_soft, tokens.accent_soft_text)
            };
            let ext = match &f.snippet {
                // A snippet's badge says what it is, whatever the file name ends in.
                Some(_) => Some("</>".to_string()),
                None => ext,
            };
            let lead = match &f.thumb {
                Some(path) => gpui::img(path.clone())
                    .size(metrics::SPACE_9)
                    .flex_none()
                    .rounded(metrics::RADIUS_SM)
                    .object_fit(gpui::ObjectFit::Cover)
                    .into_any_element(),
                None => div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(metrics::SPACE_9)
                    .rounded(metrics::RADIUS_SM)
                    .bg(badge_bg)
                    .text_color(badge_fg)
                    .text_size(metrics::FONT_SIZE_3XS)
                    .font_weight(metrics::FONT_WEIGHT_BOLD)
                    .child(match ext {
                        Some(e) => div().child(e).into_any_element(),
                        None => crate::icon::Icon::new(crate::icon::IconName::File)
                            .size(metrics::ICON_SIZE_SM)
                            .color(badge_fg)
                            .into_any_element(),
                    })
                    .into_any_element(),
            };
            let (status, status_color) = match &f.state {
                StagedState::Ready if f.snippet.is_some() => {
                    let s = f.snippet.as_ref().expect("checked");
                    (
                        t!(
                            l,
                            "composer-staged-snippet",
                            // The id names the language in the UI language ("Plain text" translated).
                            language = crate::snippet::language_by_id(&s.language)
                                .map_or_else(|| s.language.to_string(), |lang| lang.display(&l)),
                            lines = t!(l, "snippet-lines", count = s.lines as i64).into_owned()
                        )
                        .into_owned(),
                        ok,
                    )
                }
                StagedState::Ready => (
                    t!(l, "composer-staged-ready", size = size.as_str()).into_owned(),
                    ok,
                ),
                StagedState::Uploading(None) => (
                    t!(l, "composer-staged-uploading", size = size.as_str()).into_owned(),
                    tokens.text_subtle,
                ),
                StagedState::Uploading(Some(p)) => (
                    t!(
                        l,
                        "composer-staged-uploading-pct",
                        size = size.as_str(),
                        percent = (p.clamp(0.0, 1.0) * 100.0).round() as i64
                    )
                    .into_owned(),
                    tokens.text_subtle,
                ),
                StagedState::Failed(why) => (
                    t!(l, "composer-staged-failed", reason = why.as_ref()).into_owned(),
                    error,
                ),
                StagedState::Cancelled => (t!(l, "composer-staged-cancelled").into_owned(), error),
            };
            let bar = match f.state {
                StagedState::Uploading(Some(p)) => Some(p.clamp(0.0, 1.0)),
                _ => None,
            };
            let uploading = matches!(f.state, StagedState::Uploading(_));
            let this = cx.entity();
            let remove = {
                let this = this.clone();
                let label = if uploading {
                    t!(l, "composer-staged-cancel", name = name.as_str()).into_owned()
                } else {
                    t!(l, "composer-staged-remove", name = name.as_str()).into_owned()
                };
                crate::controls::icon_button_sized(
                    ("composer-staged-remove", id),
                    crate::icon::IconName::X,
                    label,
                    crate::controls::IconButtonSize::Small,
                    &colors,
                    move |_, cx| {
                        this.update(cx, |c, cx| {
                            if uploading {
                                c.cancel_staged(id, cx)
                            } else {
                                c.remove_staged(id, cx)
                            }
                        })
                    },
                )
                .debug_selector(move || format!("composer-staged-remove-{id}"))
            };
            let status_line = div()
                .flex()
                .items_center()
                .gap(metrics::SPACE_2)
                .text_size(metrics::FONT_SIZE_2XS)
                .text_color(status_color)
                .when(matches!(f.state, StagedState::Ready), |d| {
                    d.font_weight(metrics::FONT_WEIGHT_SEMIBOLD).child(
                        crate::icon::Icon::new(crate::icon::IconName::CheckCircle)
                            .size(metrics::ICON_SIZE_SM)
                            .color(status_color),
                    )
                })
                .child(div().min_w_0().child(status));
            div()
                .id(("composer-staged", id))
                .debug_selector(move || format!("composer-staged-{id}"))
                .flex()
                .flex_none()
                .items_center()
                .gap(metrics::SPACE_4)
                // A failed chip is wider so the reason reads in full.
                .w(metrics::SPACE_12 * if failed { 5.0 } else { 4.0 })
                .p(metrics::SPACE_4)
                .rounded(metrics::RADIUS_MD)
                .border_1()
                .border_color(if failed { error } else { colors.border })
                .bg(if failed {
                    tokens.danger_soft
                } else {
                    colors.surface_background
                })
                .child(lead)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(metrics::SPACE_1)
                        .child(
                            div()
                                .truncate()
                                .text_size(metrics::FONT_SIZE_XS)
                                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                .text_color(colors.text)
                                .child(name.clone()),
                        )
                        .child(status_line)
                        .when_some(bar, |d, p| {
                            d.child(
                                div()
                                    .id(("composer-staged-bar", id))
                                    .role(gpui::Role::ProgressIndicator)
                                    .aria_label(
                                        t!(l, "composer-staged-bar", name = name.as_str())
                                            .into_owned(),
                                    )
                                    .aria_min_numeric_value(0.0)
                                    .aria_max_numeric_value(100.0)
                                    .aria_numeric_value((p * 100.0).round() as f64)
                                    .w_full()
                                    .h(metrics::SPACE_2)
                                    .rounded(metrics::RADIUS_SM)
                                    .bg(tokens.border_subtle)
                                    .child(
                                        div()
                                            .h_full()
                                            .w(gpui::relative(p))
                                            .rounded(metrics::RADIUS_SM)
                                            .bg(tokens.signal),
                                    ),
                            )
                        }),
                )
                .when(failed, |d| {
                    let (hover, text) = (colors.element_hover, colors.text);
                    d.child(crate::controls::on_activate(
                        div()
                            .id(("composer-staged-retry", id))
                            .debug_selector(move || format!("composer-staged-retry-{id}"))
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(metrics::SPACE_2)
                            .h(metrics::LAYOUT_MENU_ROW)
                            .px(metrics::SPACE_4)
                            .rounded(metrics::RADIUS_MD)
                            .border_1()
                            .border_color(colors.border)
                            .bg(colors.surface_background)
                            .text_color(text)
                            .text_size(metrics::FONT_SIZE_XS)
                            .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                            .cursor_pointer()
                            .tab_stop(true)
                            .focusable()
                            .focus_visible(crate::controls::focus_style(&colors))
                            .hover(move |d| d.bg(hover))
                            .child(
                                crate::icon::Icon::new(crate::icon::IconName::Refresh)
                                    .size(metrics::ICON_SIZE_SM)
                                    .color(text),
                            )
                            .child(t!(l, "composer-staged-retry").into_owned()),
                        cx,
                        move |this, _, cx| this.retry_staged(id, cx),
                    ))
                })
                .child(remove)
        });
        Some(
            div()
                .debug_selector(|| "composer-staged".into())
                .flex()
                .flex_wrap()
                .gap(metrics::SPACE_4)
                .px(metrics::SPACE_4)
                .pt(metrics::SPACE_4)
                .children(chips)
                .into_any_element(),
        )
    }

    /// The line left of Send: what Send does now with the staged files.
    fn staged_hint(&self, files: &[StagedFile], cx: &App) -> Option<String> {
        let count = |f: fn(&StagedState) -> bool| files.iter().filter(|x| f(&x.state)).count();
        let uploading = count(|s| matches!(s, StagedState::Uploading(_)));
        let failed = count(|s| matches!(s, StagedState::Failed(_) | StagedState::Cancelled));
        let ready = count(|s| *s == StagedState::Ready);
        let l = i18n::gpui::localizer(cx);
        if uploading > 0 {
            Some(
                t!(
                    l,
                    "composer-staged-hint-uploading",
                    count = uploading as i64
                )
                .into_owned(),
            )
        } else if failed > 0 {
            Some(t!(l, "composer-staged-hint-failed", count = failed as i64).into_owned())
        } else if ready > 0 {
            Some(t!(l, "composer-staged-hint-ready", count = ready as i64).into_owned())
        } else {
            None
        }
    }

    /// The row of formatting buttons above the input: icons of one size, so the row is
    /// narrow enough for the thread panel and wraps rather than clipping.
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<S> {
        use crate::icon::IconName;
        let colors = *cx.theme().colors();
        let hover = colors.element_selected;
        let text = colors.text;
        let muted = colors.text_muted;
        // Tooltips are built when they open: a redraw formats no label.
        let button = |id: &'static str, glyph: gpui::AnyElement, tip: fn(&App) -> String| {
            div()
                .id(id)
                .debug_selector(|| id.into())
                .flex()
                .items_center()
                .justify_center()
                .w(themes::metrics::LAYOUT_ICON_BUTTON_SM)
                .h(themes::metrics::LAYOUT_ROW_COMPACT)
                .flex_none()
                .rounded_sm()
                .cursor_pointer()
                .text_size(themes::metrics::FONT_SIZE_SM)
                .text_color(muted)
                .hover(move |d| d.bg(hover).text_color(text))
                .tooltip(crate::reactions::tooltip_lazy(tip))
                .child(glyph)
        };
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_0p5()
            .px_2()
            .pt_1p5();
        for (f, id, icon, tip) in [
            (
                Format::Bold,
                "fmt-bold",
                Some(IconName::Bold),
                (|cx: &App| t!(cx, "format-bold").into_owned()) as fn(&App) -> String,
            ),
            (Format::Italic, "fmt-italic", Some(IconName::Italic), |cx| {
                t!(cx, "format-italic").into_owned()
            }),
            (Format::Strike, "fmt-strike", Some(IconName::Strike), |cx| {
                t!(cx, "format-strike").into_owned()
            }),
            (Format::Code, "fmt-code", Some(IconName::Code), |cx| {
                t!(cx, "format-code").into_owned()
            }),
            // The icon set has no code block mark: braces at the icons' visual weight.
            (Format::CodeBlock, "fmt-code-block", None, |cx| {
                t!(cx, "format-code-block").into_owned()
            }),
            (Format::Quote, "fmt-quote", Some(IconName::Quote), |cx| {
                t!(cx, "format-quote").into_owned()
            }),
            (
                Format::BulletList,
                "fmt-bullets",
                Some(IconName::List),
                |cx| t!(cx, "format-bullet-list").into_owned(),
            ),
            (
                Format::NumberedList,
                "fmt-numbered",
                Some(IconName::ListOrdered),
                |cx| t!(cx, "format-numbered-list").into_owned(),
            ),
        ] {
            let glyph = match icon {
                Some(n) => icon_glyph(n),
                None => div()
                    .font_weight(themes::metrics::FONT_WEIGHT_MEDIUM)
                    .child("{}")
                    .into_any_element(),
            };
            row = row.child(
                button(id, glyph, tip)
                    .on_click(cx.listener(move |this, _, w, cx| this.format_action(f, w, cx))),
            );
        }
        row.child(
            button("fmt-link", icon_glyph(IconName::Link), |cx| {
                t!(cx, "format-link").into_owned()
            })
            .on_click(cx.listener(|this, _, w, cx| this.begin_link(w, cx))),
        )
    }
}

/// Horizontal inset of the draft text inside the composer frame; the "is
/// typing" line lines up with it.
const INPUT_INSET_X: gpui::Pixels = metrics::SPACE_5;

impl<S: ComposerSource + 'static> Render for Composer<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.session.read(cx).read_only() {
            return self.render_read_only(cx).into_any_element();
        }
        if self.session.read(cx).must_join(&self.conversation) {
            self.join_shown = true;
            return self.render_join(cx).into_any_element();
        }
        // The join landed while Join had the focus: the draft is waiting.
        if std::mem::take(&mut self.join_shown) && self.join_focus.is_focused(window) {
            self.refocus = true;
        }
        let editing = self.editing.is_some();
        self.input.update(cx, |i, cx| {
            let placeholder = if editing {
                t!(cx, "composer-edit-placeholder")
            } else {
                t!(cx, "composer-placeholder")
            };
            i.set_placeholder_str(&placeholder);
        });
        if std::mem::take(&mut self.refocus) {
            let h = self.input.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| window.focus(&h, cx));
        }
        if let Some(l) = &self.link {
            l.input.update(cx, |i, cx| {
                i.set_placeholder_str(&t!(cx, "composer-link-placeholder"))
            });
        }
        let popup = self.popup.as_ref().map(|p| self.render_popup(p, cx));
        let typing = self.render_typing(cx);
        let slash_menu = self.slash.as_ref().map(|m| self.render_slash_menu(m, cx));
        use crate::icon::IconName;
        let toolbar = self.render_toolbar(cx);
        let emoji_picker = self.emoji_picker.clone();
        let send_later_menu = self.render_send_later(cx);
        let slash_notice = self.slash_notice(cx.localizer());
        let slash_open = self.slash_open_label(cx);
        let link_prompt = self.link.as_ref().map(|l| (l.input.clone(), l.invalid));
        self.take_restored_text(cx);
        let staged = self.staged_files(cx);
        let staged_row = self.render_staged(&staged, cx);
        let snippet_offer = self.render_snippet_offer(window, cx);
        let staged_hint = self.staged_hint(&staged, cx);
        let waiting = self.editing.is_none()
            && staged
                .iter()
                .any(|f| matches!(f.state, StagedState::Uploading(_)));
        let send_enabled = (!waiting
            && (!self.input.read(cx).text().trim().is_empty()
                || (self.editing.is_none()
                    && staged.iter().any(|f| f.state == StagedState::Ready))))
            || self.send_later.is_some()
            || self.link.is_some()
            || self.popup.is_some()
            || self.slash.is_some();
        let can_send_later = self.reply_root.is_none() && self.editing.is_none();
        let theme = cx.theme();
        let colors_v = *theme.colors();
        let error = theme.status().error;
        let tokens = themes::parla_tokens(theme);
        let subtle = tokens.text_subtle;
        let colors = &colors_v;
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let border = if focused {
            gpui::transparent_black()
        } else {
            colors.border_strong
        };
        div()
            .key_context(self.key_context.clone())
            .on_action(cx.listener(Self::send))
            .on_action(cx.listener(Self::plain_enter))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::edit_last))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::toggle_broadcast))
            .on_action(cx.listener(Self::paste))
            // The Edit menu sends the text input's Paste; capture it here so
            // images and files are attached like the composer's own paste key.
            .capture_action(cx.listener(|this, _: &crate::text_input::Paste, w, cx| {
                this.paste(&Paste, w, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(Self::accept_completion))
            .on_action(cx.listener(Self::picker_down))
            .on_action(cx.listener(|this, _: &Bold, w, cx| this.format_action(Format::Bold, w, cx)))
            .on_action(
                cx.listener(|this, _: &Italic, w, cx| this.format_action(Format::Italic, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Strike, w, cx| this.format_action(Format::Strike, w, cx)),
            )
            .on_action(cx.listener(|this, _: &Code, w, cx| this.format_action(Format::Code, w, cx)))
            .on_action(cx.listener(|this, _: &CodeBlock, w, cx| {
                this.format_action(Format::CodeBlock, w, cx)
            }))
            .on_action(
                cx.listener(|this, _: &Quote, w, cx| this.format_action(Format::Quote, w, cx)),
            )
            .on_action(cx.listener(|this, _: &BulletList, w, cx| {
                this.format_action(Format::BulletList, w, cx)
            }))
            .on_action(cx.listener(|this, _: &NumberedList, w, cx| {
                this.format_action(Format::NumberedList, w, cx)
            }))
            .on_action(cx.listener(Self::link_action))
            .on_action(cx.listener(Self::send_later_action))
            .relative()
            .w_full()
            .px_3()
            .py_2()
            .bg(colors.editor_background)
            .children(popup)
            .children(slash_menu)
            .children(send_later_menu)
            .children(typing)
            .child(
                div()
                    .debug_selector(|| "composer-frame".into())
                    .w_full()
                    .min_h(themes::metrics::LAYOUT_COMPOSER_MIN)
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(themes::metrics::RADIUS_LG)
                    .border_1()
                    .border_color(border)
                    .bg(colors.surface_background)
                    .when(focused, |d| d.shadow(crate::controls::ring(colors)))
                    .child(toolbar)
                    .when_some(link_prompt, |d, (input, invalid)| {
                        d.child(
                            div()
                                .debug_selector(|| "composer-link".into())
                                .mx_2()
                                .mt_1()
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(colors.border_focused)
                                .child(input)
                                .when(invalid, |d| {
                                    d.child(
                                        div()
                                            .text_xs()
                                            .text_color(error)
                                            .child(t!(cx, "composer-link-invalid").into_owned()),
                                    )
                                }),
                        )
                    })
                    .children(snippet_offer)
                    .children(staged_row)
                    .child(
                        div()
                            .debug_selector(|| "composer-input".into())
                            .w_full()
                            .min_h(themes::metrics::LAYOUT_INPUT_MIN)
                            .px(INPUT_INSET_X)
                            .py_2()
                            .child(self.input.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_end()
                            .gap(themes::metrics::SPACE_1)
                            .px_2()
                            .pt_1()
                            .pb_2()
                            .text_xs()
                            .text_color(colors.text_muted)
                            .child(
                                action_button(
                                    "composer-attach",
                                    IconName::Paperclip,
                                    |cx| t!(cx, "composer-attach").into_owned(),
                                    colors,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.attach(cx))),
                            )
                            .child(
                                div()
                                    .relative()
                                    .child(
                                        action_button(
                                            "composer-emoji",
                                            IconName::Smile,
                                            |cx| t!(cx, "composer-emoji").into_owned(),
                                            colors,
                                        )
                                        // On press, not click: an outside press has just closed
                                        // the open picker, and this one must not reopen it.
                                        .on_mouse_down(
                                            gpui::MouseButton::Left,
                                            cx.listener(|this, _, w, cx| {
                                                if !std::mem::take(&mut this.picker_just_closed) {
                                                    this.toggle_emoji_picker(w, cx);
                                                }
                                            }),
                                        ),
                                    )
                                    .when_some(emoji_picker, |d, p| {
                                        d.child(gpui::deferred(
                                            gpui::anchored()
                                                .anchor(gpui::Anchor::BottomLeft)
                                                // Above the button: the anchor is below it.
                                                .offset(gpui::point(
                                                    gpui::px(0.),
                                                    -themes::metrics::LAYOUT_ROW_COMPACT,
                                                ))
                                                .snap_to_window_with_margin(
                                                    themes::metrics::SPACE_2,
                                                )
                                                .child(
                                                    div()
                                                        .debug_selector(|| {
                                                            "composer-emoji-picker".into()
                                                        })
                                                        .pb_1()
                                                        .on_mouse_down_out(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.close_emoji_picker_by_click(
                                                                    window, cx,
                                                                )
                                                            },
                                                        ))
                                                        .child(p),
                                                ),
                                        ))
                                    }),
                            )
                            .when(can_send_later, |d| {
                                d.child(
                                    action_button(
                                        "composer-send-later",
                                        IconName::Clock,
                                        |cx| t!(cx, "send-later-button").into_owned(),
                                        colors,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, w, cx| this.send_later_action(&SendLater, w, cx),
                                    )),
                                )
                            })
                            .child(
                                div()
                                    .debug_selector(|| "composer-staged-hint".into())
                                    .flex_1()
                                    .min_w_0()
                                    .px_2()
                                    .text_color(subtle)
                                    // One line always: a long hint ends in an ellipsis.
                                    .truncate()
                                    .when(staged_hint.is_none(), |d| {
                                        d.flex()
                                            .flex_wrap()
                                            .items_center()
                                            .h(themes::metrics::LAYOUT_ROW_COMPACT)
                                    })
                                    .when_some(staged_hint.clone(), |d, h| {
                                        d.child(SharedString::from(h))
                                    })
                                    // The whole hint or none of it: a one-line box in which
                                    // a hint that does not fit wraps below the spacer, out
                                    // of view, instead of clipping mid-word. The auto margin
                                    // right-aligns it.
                                    .when(staged_hint.is_none(), |d| {
                                        d.child(
                                            div()
                                                .w(themes::metrics::SPACE_1)
                                                .h(themes::metrics::LAYOUT_ROW_COMPACT),
                                        )
                                        .child(
                                            self.footer_glyphs()
                                                .row(themes::metrics::ICON_SIZE_SM, subtle)
                                                .flex_none()
                                                .ml_auto(),
                                        )
                                    }),
                            )
                            .child(crate::controls::on_activate(
                                crate::controls::button(
                                    "composer-send",
                                    t!(cx, "composer-send").into_owned(),
                                    crate::controls::ButtonKind::Primary,
                                    send_enabled,
                                    colors,
                                )
                                .debug_selector(|| "composer-send".into())
                                .flex_none()
                                .min_h(themes::metrics::LAYOUT_ROW_COMPACT)
                                .py_0()
                                .px_3()
                                .text_size(themes::metrics::FONT_SIZE_XS),
                                cx,
                                |this, w, cx| {
                                    this.send(&Send, w, cx);
                                    this.refocus = true;
                                    cx.notify();
                                },
                            )),
                    ),
            )
            .when_some(
                self.notice.as_ref().map(|n| n.text(cx.localizer())),
                |d, n| {
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(metrics::SPACE_4)
                            .text_xs()
                            .child(div().text_color(error).child(n))
                            // A paste too large for a snippet can still go as a file.
                            .when(self.oversized.is_some(), |d| {
                                d.child(crate::controls::on_activate(
                                    div()
                                        .id("composer-paste-as-file")
                                        .debug_selector(|| "composer-paste-as-file".into())
                                        .px(metrics::SPACE_2)
                                        .rounded(metrics::RADIUS_SM)
                                        .text_color(colors.text_accent)
                                        .cursor_pointer()
                                        .tab_stop(true)
                                        .focusable()
                                        .focus_visible(crate::controls::focus_style(colors))
                                        .child(t!(cx, "snippet-attach-as-file").into_owned()),
                                    cx,
                                    |this, _, cx| this.attach_oversized_paste(cx),
                                ))
                            }),
                    )
                },
            )
            .when_some(slash_notice, |d, n| {
                d.child(
                    div()
                        .flex()
                        .gap_2()
                        .pt_1()
                        .text_xs()
                        .child(div().text_color(error).child(n))
                        .when_some(slash_open, |d, label| {
                            d.child(
                                div()
                                    .id("composer-open-in-provider")
                                    .text_color(colors.text_accent)
                                    .child(label)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.open_in_provider(cx)),
                                    ),
                            )
                        }),
                )
            })
            .when(self.reply_root.is_some(), |d| {
                d.child(crate::controls::on_activate(
                    div()
                        .id("composer-broadcast")
                        .debug_selector(|| "composer-broadcast".to_string())
                        .role(gpui::Role::CheckBox)
                        .aria_label(t!(cx, "composer-broadcast").into_owned())
                        .aria_toggled(if self.broadcast {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                        .pt_1()
                        .flex()
                        .items_center()
                        .gap(themes::metrics::SPACE_2)
                        .text_xs()
                        .text_color(colors.text_muted)
                        .tab_stop(true)
                        .focusable()
                        .focus_visible(crate::controls::focus_style(colors))
                        .cursor_pointer()
                        .child(
                            div()
                                .debug_selector(|| "composer-broadcast-checkbox".to_string())
                                .child(crate::controls::check_box(
                                    self.broadcast,
                                    colors,
                                    tokens.border_control,
                                )),
                        )
                        .child(t!(cx, "composer-broadcast").into_owned()),
                    cx,
                    |this, w, cx| this.toggle_broadcast(&ToggleBroadcast, w, cx),
                ))
            })
            .into_any_element()
    }
}

/// An icon of the composer's rows, in the muted text colour of its button (the
/// button's hover colour does not reach an svg, so it stays muted).
fn icon_glyph(name: crate::icon::IconName) -> gpui::AnyElement {
    crate::icon::Icon::new(name)
        .size(themes::metrics::ICON_SIZE_MD)
        .into_any_element()
}

/// An icon button of the composer's action row, as the board draws them.
fn action_button(
    id: &'static str,
    icon: crate::icon::IconName,
    tip: fn(&App) -> String,
    colors: &themes::ThemeColors,
) -> gpui::Stateful<gpui::Div> {
    let hover = colors.element_selected;
    div()
        .id(id)
        .debug_selector(|| id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(themes::metrics::LAYOUT_ROW_COMPACT)
        .h(themes::metrics::LAYOUT_ROW_COMPACT)
        .rounded_sm()
        .cursor_pointer()
        .hover(move |d| d.bg(hover))
        .tooltip(crate::reactions::tooltip_lazy(tip))
        .child(crate::icon::Icon::new(icon).size(themes::metrics::ICON_SIZE_LG))
}
