use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use domain::{
    Capabilities, ClientMsgId, ContentParser, Conversation, ConversationId, CustomEmoji,
    DeliveryState, EmojiRef, FileId, FileRef, ImageRef, Member, Message, MessageId, MessageTs,
    MessageWindow, PlainParser, RemoteUrl, UserId,
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ListAlignment, ListState, Render, SharedString, Window, actions, div, list, prelude::*, px,
};
use i18n::Translate as _;
use jiff::tz::TimeZone;
use settings::{Clock, InlineImages};
use themes::ActiveTheme;
use themes::parla_tokens;

use crate::clip::{self, ClipAction, ClipControl, ClipController, ClipKey, ClipState};
use crate::emoji_picker::{EmojiPicked, EmojiPicker, PickerDismissed};
use crate::files::{self, FileNotice};
use crate::message_actions::{MessageAction, actions_for};
use crate::message_row::{AuthorLink, MessageRow, author_name, continues, format_time, user_name};
use crate::reactions::{Pill, pills_row};
use crate::render_cache::RenderCache;
use crate::rich_text::{
    OpenConversation, OpenUrl, OpenUser, RichColors, RichTextCx, render_rich_text,
};
use crate::snippet;
use crate::thread_summary::{SummaryAuthor, ThreadSummary, summary_for};
use crate::timeline_dividers::{self, Lead};
use crate::vtt as clip_vtt;

actions!(
    timeline,
    [
        EditMessage,
        DeleteMessage,
        AddReaction,
        OpenThread,
        MarkUnread,
        JumpToLatest,
        TogglePin,
        ToggleSaved,
        CopyLink
    ]
);

/// How long a jump target stays highlighted (one shot, not an idle timer).
pub const HIGHLIGHT: std::time::Duration = std::time::Duration::from_millis(2000);

/// Key context of the timeline view.
pub const KEY_CONTEXT: &str = "Timeline";

/// Snippet cards the timeline keeps built at most.
const SNIPPET_CACHE_MAX: usize = 64;

/// A built snippet card and what it was built from.
struct CachedSnippet {
    preview: Arc<str>,
    lines: Option<u32>,
    size: u64,
    card: snippet::SnippetCard,
}

/// Completion callback of a caption fetch: the bytes, or why it failed.
pub type CaptionsDone = Box<dyn FnOnce(Result<Vec<u8>, String>, &mut App)>;

/// Space, Left, Right or M without modifiers.
fn clip_key_of(ev: &gpui::KeyDownEvent) -> Option<ClipKey> {
    let m = &ev.keystroke.modifiers;
    if m.platform || m.control || m.alt || m.shift || m.function {
        return None;
    }
    ClipKey::from_key(&ev.keystroke.key)
}

/// A clip card is a tab stop with the focus ring, but a click on it does not
/// take keyboard focus (the composer keeps it): reach it with Tab.
fn clip_card_focus(
    card: gpui::Stateful<gpui::Div>,
    colors: &themes::ThemeColors,
) -> gpui::Stateful<gpui::Div> {
    card.tab_stop(true)
        .focusable()
        .focus_visible(crate::controls::focus_style(colors))
        .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
            window.prevent_default()
        })
}

/// Completion callback of a clip download.
pub type ClipDone = Box<dyn FnOnce(Result<(), String>, &mut App)>;

/// What the timeline needs from the session, so `components` does not depend on `parla`.
pub trait TimelineSource {
    fn window(&self, c: &ConversationId) -> Option<&MessageWindow>;
    /// The thread under `root` is open in a panel: its summary pill shows the
    /// open state and the root row reads as selected.
    fn thread_open(&self, _root: &MessageId) -> bool {
        false
    }
    fn member(&self, u: &UserId) -> Option<&Member>;
    fn self_user(&self) -> Option<&UserId>;
    fn capabilities(&self) -> &Capabilities;
    /// The workspace session ended: dropped files are not uploaded.
    fn session_expired(&self) -> bool {
        false
    }
    /// The realtime connection is down: unsent messages wait as "Queued".
    fn offline(&self) -> bool {
        false
    }
    fn load_older(&mut self, c: ConversationId, cx: &mut App);
    /// The first history page of `c` failed to load (the pane offers Retry).
    fn history_failed(&self, _c: &ConversationId) -> bool {
        false
    }
    /// Requests the first history page of `c` again.
    fn retry_history(&mut self, _c: ConversationId, _cx: &mut App) {}
    /// The message a jump (search hit, pin, ...) is bringing into view, if any.
    fn jump(&self) -> Option<&MessageId> {
        None
    }
    /// The read marker `c` had when it was opened, before opening marked it
    /// read (`None` inside: nothing was unread). `None` when the source does
    /// not track it; the timeline then reads the conversation's marker.
    fn opened_marker(&self, _c: &ConversationId) -> Option<Option<MessageTs>> {
        None
    }
    /// Leaves a detached window (opened by a jump) for the newest messages.
    fn jump_to_latest(&mut self, _c: ConversationId, _cx: &mut App) {}
    fn edit(&mut self, id: MessageId, text: String, cx: &mut App);
    fn delete(&mut self, id: MessageId, cx: &mut App);
    /// Adds (`add`) or removes the current user's reaction.
    fn react(&mut self, id: MessageId, emoji: EmojiRef, add: bool, cx: &mut App);
    /// Custom emoji of the workspace, for the picker.
    fn custom_emoji(&self) -> Vec<CustomEmoji>;
    /// The user group `id`, so a group mention shows its handle and is
    /// highlighted when the user is in it (none by default).
    fn user_group(&self, _id: &str) -> Option<&domain::UserGroup> {
        None
    }
    /// Image of the custom emoji `shortcode`, if the workspace has one. Called
    /// while rendering, so implementations must look it up, not copy the table.
    fn custom_emoji_image(&self, shortcode: &str) -> Option<domain::ImageRef> {
        self.custom_emoji()
            .into_iter()
            .find(|c| &*c.shortcode == shortcode)
            .map(|c| c.image)
    }
    /// Emoji used before in this workspace, newest first, for the picker's Recent row.
    fn recent_emoji(&self) -> Vec<EmojiRef> {
        Vec::new()
    }
    /// Parser for the provider's message markup (plain text by default).
    fn content(&self) -> Arc<dyn ContentParser> {
        Arc::new(PlainParser)
    }
    fn conversation(&self, _id: &ConversationId) -> Option<&Conversation> {
        None
    }
    /// The whole workspace, when the source holds it: names conversations
    /// like the header does.
    fn workspace_state(&self) -> Option<&domain::WorkspaceState> {
        None
    }
    fn provider_name(&self) -> &str {
        "provider"
    }
    /// Permalink to the message in the provider's own client, when it has one.
    fn web_url(&self, _id: &MessageId) -> Option<String> {
        None
    }
    fn open_conversation(&mut self, _id: ConversationId, _cx: &mut App) {}
    /// A huddle is live in `c` (a call card may offer Join).
    fn huddle_live(&self, _c: &ConversationId) -> bool {
        false
    }
    /// Joins the live huddle of `c` (opens it per the call settings).
    fn join_huddle(&mut self, _c: ConversationId, _cx: &mut App) {}
    /// Opens the direct message with `user`, when there is one.
    fn open_direct(&mut self, _user: UserId, _cx: &mut App) {}
    /// Shows the profile of `user` (an author or a mention was clicked).
    fn open_profile(&mut self, _user: UserId, _cx: &mut App) {}
    /// Downloads `file` through the provider (host-checked) into the downloads folder.
    fn download(&mut self, _file: &FileRef, _cx: &mut App) {}
    /// Opens the media viewer on `gallery` (an image or PDF was clicked). A
    /// source without a viewer downloads the file instead.
    fn open_media(&mut self, gallery: crate::media_viewer::Gallery, cx: &mut App) {
        if let Some(item) = gallery.current() {
            let file = item.file.clone();
            self.download(&file, cx);
        }
    }
    /// Opens the code viewer on the current file of `gallery` (a snippet's Open
    /// in viewer, or a code file was clicked). A source without a viewer
    /// downloads the file instead.
    fn open_code(&mut self, gallery: crate::media_viewer::Gallery, cx: &mut App) {
        if let Some(item) = gallery.current() {
            let file = item.file.clone();
            self.download(&file, cx);
        }
    }
    /// Fetches a clip rendition through the provider (host-checked) into `dest`,
    /// then calls `done`. Not available by default.
    fn download_clip(&mut self, _url: RemoteUrl, _dest: PathBuf, done: ClipDone, cx: &mut App) {
        done(Err("clips cannot be fetched here".into()), cx);
    }
    /// Fetches a clip's captions through the provider (host-checked), at most
    /// `max_bytes` (the fetch stops past it), then calls `done`. Not available
    /// by default.
    fn fetch_captions(
        &mut self,
        _url: RemoteUrl,
        _max_bytes: u64,
        done: CaptionsDone,
        cx: &mut App,
    ) {
        // Never shown: a failed fetch shows the localized captions error.
        done(Err("unsupported".into()), cx);
    }
    /// Attaches local files to the draft of `c` (or its thread); they upload on Send.
    fn stage(
        &mut self,
        _c: ConversationId,
        _thread: Option<MessageTs>,
        _paths: Vec<PathBuf>,
        _cx: &mut App,
    ) {
    }
    /// Latest download or upload result, shown as a toast.
    fn file_notice(&self) -> Option<FileNotice> {
        None
    }
    fn dismiss_file_notice(&mut self, _cx: &mut App) {}
    /// Re-queues the outgoing message `id` that failed to send.
    fn retry_send(&mut self, _id: ClientMsgId, _cx: &mut App) {}
    /// Discards the outgoing message `id` that failed to send.
    fn discard_send(&mut self, _id: ClientMsgId, _cx: &mut App) {}
    /// Pins (`pinned`) or unpins the message.
    fn set_pinned(&mut self, _id: MessageId, _pinned: bool, _cx: &mut App) {}
    /// Saves the message for later (`saved`) or removes it from the saved items.
    fn set_saved(&mut self, _id: MessageId, _saved: bool, _cx: &mut App) {}
    /// Marks everything from `id` on as unread again (the read marker moves before it).
    fn mark_unread(&mut self, _id: MessageId, _cx: &mut App) {}
    /// "Remind me about this" is offered: Parla keeps the reminders itself.
    fn can_remind(&self) -> bool {
        false
    }
    /// Adds a reminder about message `id`, due at `at` (unix seconds).
    fn remind_about(&mut self, _id: MessageId, _at: i64, _cx: &mut App) {}
    /// The zone reminder presets are read in (the Reminders view's); `None`
    /// uses the timeline's.
    fn reminder_zone(&self, _cx: &App) -> Option<TimeZone> {
        None
    }
    /// Reports where the user's attention is: this conversation, whether the
    /// view is pinned to the newest message and whether the window is active.
    fn focus(
        &mut self,
        _conversation: ConversationId,
        _at_bottom: bool,
        _window_active: bool,
        _cx: &mut App,
    ) {
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelineEvent {
    /// The user asked to edit one of their own messages; the composer takes over.
    EditRequested { id: MessageId, text: String },
    /// The user asked to delete one of their own messages; the host opens the
    /// confirmation dialog. `text` is a plain, one-line, capped preview of
    /// the message and is empty when the message has no text (files only).
    DeleteRequested { id: MessageId, text: String },
    /// The pending delete went away without an answer (the conversation or
    /// the selection changed); the host closes its confirmation dialog.
    DeleteCancelled,
    /// The user asked to open the thread under this root message.
    OpenThread(MessageId),
}

fn is_own(m: &Message, me: Option<&UserId>) -> bool {
    m.author.is_some() && m.author.as_ref() == me && m.state == DeliveryState::Delivered
}

/// What a row is, across its life: the client id while it has one (a sent
/// message keeps it when the server confirms it under its own ts), else the
/// message id. The list edits follow keys, so a confirmation updates the row
/// in place instead of removing one row and inserting another.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum RowKey {
    Client(domain::ClientMsgId),
    Id(MessageId),
}

fn row_key(m: &Message) -> RowKey {
    match &m.client_msg_id {
        Some(c) => RowKey::Client(c.clone()),
        None => RowKey::Id(m.id.clone()),
    }
}

/// How the list must change to follow a new message window.
#[derive(Debug, PartialEq, Eq)]
enum IdOps {
    Same,
    /// Nothing in common: start over (scroll position is lost).
    Reset,
    /// `(old range, new item count)` splices, applied in order.
    Splice(Vec<(std::ops::Range<usize>, usize)>),
}

/// The list edits that turn `old` into `new`, keeping the scroll position when
/// older pages are prepended, newer messages appended, or messages removed
/// (a delete in the middle, the top trimmed).
fn id_splices<T: PartialEq>(old: &[T], new: &[T]) -> IdOps {
    if old == new {
        return IdOps::Same;
    }
    if old.is_empty() || new.is_empty() {
        return IdOps::Reset;
    }
    // Prepend (and append): the old window sits inside the new one.
    if let Some(p) = new.iter().position(|i| *i == old[0])
        && new[p..].starts_with(old)
    {
        let mut ops = Vec::new();
        if p > 0 {
            ops.push((0..0, p));
        }
        let tail = p + old.len();
        if new.len() > tail {
            ops.push((tail..tail, new.len() - tail));
        }
        return IdOps::Splice(ops);
    }
    let mut ops = Vec::new();
    // Top trim (possibly with newer messages appended).
    let mut old = old;
    if let Some(q) = old.iter().position(|i| *i == new[0])
        && q > 0
        && new.starts_with(&old[q..])
    {
        ops.push((0..q, 0));
        let appended = new.len() - (old.len() - q);
        old = &old[q..];
        if appended > 0 {
            ops.push((old.len()..old.len(), appended));
        }
        return IdOps::Splice(ops);
    }
    // Anything else: replace the differing middle, keep both ends.
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix);
    if prefix == 0 && suffix == 0 {
        return IdOps::Reset;
    }
    IdOps::Splice(vec![(
        prefix..old.len() - suffix,
        new.len() - suffix - prefix,
    )])
}

/// Hash of what a row's height depends on, so a notify that changed nothing
/// visible does not lay every row out again.
fn window_signature(msgs: &[Message]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for m in msgs {
        m.id.ts.0.hash(&mut h);
        m.edited.as_ref().map(|e| &e.0).hash(&mut h);
        m.text.len().hash(&mut h);
        std::mem::discriminant(&m.state).hash(&mut h);
        m.author.as_ref().map(|a| &a.0).hash(&mut h);
        m.reply_count.hash(&mut h);
        m.files.len().hash(&mut h);
        m.pinned.hash(&mut h);
        m.saved.hash(&mut h);
        for r in m.reactions.iter() {
            (&r.emoji.shortcode, r.count, r.me).hash(&mut h);
        }
    }
    h.finish()
}

/// The intro of a direct message or group: its name as the header shows it,
/// and the line under it (a self DM has its own).
pub fn intro_copy(
    state: &domain::WorkspaceState,
    c: &Conversation,
    l: &i18n::Localizer,
) -> (String, String) {
    let name = crate::header::header_info(state, c, l).title;
    let own =
        c.kind == domain::ConversationKind::Direct && c.peer.is_some() && c.peer == state.self_user;
    let line = if own {
        t!(l, "timeline-intro-self")
    } else if c.kind == domain::ConversationKind::Group {
        t!(l, "timeline-intro-group", name = name.clone())
    } else {
        t!(l, "timeline-intro", name = name.clone())
    }
    .into_owned();
    (name, line)
}

pub struct Timeline<S: TimelineSource + 'static> {
    session: Entity<S>,
    conversation: ConversationId,
    clock: Clock,
    tz: TimeZone,
    list: ListState,
    ids: Vec<MessageId>,
    /// [`RowKey`] of each row of `ids`.
    keys: Vec<RowKey>,
    /// See [`window_signature`].
    sig: u64,
    /// Dividers above each row, computed when the window changes.
    leads: Vec<Lead>,
    /// The read marker when the conversation was opened (`None` inside: nothing
    /// unread then). It stays put while reading; `None` outside: not captured yet.
    open_marker: Option<Option<MessageTs>>,
    /// The local date the day labels were computed for.
    leads_day: Option<jiff::civil::Date>,
    /// `has_more_before` of the window the leads were computed for.
    more_before: bool,
    selected: Option<MessageId>,
    pending_delete: Option<MessageId>,
    requested_older: Option<MessageId>,
    rendered: usize,
    cache: RenderCache,
    code_font: SharedString,
    picker: Option<Entity<EmojiPicker>>,
    picker_target: Option<MessageId>,
    inline_images: InlineImages,
    revealed: HashSet<FileId>,
    /// Snippet cards shown in full (the rest show their first lines). One id per snippet the
    /// user expanded in this view; ids are short and the set is dropped with the view.
    snippets_expanded: HashSet<FileId>,
    /// Snippet cards already built from their file, so a repaint builds none (keyed by file,
    /// valid while the file's preview is the same text). Never more than
    /// [`SNIPPET_CACHE_MAX`]: the cache is emptied when it fills.
    snippet_cards: HashMap<FileId, CachedSnippet>,
    /// Snippet cards built so far (for tests).
    snippet_card_builds: usize,
    /// Clips the user started; at most one holds a decoder.
    clips: HashMap<FileId, ClipController>,
    /// Clip rows painted since the last render, for the scroll-away sweep.
    clips_seen: HashSet<FileId>,
    upload_notice: Option<Vec<crate::upload::Refusal>>,
    focus: FocusHandle,
    /// The open clip's seek bar (one clip is open at a time).
    clip_seek_focus: FocusHandle,
    /// Last `(conversation, at_bottom, window_active)` sent to the source.
    reported: Option<(ConversationId, bool, bool)>,
    window_active: bool,
    activation: Option<gpui::Subscription>,
    /// Message whose context menu is open.
    menu: Option<MessageId>,
    /// The message whose "Remind me" presets are open.
    remind_menu: Option<MessageId>,
    /// The first preset, focused when they open.
    remind_first: FocusHandle,
    /// Message under the pointer (shows its "..." button).
    hovered: Option<MessageId>,
    /// Message to scroll to and select once it is in the loaded window.
    reveal: Option<MessageId>,
    /// Jump target shown highlighted until the next selection or [`HIGHLIGHT`] passes.
    highlighted: Option<MessageId>,
    /// Last jump this view acted on, so each jump is shown once.
    handled_jump: Option<MessageId>,
    highlight_timer: Option<gpui::Task<()>>,
    /// The one timer for the earliest status expiry among the loaded authors
    /// (the deadline in Unix seconds, and the task): rows read the clock when
    /// drawn, but a retained row only draws again when told to.
    status_wake: Option<(i64, gpui::Task<()>)>,
    follow_mode: gpui::FollowMode,
    open_url: Option<OpenUrl>,
    /// The rows' open conversation, open user and join huddle callbacks: the
    /// same for every row, so built once instead of on each row's redraw.
    row_fns: Option<(OpenConversation, OpenUser, OpenConversation)>,
    /// Row density, refreshed from the settings global (never read per row).
    density: settings::Density,
    /// `messages.show_avatars`, refreshed from the settings global.
    show_avatars: bool,
}

/// Background of a message that mentions the user; the selected surface wins.
fn mention_tint<T>(mention: bool, selected: bool, tint: T) -> Option<T> {
    (mention && !selected).then_some(tint)
}

impl<S: TimelineSource + 'static> EventEmitter<TimelineEvent> for Timeline<S> {}

impl<S: TimelineSource + 'static> Timeline<S> {
    pub fn new(
        session: Entity<S>,
        conversation: ConversationId,
        clock: Clock,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&session, |this, _, cx| {
            this.sync(cx);
            this.arm_status_wake(cx);
            this.apply_jump(cx);
            this.apply_reveal(cx);
            cx.notify();
        })
        .detach();
        // A language switch changes text widths: lay rows out again.
        cx.observe_global::<i18n::gpui::I18n>(|this, cx| {
            this.list.remeasure();
            cx.notify();
        })
        .detach();
        // A density, clock, avatar, image or code font change moves row
        // content or height: copy them once and lay rows out again.
        cx.observe_global::<settings::SettingsStore>(|this, cx| {
            let density = crate::density::current(cx);
            let mut changed = density != this.density;
            this.density = density;
            if let Some(d) = crate::live_settings::display(cx) {
                changed |= d.clock != this.clock
                    || d.inline_images != this.inline_images
                    || d.code_font != this.code_font
                    || d.show_avatars != this.show_avatars;
                this.clock = d.clock;
                this.inline_images = d.inline_images;
                this.code_font = d.code_font;
                this.show_avatars = d.show_avatars;
            }
            if changed {
                this.list.remeasure();
                cx.notify();
            }
        })
        .detach();
        // design: allow list overdraw estimate in pixels, a rendering heuristic and not chrome
        let list = ListState::new(0, ListAlignment::Bottom, px(400.0));
        list.set_follow_mode(gpui::FollowMode::Tail);
        let view = cx.entity().downgrade();
        list.set_scroll_handler(move |ev, _, cx| {
            let top = ev.visible_range.start == 0 && ev.count > 0;
            let view = view.clone();
            // Deferred: the handler can run inside an update of the view itself.
            cx.defer(move |cx| {
                view.update(cx, |t, cx| {
                    if top {
                        t.maybe_load_older(cx);
                    }
                    t.report_focus(cx);
                })
                .ok();
            });
        });
        let mut this = Self {
            session,
            conversation,
            clock,
            tz: crate::time_zone(cx),
            list,
            ids: Vec::new(),
            keys: Vec::new(),
            sig: 0,
            leads: Vec::new(),
            open_marker: None,
            leads_day: None,
            more_before: false,
            selected: None,
            pending_delete: None,
            requested_older: None,
            rendered: 0,
            cache: RenderCache::new(),
            code_font: crate::live_settings::code_font(cx),
            picker: None,
            picker_target: None,
            inline_images: crate::live_settings::display(cx)
                .map_or(InlineImages::OnClick, |d| d.inline_images),
            revealed: HashSet::new(),
            snippets_expanded: HashSet::new(),
            snippet_cards: HashMap::new(),
            snippet_card_builds: 0,
            clips: HashMap::new(),
            clips_seen: HashSet::new(),
            upload_notice: None,
            focus: cx.focus_handle(),
            clip_seek_focus: cx.focus_handle().tab_stop(true),
            reported: None,
            window_active: true,
            activation: None,
            menu: None,
            remind_menu: None,
            remind_first: cx.focus_handle().tab_stop(true),
            hovered: None,
            reveal: None,
            highlighted: None,
            handled_jump: None,
            highlight_timer: None,
            status_wake: None,
            follow_mode: gpui::FollowMode::Tail,
            open_url: None,
            row_fns: None,
            density: crate::density::current(cx),
            show_avatars: crate::live_settings::display(cx).is_none_or(|d| d.show_avatars),
        };
        this.sync(cx);
        this.arm_status_wake(cx);
        this.apply_jump(cx);
        this.schedule_focus_report(cx);
        this
    }

    fn schedule_focus_report(&self, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            view.update(cx, |t, cx| t.report_focus(cx)).ok();
        });
    }

    /// Tells the source about a change of conversation, scroll position
    /// (bottom or not) or window activation; unchanged state is not repeated.
    fn report_focus(&mut self, cx: &mut Context<Self>) {
        let now = (
            self.conversation.clone(),
            self.list.is_following_tail(),
            self.window_active,
        );
        if self.reported.as_ref() == Some(&now) {
            return;
        }
        self.reported = Some(now.clone());
        self.session
            .update(cx, |s, cx| s.focus(now.0, now.1, now.2, cx));
    }

    /// The selected message, if any.
    pub fn selected(&self) -> Option<&MessageId> {
        self.selected.as_ref()
    }

    /// The jump target currently highlighted, if any.
    pub fn highlighted(&self) -> Option<&MessageId> {
        self.highlighted.as_ref()
    }

    /// True while the view shows a window detached from the newest messages.
    pub fn is_detached(&self, cx: &App) -> bool {
        self.session
            .read(cx)
            .window(&self.conversation)
            .is_some_and(|w| !w.is_live())
    }

    /// Leaves a detached window for the newest messages.
    pub fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        let c = self.conversation.clone();
        self.highlighted = None;
        self.highlight_timer = None;
        self.session.update(cx, |s, cx| s.jump_to_latest(c, cx));
        cx.notify();
    }

    pub fn set_open_url(&mut self, open_url: OpenUrl) {
        self.open_url = Some(open_url);
    }

    fn jump_to_latest_action(&mut self, _: &JumpToLatest, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_detached(cx) {
            self.jump_to_latest(cx);
        }
    }

    /// The earliest future status expiry among the authors in the window.
    fn next_status_expiry(&self, cx: &App) -> Option<i64> {
        let s = self.session.read(cx);
        let now = crate::now_secs(cx);
        let w = s.window(&self.conversation)?;
        w.messages()
            .iter()
            .filter_map(|m| m.author.as_ref())
            .filter_map(|u| s.member(u))
            .filter_map(|mem| mem.status.as_ref()?.expires)
            .filter(|e| *e > now)
            .min()
    }

    /// Keeps one timer on the earliest status expiry: when it fires the rows
    /// draw again without that status. Nothing runs while no status expires
    /// (and nothing in frozen screenshots).
    fn arm_status_wake(&mut self, cx: &mut Context<Self>) {
        let next = if crate::motion(cx) == crate::Motion::Frozen {
            None
        } else {
            self.next_status_expiry(cx)
        };
        if self.status_wake.as_ref().map(|(at, _)| *at) == next {
            return;
        }
        self.status_wake = next.map(|at| {
            let wait = std::time::Duration::from_secs(
                u64::try_from(at - crate::now_secs(cx)).unwrap_or(0),
            );
            (
                at,
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    this.update(cx, |t, cx| {
                        t.status_wake = None;
                        t.list.remeasure();
                        t.arm_status_wake(cx);
                        cx.notify();
                    })
                    .ok();
                }),
            )
        });
    }

    /// Scrolls to a new jump target of this conversation and highlights it once.
    fn apply_jump(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.session.read(cx).jump().cloned() else {
            return;
        };
        if id.conversation != self.conversation || self.handled_jump.as_ref() == Some(&id) {
            return;
        }
        self.handled_jump = Some(id.clone());
        self.reveal(id.clone(), cx);
        self.highlighted = Some(id);
        self.highlight_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HIGHLIGHT).await;
            this.update(cx, |t, cx| {
                t.highlighted = None;
                t.highlight_timer = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Scrolls to `id` and selects it. When it is older than the loaded
    /// window, older pages load until it shows up or history runs out.
    pub fn reveal(&mut self, id: MessageId, cx: &mut Context<Self>) {
        self.reveal = Some(id);
        self.apply_reveal(cx);
    }

    fn apply_reveal(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.reveal.clone() else {
            return;
        };
        if id.conversation != self.conversation {
            self.reveal = None;
            return;
        }
        if let Some(ix) = self.ids.iter().position(|i| *i == id) {
            self.reveal = None;
            self.select(Some(id), cx);
            self.list.scroll_to(gpui::ListOffset {
                item_ix: ix,
                offset_in_item: px(0.0),
            });
            return;
        }
        let more = self
            .session
            .read(cx)
            .window(&self.conversation)
            .is_some_and(|w| w.has_more_before());
        if more {
            self.requested_older = None;
            self.maybe_load_older(cx);
        } else if !self.ids.is_empty() {
            // History is exhausted and the message is not in it. (An empty
            // window may still be loading, so the request waits.)
            self.reveal = None;
        }
    }

    /// Marks `id` and everything after it unread.
    pub fn mark_unread(&mut self, id: MessageId, cx: &mut Context<Self>) {
        self.menu = None;
        // The "New" divider moves to the marked message.
        if let Some(w) = self.session.read(cx).window(&self.conversation) {
            let before = w
                .messages()
                .iter()
                .take_while(|m| m.id != id)
                .last()
                .map(|m| m.id.ts.clone())
                .or_else(|| Some(id.ts.just_before()));
            self.open_marker = Some(before);
        }
        self.session.update(cx, |s, cx| s.mark_unread(id, cx));
        self.recompute_leads(cx);
        self.list.remeasure();
        cx.notify();
    }

    fn mark_unread_action(&mut self, _: &MarkUnread, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            self.mark_unread(id, cx);
        }
    }

    /// Font family of code spans (`buffer_font_family`).
    pub fn set_code_font(&mut self, family: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.code_font = family.into();
        self.list.remeasure();
        cx.notify();
    }

    /// `messages.inline_images`: how image files preview in the timeline.
    pub fn set_inline_images(&mut self, mode: InlineImages, cx: &mut Context<Self>) {
        self.inline_images = mode;
        self.list.remeasure();
        cx.notify();
    }

    /// Attaches dropped files (or the paths of any other attach route) to this conversation's draft.
    /// Files over 1 GB are refused; the reason is kept in [`Self::upload_notice`].
    pub fn drop_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.session.read(cx).session_expired() {
            return;
        }
        let planned = crate::upload::plan(&paths);
        self.upload_notice = (!planned.refused.is_empty()).then_some(planned.refused);
        if !planned.accepted.is_empty() {
            let c = self.conversation.clone();
            let ok: Vec<PathBuf> = planned.accepted.into_iter().map(|u| u.path).collect();
            self.session.update(cx, |s, cx| s.stage(c, None, ok, cx));
        }
        cx.notify();
    }

    /// Why the last attach was (partly) refused, if it was.
    pub fn upload_notice(&self, l: &i18n::Localizer) -> Option<String> {
        self.upload_notice
            .as_ref()
            .map(|r| crate::upload::refusals_text(r, l))
    }

    /// Opens the file picker and attaches the chosen files to the draft.
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update(cx, |t, cx| t.drop_paths(paths, cx)).ok();
            }
        })
        .detach();
    }

    fn clip_click(&mut self, f: FileRef, msg: MessageId, cx: &mut Context<Self>) {
        if !self.clips.contains_key(&f.id) {
            // One decoder at a time keeps memory bounded.
            for c in self.clips.values_mut() {
                c.release();
            }
            self.clips.retain(|_, c| *c.state() != ClipState::Poster);
            let mut c = ClipController::new(
                &f,
                &settings::paths::cache_dir(),
                Box::new(crate::platform::new_player),
            );
            c.set_on(self.clip_handler(f.clone(), msg.clone(), cx));
            self.clips.insert(f.id.clone(), c);
        }
        self.clips_seen.insert(f.id.clone());
        let Some(ctl) = self.clips.get_mut(&f.id) else {
            return;
        };
        match ctl.click() {
            ClipAction::None => {}
            ClipAction::OpenInBrowser => {
                if let Some(url) = self.session.read(cx).web_url(&msg)
                    && let crate::links::LinkAction::OpenExternal(u) = crate::links::open_link(&url)
                {
                    cx.open_url(&u);
                }
            }
            ClipAction::Download { url, dest } => {
                let view = cx.entity().downgrade();
                let id = f.id.clone();
                self.session.update(cx, |s, cx| {
                    s.download_clip(
                        url,
                        dest,
                        Box::new(move |r, cx| {
                            view.update(cx, |t, cx| t.clip_downloaded(id, r, cx)).ok();
                        }),
                        cx,
                    )
                });
            }
        }
        cx.notify();
    }

    fn clip_downloaded(&mut self, id: FileId, r: Result<(), String>, cx: &mut Context<Self>) {
        if let Some(c) = self.clips.get_mut(&id) {
            match r {
                Ok(()) => c.downloaded(),
                Err(e) => c.download_failed(e),
            }
            self.list.remeasure();
            cx.notify();
        }
    }

    /// Runs a clip control from the card.
    /// Runs a clip control from the card; returns whether it applied.
    fn clip_control(
        &mut self,
        f: FileRef,
        msg: MessageId,
        control: ClipControl,
        cx: &mut Context<Self>,
    ) -> bool {
        match control {
            ClipControl::PlayPause => {
                self.clip_click(f, msg, cx);
                return true;
            }
            ClipControl::Key(k) => return self.clip_key(f, msg, k, cx),
            _ => {}
        }
        let Some(ctl) = self.clips.get_mut(&f.id) else {
            return false;
        };
        match control {
            ClipControl::Seek(x) => ctl.seek_fraction(x),
            ClipControl::SeekBy(ms) => ctl.seek_by(ms),
            ClipControl::Speed => {
                ctl.cycle_speed();
            }
            ClipControl::Mute => {
                ctl.toggle_mute();
            }
            ClipControl::Volume(v) => ctl.set_volume(v),
            ClipControl::VolumeBy(d) => {
                let v = if ctl.muted() { 0.0 } else { ctl.volume() };
                // Whole percents, so steps land on 0.8, not 0.79999.
                ctl.set_volume(((v + d) * 100.0).round() / 100.0);
            }
            ClipControl::Captions => {
                if let Some(url) = ctl.toggle_captions() {
                    self.fetch_captions(f.id.clone(), url, cx);
                }
                self.list.remeasure();
            }
            ClipControl::PlayPause | ClipControl::Key(_) => {}
        }
        cx.notify();
        true
    }

    /// A key on a focused clip card; returns whether it applied, so an unused
    /// Left or Right still reaches the timeline.
    fn clip_key(
        &mut self,
        f: FileRef,
        msg: MessageId,
        key: ClipKey,
        cx: &mut Context<Self>,
    ) -> bool {
        if key == ClipKey::PlayPause {
            self.clip_click(f, msg, cx);
            return true;
        }
        let applied = self.clips.get_mut(&f.id).is_some_and(|c| c.key(key));
        if applied {
            cx.notify();
        }
        applied
    }

    /// The control handler a clip keeps for its open life: built once, so a
    /// frame clones no file or message.
    fn clip_handler(
        &self,
        f: FileRef,
        msg: MessageId,
        cx: &mut Context<Self>,
    ) -> clip::OnClipControl {
        let this = cx.entity().downgrade();
        Rc::new(move |control, _: &mut Window, cx: &mut App| {
            this.update(cx, |t, cx| {
                t.clip_control(f.clone(), msg.clone(), control, cx)
            })
            .unwrap_or(false)
        })
    }

    /// Fetches a clip's captions (at most [`clip_vtt::MAX_BYTES`], capped while
    /// streaming, kept in memory only), then parses them off the UI thread.
    fn fetch_captions(&mut self, id: FileId, url: RemoteUrl, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        self.session.update(cx, |s, cx| {
            s.fetch_captions(
                url,
                clip_vtt::MAX_BYTES as u64,
                // Deferred: a source may answer at once, inside this update.
                Box::new(move |r, cx| {
                    cx.defer(move |cx| {
                        view.update(cx, |t, cx| match r {
                            Ok(bytes) => t.load_captions(id, bytes, cx),
                            Err(_) => t.captions_failed(&id, cx),
                        })
                        .ok();
                    });
                }),
                cx,
            )
        });
    }

    fn load_captions(&mut self, id: FileId, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let parse = cx.background_spawn(async move {
            (bytes.len() <= clip_vtt::MAX_BYTES)
                .then(|| clip_vtt::parse(&String::from_utf8_lossy(&bytes)))
        });
        cx.spawn(async move |this, cx| {
            let cues = parse.await;
            this.update(cx, |t, cx| match cues {
                Some(cues) => {
                    if let Some(c) = t.clips.get_mut(&id) {
                        c.captions_loaded(cues);
                        t.list.remeasure();
                        cx.notify();
                    }
                }
                None => t.captions_failed(&id, cx),
            })
            .ok();
        })
        .detach();
    }

    fn captions_failed(&mut self, id: &FileId, cx: &mut Context<Self>) {
        if let Some(c) = self.clips.get_mut(id) {
            c.captions_failed();
            self.list.remeasure();
            cx.notify();
        }
    }

    /// Moves keyboard focus to the open clip's seek bar.
    pub fn focus_clip_seek(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.clip_seek_focus, cx);
    }

    /// Puts a ready controller on a clip card, as a click and a download would.
    /// For fixtures and tests (a scripted player, a staged position).
    pub fn insert_clip(
        &mut self,
        f: FileRef,
        msg: MessageId,
        mut ctl: ClipController,
        cx: &mut Context<Self>,
    ) {
        let id = f.id.clone();
        ctl.set_on(self.clip_handler(f, msg, cx));
        // Seen, so the first render's sweep keeps it.
        self.clips_seen.insert(id.clone());
        self.clips.insert(id, ctl);
        self.list.remeasure();
        cx.notify();
    }

    /// Pauses clips whose rows were not painted since the last render and drops
    /// their decoders (a download in flight is kept).
    fn sweep_clips(&mut self) {
        // Cleared, not replaced, so the set keeps its room between frames.
        let seen = &self.clips_seen;
        self.clips.retain(|id, c| {
            if seen.contains(id) {
                return true;
            }
            c.scrolled_away();
            if *c.state() == ClipState::Downloading {
                return true;
            }
            c.release();
            false
        });
        self.clips_seen.clear();
    }

    fn file_click(&mut self, f: FileRef, msg: MessageId, cx: &mut Context<Self>) {
        if clip::is_clip(&f) {
            return self.clip_click(f, msg, cx);
        }
        if self.inline_images == InlineImages::OnClick
            && f.kind == domain::FileKind::Image
            && !f.tombstone
            && !self.revealed.contains(&f.id)
            && f.best_thumb(files::PREVIEW_W).is_some()
        {
            self.revealed.insert(f.id.clone());
            self.list.remeasure();
            cx.notify();
        } else if let Some(g) = self.gallery(&f, &msg, cx) {
            self.session.update(cx, |s, cx| s.open_media(g, cx));
        } else if crate::code_viewer::is_code(&f) {
            self.open_code(f, msg, cx);
        } else if !f.tombstone {
            self.session.update(cx, |s, cx| s.download(&f, cx));
        }
    }

    /// The viewer gallery a click on `f` opens: the images of the loaded window.
    fn gallery(
        &self,
        f: &FileRef,
        msg: &MessageId,
        cx: &App,
    ) -> Option<crate::media_viewer::Gallery> {
        if !crate::media_viewer::viewable(f) {
            return None;
        }
        let l = i18n::gpui::localizer(cx);
        let src = self.session.read(cx);
        let window = src.window(&self.conversation)?;
        let g = crate::media_viewer::Gallery::from_window(window, msg, &f.id, |u| {
            crate::message_row::author_name(u.and_then(|u| src.member(u)), &l)
        })?;
        Some(self.placed(g, cx))
    }

    /// Opens the code viewer on file `f` of message `msg` (Open in viewer on a
    /// snippet, or a click on a code file).
    fn open_code(&mut self, f: FileRef, msg: MessageId, cx: &mut Context<Self>) {
        let sender = {
            let l = i18n::gpui::localizer(cx);
            let src = self.session.read(cx);
            let author = src
                .window(&self.conversation)
                .and_then(|w| w.messages().iter().find(|m| m.id == msg))
                .and_then(|m| m.author.clone());
            crate::message_row::author_name(author.as_ref().and_then(|u| src.member(u)), &l)
        };
        let item = crate::media_viewer::ViewerItem {
            file: f,
            message: msg,
            sender,
        };
        let g = self.placed(crate::media_viewer::Gallery::new(vec![item], 0), cx);
        self.session.update(cx, |s, cx| s.open_code(g, cx));
    }

    /// `g` with the conversation's name and the clock settings.
    fn placed(&self, g: crate::media_viewer::Gallery, cx: &App) -> crate::media_viewer::Gallery {
        let l = i18n::gpui::localizer(cx);
        let src = self.session.read(cx);
        let place = match (src.workspace_state(), src.conversation(&self.conversation)) {
            (Some(state), Some(c)) => Some(
                crate::sidebar::conversation_label(
                    state,
                    &crate::sidebar::MemberIndex::default(),
                    c,
                    &l,
                )
                .to_string()
                .into(),
            ),
            (None, Some(c)) if !c.name.is_empty() => Some(c.name.to_string().into()),
            _ => None,
        };
        g.with_place(place).with_time(self.clock, self.tz.clone())
    }

    /// The card of snippet file `f`, from the cache or built once.
    fn snippet_card(&mut self, f: &FileRef, expanded: bool) -> Option<snippet::SnippetCard> {
        if f.tombstone || f.kind != domain::FileKind::Snippet {
            return None;
        }
        let preview = f.preview.as_ref()?;
        let fresh = |e: &CachedSnippet| {
            Arc::ptr_eq(&e.preview, preview) && e.lines == f.lines && e.size == f.size
        };
        if !self.snippet_cards.get(&f.id).is_some_and(fresh) {
            if self.snippet_cards.len() >= SNIPPET_CACHE_MAX {
                self.snippet_cards.clear();
            }
            let files::Card::Snippet(card) = files::card_with(f, self.inline_images, false, false)
            else {
                return None;
            };
            self.snippet_card_builds += 1;
            self.snippet_cards.insert(
                f.id.clone(),
                CachedSnippet {
                    preview: preview.clone(),
                    lines: f.lines,
                    size: f.size,
                    card,
                },
            );
        }
        let mut card = self.snippet_cards.get(&f.id)?.card.clone();
        card.expanded = expanded;
        Some(card)
    }

    /// Snippet cards built so far; a repaint of unchanged snippets adds none.
    #[doc(hidden)]
    pub fn snippet_card_builds(&self) -> usize {
        self.snippet_card_builds
    }

    /// Snippet cards held in the cache.
    #[doc(hidden)]
    pub fn snippet_cache_len(&self) -> usize {
        self.snippet_cards.len()
    }

    /// Shows the snippet `id` in full, or cuts it back to its first lines.
    pub fn toggle_snippet(&mut self, id: FileId, cx: &mut Context<Self>) {
        if !self.snippets_expanded.remove(&id) {
            self.snippets_expanded.insert(id);
        }
        self.list.remeasure();
        cx.notify();
    }

    pub fn set_clock(&mut self, clock: Clock, cx: &mut Context<Self>) {
        self.clock = clock;
        self.list.remeasure();
        cx.notify();
    }

    pub fn set_conversation(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        if c == self.conversation {
            return;
        }
        self.conversation = c;
        self.selected = None;
        self.drop_pending_delete(cx);
        self.requested_older = None;
        self.ids.clear();
        self.keys.clear();
        self.leads.clear();
        self.open_marker = None;
        self.list.reset(0);
        self.list.set_follow_mode(gpui::FollowMode::Tail);
        self.follow_mode = gpui::FollowMode::Tail;
        self.menu = None;
        self.picker = None;
        self.picker_target = None;
        self.upload_notice = None;
        self.hovered = None;
        self.reveal = None;
        self.highlighted = None;
        self.highlight_timer = None;
        self.status_wake = None;
        self.sync(cx);
        self.arm_status_wake(cx);
        self.apply_jump(cx);
        self.schedule_focus_report(cx);
        cx.notify();
    }

    /// The code font family, clock, and avatar switch the rows render with (test hooks).
    pub fn code_font(&self) -> &SharedString {
        &self.code_font
    }
    pub fn clock(&self) -> Clock {
        self.clock
    }
    pub fn inline_images(&self) -> InlineImages {
        self.inline_images
    }
    pub fn shows_avatars(&self) -> bool {
        self.show_avatars
    }

    /// The row density the list is laid out for (a test hook).
    pub fn density(&self) -> settings::Density {
        self.density
    }

    /// The dividers above each row (a test hook).
    pub fn leads(&self) -> &[Lead] {
        &self.leads
    }

    /// Number of rows built by the virtualised list since the last render.
    pub fn rendered_rows(&self) -> usize {
        self.rendered
    }

    /// True while the view is pinned to the newest message.
    pub fn is_at_bottom(&self) -> bool {
        self.list.is_following_tail()
    }

    pub fn pending_delete(&self) -> Option<&MessageId> {
        self.pending_delete.as_ref()
    }

    /// Summary row data for a thread root; `None` without the threads capability or replies.
    pub fn thread_summary(&self, id: &MessageId, cx: &App) -> Option<ThreadSummary> {
        let s = self.session.read(cx);
        if !s.capabilities().threads {
            return None;
        }
        let m = s
            .window(&self.conversation)?
            .messages()
            .iter()
            .find(|m| &m.id == id)?;
        self.thread_summary_of(m, cx)
    }

    /// [`Self::thread_summary`] for a message the caller already holds.
    fn thread_summary_of(&self, m: &domain::Message, cx: &App) -> Option<ThreadSummary> {
        let s = self.session.read(cx);
        if !s.capabilities().threads {
            return None;
        }
        let l = cx.localizer();
        let authors = m
            .reply_users
            .iter()
            .map(|u| {
                let member = s.member(u);
                SummaryAuthor {
                    name: user_name(member, l),
                    image: member.and_then(|m| {
                        m.avatar_for(themes::metrics::LAYOUT_AVATAR_THREAD.as_f32())
                            .cloned()
                    }),
                }
            })
            .collect::<Vec<_>>();
        summary_for(m, authors, crate::now_secs(cx), l)
    }

    /// Asks the host to open the thread panel for `root`.
    pub fn open_thread(&mut self, root: MessageId, cx: &mut Context<Self>) {
        if self.session.read(cx).capabilities().threads {
            cx.emit(TimelineEvent::OpenThread(root));
        }
    }

    fn open_thread_action(&mut self, _: &OpenThread, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            self.open_thread(id, cx);
        }
    }

    pub fn picker(&self) -> Option<&Entity<EmojiPicker>> {
        self.picker.as_ref()
    }

    /// Pills of message `id`, in provider order.
    pub fn reaction_pills(&self, id: &MessageId, cx: &App) -> Vec<Pill> {
        let s = self.session.read(cx);
        let Some(m) = s
            .window(&self.conversation)
            .and_then(|w| w.messages().iter().find(|m| &m.id == id))
        else {
            return Vec::new();
        };
        self.reaction_pills_of(m, cx)
    }

    /// [`Self::reaction_pills`] for a message the caller already holds.
    fn reaction_pills_of(&self, m: &domain::Message, cx: &App) -> Vec<Pill> {
        let s = self.session.read(cx);
        crate::reactions::pills_of(
            m,
            |u| s.member(u),
            |c| s.custom_emoji_image(c),
            cx.localizer(),
        )
    }

    /// Toggles the current user's reaction (what a pill click does).
    pub fn toggle_reaction(&mut self, id: MessageId, emoji: EmojiRef, cx: &mut Context<Self>) {
        let me = self
            .reaction_pills(&id, cx)
            .iter()
            .any(|p| p.emoji.shortcode == emoji.shortcode && p.me);
        self.session.update(cx, |s, cx| s.react(id, emoji, !me, cx));
    }

    /// Opens the emoji picker for message `id`.
    pub fn open_picker(&mut self, id: MessageId, window: &mut Window, cx: &mut Context<Self>) {
        let (custom, enabled, recent) = {
            let s = self.session.read(cx);
            (
                s.custom_emoji(),
                s.capabilities().custom_emoji,
                s.recent_emoji(),
            )
        };
        let picker = cx.new(|cx| {
            let mut p = EmojiPicker::new(custom, enabled, cx);
            p.set_recent(recent, cx);
            p
        });
        cx.subscribe(&picker, |this, _, ev: &EmojiPicked, cx| {
            if let Some(id) = this.picker_target.take() {
                this.picker = None;
                this.toggle_reaction_to(id, ev.0.clone(), true, cx);
            }
            cx.notify();
        })
        .detach();
        cx.subscribe(&picker, |this, _, _: &PickerDismissed, cx| {
            this.picker = None;
            this.picker_target = None;
            cx.notify();
        })
        .detach();
        window.focus(&picker.focus_handle(cx), cx);
        self.picker = Some(picker);
        self.picker_target = Some(id);
        cx.notify();
    }

    fn toggle_reaction_to(
        &mut self,
        id: MessageId,
        emoji: EmojiRef,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        self.session.update(cx, |s, cx| s.react(id, emoji, add, cx));
    }

    fn add_reaction(&mut self, _: &AddReaction, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            self.open_picker(id, window, cx);
        }
    }

    pub fn select(&mut self, id: Option<MessageId>, cx: &mut Context<Self>) {
        if self.selected != id {
            self.drop_pending_delete(cx);
        }
        self.selected = id;
        self.list.remeasure();
        cx.notify();
    }

    /// Runs the delete of the message waiting for an answer.
    pub fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.pending_delete.take() {
            self.session.update(cx, |s, cx| s.delete(id, cx));
            cx.notify();
        }
    }

    /// Keeps the message that was waiting for a delete answer.
    pub fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        if self.pending_delete.take().is_some() {
            cx.notify();
        }
    }

    /// Forgets the pending delete and tells the host to close its dialog.
    fn drop_pending_delete(&mut self, cx: &mut Context<Self>) {
        if self.pending_delete.take().is_some() {
            cx.emit(TimelineEvent::DeleteCancelled);
        }
    }

    /// The preview of `m` for the delete dialog: plain text, one line.
    fn delete_preview(&self, m: &Message, cx: &App) -> String {
        let s = self.session.read(cx);
        let none = domain::UserGroups::default();
        let groups = s.workspace_state().map_or(&none, |w| &w.user_groups);
        crate::message_row::preview_text(s.content().as_ref(), m, groups)
    }

    fn selected_own(&self, cx: &App) -> Option<Message> {
        let id = self.selected.as_ref()?;
        let s = self.session.read(cx);
        let m = s
            .window(&self.conversation)?
            .messages()
            .iter()
            .find(|m| &m.id == id)?;
        is_own(m, s.self_user()).then(|| m.clone())
    }

    fn edit_message(&mut self, _: &EditMessage, _: &mut Window, cx: &mut Context<Self>) {
        if !self.session.read(cx).capabilities().edit {
            return;
        }
        if let Some(m) = self.selected_own(cx) {
            cx.emit(TimelineEvent::EditRequested {
                id: m.id,
                text: m.text.to_string(),
            });
        }
    }

    fn delete_message(&mut self, _: &DeleteMessage, _: &mut Window, cx: &mut Context<Self>) {
        if !self.session.read(cx).capabilities().delete {
            return;
        }
        if let Some(m) = self.selected_own(cx) {
            let id = m.id.clone();
            self.pending_delete = Some(id.clone());
            let text = self.delete_preview(&m, cx);
            cx.emit(TimelineEvent::DeleteRequested { id, text });
            cx.notify();
        }
    }

    fn find(&self, id: &MessageId, cx: &App) -> Option<Message> {
        self.session
            .read(cx)
            .window(&self.conversation)?
            .messages()
            .iter()
            .find(|m| &m.id == id)
            .cloned()
    }

    /// The actions the menu of message `id` offers (see [`actions_for`]).
    pub fn actions(&self, id: &MessageId, cx: &App) -> Vec<MessageAction> {
        let s = self.session.read(cx);
        let Some(m) = self.find(id, cx) else {
            return Vec::new();
        };
        let mut out = actions_for(
            &m,
            s.capabilities(),
            is_own(&m, s.self_user()),
            s.web_url(id).is_some(),
        );
        if s.can_remind() && !out.is_empty() {
            // After Save for later, as in the board's menu.
            let at = out
                .iter()
                .position(|a| matches!(a, MessageAction::Save | MessageAction::RemoveSaved))
                .or_else(|| out.iter().position(|a| *a == MessageAction::AddReaction))
                .map_or(0, |i| i + 1);
            out.insert(at, MessageAction::RemindMe);
        }
        out
    }

    /// The message whose "Remind me" presets are open.
    pub fn remind_menu(&self) -> Option<&MessageId> {
        self.remind_menu.as_ref()
    }

    /// The first "Remind me" preset's focus.
    pub fn remind_focus(&self) -> &FocusHandle {
        &self.remind_first
    }

    /// The zone the "Remind me" presets are read and labelled in.
    fn remind_zone(&self, cx: &App) -> TimeZone {
        self.session
            .read(cx)
            .reminder_zone(cx)
            .unwrap_or_else(|| self.tz.clone())
    }

    /// Closes the "Remind me" presets, the focus back on the timeline.
    pub fn close_remind(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remind_menu.take().is_some() {
            self.list.remeasure();
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    /// Adds a reminder about message `id` at `preset` and closes the presets.
    pub fn remind(&mut self, id: MessageId, preset: domain::RemindPreset, cx: &mut Context<Self>) {
        self.remind_menu = None;
        self.list.remeasure();
        let now = crate::now(cx).to_zoned_datetime(self.remind_zone(cx));
        if let Some(at) = preset.at(&now) {
            self.session.update(cx, |s, cx| s.remind_about(id, at, cx));
        }
        cx.notify();
    }

    /// Opens the actions menu of message `id` (the hover button and the
    /// right click do the same).
    pub fn open_menu(&mut self, id: MessageId, cx: &mut Context<Self>) {
        self.menu = Some(id.clone());
        self.select(Some(id), cx);
    }

    /// Runs a menu action on message `id` and closes the menu. An action the
    /// message does not offer does nothing.
    pub fn run_action(
        &mut self,
        action: MessageAction,
        id: MessageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        if !self.actions(&id, cx).contains(&action) {
            cx.notify();
            return;
        }
        match action {
            MessageAction::ReplyInThread => self.open_thread(id, cx),
            MessageAction::AddReaction => self.open_picker(id, window, cx),
            MessageAction::Save | MessageAction::RemoveSaved => {
                self.set_saved(id, action == MessageAction::Save, cx)
            }
            MessageAction::Pin | MessageAction::Unpin => {
                self.set_pinned(id, action == MessageAction::Pin, cx)
            }
            MessageAction::CopyLink => {
                self.copy_link(&id, cx);
            }
            // Opens the presets with the first one focused; again, closes them.
            MessageAction::RemindMe if self.remind_menu.as_ref() == Some(&id) => {
                self.close_remind(window, cx);
            }
            MessageAction::RemindMe => {
                self.remind_menu = Some(id);
                self.list.remeasure();
                window.focus(&self.remind_first, cx);
            }
            MessageAction::MarkUnread => self.mark_unread(id, cx),
            MessageAction::Edit => {
                if let Some(m) = self.find(&id, cx) {
                    cx.emit(TimelineEvent::EditRequested {
                        id: m.id,
                        text: m.text.to_string(),
                    });
                }
            }
            MessageAction::Delete => {
                if let Some(m) = self.find(&id, cx) {
                    self.pending_delete = Some(id.clone());
                    let text = self.delete_preview(&m, cx);
                    cx.emit(TimelineEvent::DeleteRequested { id, text });
                }
            }
        }
        cx.notify();
    }

    /// Pins or unpins message `id`.
    pub fn set_pinned(&mut self, id: MessageId, pinned: bool, cx: &mut Context<Self>) {
        self.session
            .update(cx, |s, cx| s.set_pinned(id, pinned, cx));
    }

    /// Saves message `id` for later or removes it from the saved items.
    pub fn set_saved(&mut self, id: MessageId, saved: bool, cx: &mut Context<Self>) {
        self.session.update(cx, |s, cx| s.set_saved(id, saved, cx));
    }

    /// Puts the provider's link to message `id` on the clipboard; false when
    /// the provider has none.
    pub fn copy_link(&mut self, id: &MessageId, cx: &mut Context<Self>) -> bool {
        match self.session.read(cx).web_url(id) {
            Some(url) => {
                crate::toast_stack::Toasts::copy_link(url, cx);
                true
            }
            None => false,
        }
    }

    fn toggle_pin_action(&mut self, _: &TogglePin, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if let Some(m) = self.find(&id, cx) {
            let a = if m.pinned {
                MessageAction::Unpin
            } else {
                MessageAction::Pin
            };
            self.run_action(a, id, window, cx);
        }
    }

    fn toggle_saved_action(
        &mut self,
        _: &ToggleSaved,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if let Some(m) = self.find(&id, cx) {
            let a = if m.saved {
                MessageAction::RemoveSaved
            } else {
                MessageAction::Save
            };
            self.run_action(a, id, window, cx);
        }
    }

    fn copy_link_action(&mut self, _: &CopyLink, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            self.run_action(MessageAction::CopyLink, id, window, cx);
        }
    }

    fn maybe_load_older(&mut self, cx: &mut Context<Self>) {
        let Some(oldest) = self.ids.first().cloned() else {
            return;
        };
        if self.requested_older.as_ref() == Some(&oldest) {
            return;
        }
        let more = self
            .session
            .read(cx)
            .window(&self.conversation)
            .is_some_and(|w| w.has_more_before());
        if !more {
            return;
        }
        self.requested_older = Some(oldest);
        let c = self.conversation.clone();
        self.session.update(cx, |s, cx| s.load_older(c, cx));
    }

    /// Mirrors the source's window into the list, keeping the scroll position
    /// when older pages are prepended or newer messages appended.
    fn sync(&mut self, cx: &App) {
        // A detached window must not stick to its bottom: it is a place in
        // history, not the live tail.
        let mode = if self.is_detached(cx) {
            gpui::FollowMode::Normal
        } else {
            gpui::FollowMode::Tail
        };
        if self.follow_mode != mode {
            self.follow_mode = mode;
            self.list.set_follow_mode(mode);
        }
        let (new, new_keys, sig) = {
            let session = self.session.read(cx);
            match session.window(&self.conversation) {
                Some(w) => (
                    w.messages().iter().map(|m| m.id.clone()).collect(),
                    w.messages().iter().map(row_key).collect(),
                    window_signature(w.messages()),
                ),
                None => (Vec::new(), Vec::new(), 0),
            }
        };
        let old = std::mem::replace(&mut self.ids, new);
        let old_keys = std::mem::replace(&mut self.keys, new_keys);
        self.follow_confirmed(&old, &old_keys);
        let marker_was_none = self.open_marker.is_none();
        let remeasure = match id_splices(&old_keys, &self.keys) {
            IdOps::Same => sig != self.sig,
            IdOps::Reset => {
                self.list.reset(self.ids.len());
                false
            }
            IdOps::Splice(ops) => {
                for (range, count) in ops {
                    self.list.splice(range, count);
                }
                true
            }
        };
        let more_before = self
            .session
            .read(cx)
            .window(&self.conversation)
            .is_some_and(|w| w.has_more_before());
        // The start of history shows or hides the intro above the first row.
        let flipped = more_before != self.more_before;
        let changed = remeasure || sig != self.sig || self.leads.len() != self.ids.len() || flipped;
        self.more_before = more_before;
        self.sig = sig;
        self.capture_marker(cx);
        let marker_new = marker_was_none && self.open_marker.is_some();
        let day_rolled =
            self.leads_day != timeline_dividers::local_day(crate::now_secs(cx), &self.tz);
        if changed || marker_new || day_rolled {
            self.recompute_leads(cx);
        }
        // Rows are laid out again only when the window changed or something a
        // row shows did (an edit, a reaction, a delivery state), not on every
        // session notify.
        if remeasure || marker_new || day_rolled || flipped {
            self.list.remeasure();
        }
    }

    /// A message the server confirmed has a new id (its ts): the per-message
    /// state that names it (selection, open menus, hover) moves to it.
    fn follow_confirmed(&mut self, old: &[MessageId], old_keys: &[RowKey]) {
        if old == self.ids.as_slice() {
            return;
        }
        let by_key: HashMap<&RowKey, &MessageId> = old_keys.iter().zip(old).collect();
        let moved: HashMap<MessageId, MessageId> = self
            .keys
            .iter()
            .zip(&self.ids)
            .filter_map(|(k, n)| {
                by_key
                    .get(k)
                    .filter(|o| **o != n)
                    .map(|o| ((*o).clone(), n.clone()))
            })
            .collect();
        if moved.is_empty() {
            return;
        }
        for slot in [
            &mut self.selected,
            &mut self.pending_delete,
            &mut self.menu,
            &mut self.remind_menu,
            &mut self.hovered,
            &mut self.highlighted,
            &mut self.picker_target,
        ] {
            if let Some(n) = slot.as_ref().and_then(|i| moved.get(i)) {
                *slot = Some(n.clone());
            }
        }
    }

    /// Remembers the read marker the first time the conversation has messages,
    /// so the "New" divider stays where it was while the user reads.
    fn capture_marker(&mut self, cx: &App) {
        if self.open_marker.is_some() || self.ids.is_empty() {
            return;
        }
        let s = self.session.read(cx);
        if let Some(m) = s.opened_marker(&self.conversation) {
            self.open_marker = Some(m);
            return;
        }
        self.open_marker = Some(
            s.conversation(&self.conversation)
                .and_then(|c| c.unread.then(|| c.last_read.clone()).flatten()),
        );
    }

    /// Day and "New" dividers for the loaded window (once per window change).
    fn recompute_leads(&mut self, cx: &App) {
        let s = self.session.read(cx);
        let Some(w) = s.window(&self.conversation) else {
            self.leads.clear();
            return;
        };
        let marker = self.open_marker.as_ref().and_then(|m| m.as_ref());
        let now = crate::now_secs(cx);
        self.leads_day = timeline_dividers::local_day(now, &self.tz);
        // With older history unloaded the first unread message is not known
        // until the window reaches the marker.
        let reached = !w.has_more_before()
            || marker.is_some_and(|r| w.messages().first().is_some_and(|m| m.id.ts <= *r));
        self.leads = timeline_dividers::compute(
            w.messages(),
            &self.tz,
            now,
            marker.filter(|_| reached),
            s.self_user(),
        );
    }

    fn render_item(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.rendered += 1;
        if ix == 0 {
            // The oldest loaded message is on screen: the top was reached.
            let view = cx.entity();
            cx.defer(move |cx| {
                view.update(cx, |t, cx| t.maybe_load_older(cx));
            });
        }
        let theme = cx.theme().clone();
        let l = i18n::gpui::localizer(cx);
        let session_entity = self.session.clone();
        let session = self.session.read(cx);
        let Some(msgs) = session.window(&self.conversation).map(|w| w.messages()) else {
            return div().into_any_element();
        };
        let Some(m) = msgs.get(ix) else {
            return div().into_any_element();
        };
        let grouped = ix > 0 && continues(&msgs[ix - 1], m);
        let author_member = m.author.as_ref().and_then(|u| session.member(u));
        let author = author_name(author_member, &l);
        // An open thread reads as selected: the root row and its summary pill
        // show the open state (Thread board).
        let thread_open = session.thread_open(&m.id);
        let selected = thread_open
            || self.selected.as_ref() == Some(&m.id)
            || self.highlighted.as_ref() == Some(&m.id);
        let is_bot = author_member.is_some_and(|m| m.is_bot);
        let delivered = m.state == DeliveryState::Delivered;
        let id = m.id.clone();
        let id_for_pills = m.id.clone();
        let has_reactions = !m.reactions.is_empty();
        let msg_files = m.files.clone();
        let parser = session.content();
        let rich = self.cache.get_or_parse(m, parser.as_ref());
        // A pending row dims through the inherited text colour, which does not
        // reach the colours the rich text sets itself; fade them over the row's
        // ground instead (main's group opacity, one paint each).
        let queued = session.offline();
        let strength = crate::message_row::row_opacity(&m.state, queued);
        let colors = crate::message_row::rich_colors(
            RichColors::from_theme(&theme),
            &m.state,
            queued,
            theme.colors().background,
        );
        let mut rcx = RichTextCx::new(session, colors, l.clone(), self.tz.clone());
        rcx.message = Some(m.id.clone());
        rcx.clock = self.clock;
        rcx.code_font = self.code_font.clone();
        rcx.id = crate::rich_text::element_key(&m.id);
        rcx.on_open_url = self.open_url.clone();
        // Emoji and images are leaves that ignore the text colour; their own
        // opacity dims them like main's group opacity, and nothing overlaps.
        let dimmed_image =
            |src: &ImageRef, alt: &str| crate::media::image_hook_dimmed(src, alt, strength);
        let dimmed_emoji =
            |src: &ImageRef, alt: &str| crate::media::emoji_hook_dimmed(src, alt, strength);
        if cx.has_global::<crate::media::ImageCache>() {
            if strength < 1.0 {
                rcx.image = Some(&dimmed_image);
                rcx.emoji = Some(&dimmed_emoji);
            } else {
                rcx.image = Some(&crate::media::image_hook);
                rcx.emoji = Some(&crate::media::emoji_hook);
            }
        }
        let for_user = session_entity.clone();
        let failed_actions = {
            let (retry, discard) = (session_entity.clone(), session_entity.clone());
            crate::message_row::failed_actions_for(
                m,
                move |id, cx| retry.update(cx, |s, cx| s.retry_send(id, cx)),
                move |id, cx| discard.update(cx, |s, cx| s.discard_send(id, cx)),
            )
        };
        let for_author = session_entity.clone();
        let (open_c, open_u, join) = self
            .row_fns
            .get_or_insert_with(|| {
                let for_huddle = session_entity.clone();
                (
                    Rc::new(move |id, _, cx| {
                        session_entity.update(cx, |s, cx| s.open_conversation(id, cx))
                    }),
                    Rc::new(move |user, _, cx| {
                        for_user.update(cx, |s, cx| s.open_profile(user, cx))
                    }),
                    Rc::new(move |c, _, cx| for_huddle.update(cx, |s, cx| s.join_huddle(c, cx))),
                )
            })
            .clone();
        rcx.on_open_conversation = Some(open_c);
        rcx.on_open_user = Some(open_u);
        rcx.on_join_huddle = Some(join);
        let author_id = m.author.clone();
        let avatar = author_member.and_then(|mem| {
            mem.avatar_for(themes::metrics::LAYOUT_MSG_AVATAR.as_f32())
                .cloned()
        });
        let status = author_member
            .and_then(|mem| mem.status.as_ref())
            .filter(|st| domain::status_visible(st, crate::now_secs(cx)))
            .and_then(|st| st.emoji.as_ref())
            .and_then(|e| e.unicode.clone())
            .map(SharedString::from);
        let author_link = author_id.map(|u| AuthorLink {
            key: SharedString::from(m.id.ts.0.clone()),
            on_click: Rc::new(move |_, cx| {
                for_author.update(cx, |s, cx| s.open_profile(u.clone(), cx))
            }),
        });
        let mention = session.self_user().is_some_and(|me| {
            m.mentions
                .iter()
                .any(|x| matches!(x, domain::Mention::User(u) if u == me))
        });
        let row = MessageRow {
            rich: Some(render_rich_text(&rich, &rcx)),
            author,
            status,
            is_bot,
            author_link,
            time: format_time(&m.id.ts, self.clock, &self.tz, &l).into(),
            // `rich` is set, so the plain text is never shown.
            text: SharedString::default(),
            edited: m.edited.is_some(),
            state: m.state.clone(),
            queued,
            failed_actions,
            grouped,
            selected,
            ground: theme.colors().background,
            density: self.density,
            show_avatar: self.show_avatars,
            avatar,
        }
        .render(&theme, &l);
        let colors = theme.colors();
        // Every delivered message offers at least Add reaction and Mark unread,
        // so the hover button needs no per-row lookup; the full list is only
        // built for the one row whose menu is open.
        let menu_open = self.menu.as_ref() == Some(&id_for_pills);
        let actions = if menu_open {
            self.actions(&id_for_pills, cx)
        } else {
            Vec::new()
        };
        let more =
            (delivered && !menu_open && self.hovered.as_ref() == Some(&id_for_pills)).then(|| {
                let target = id_for_pills.clone();
                let ts = id_for_pills.ts.0.clone();
                div()
                    .id(("timeline-more", ix))
                    .debug_selector(move || format!("message-more-{ts}"))
                    .absolute()
                    .top(px(2.0))
                    .right(px(12.0))
                    .px_1()
                    .rounded_md()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.element_background)
                    .text_color(colors.text_muted)
                    .cursor_pointer()
                    .child("\u{22ef}")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = Some(target.clone());
                        this.select(Some(target.clone()), cx);
                    }))
            });
        let mut el = div()
            .id(("timeline-row", ix))
            .relative()
            .on_hover(cx.listener({
                let target = id_for_pills.clone();
                move |this, hovered: &bool, _, cx| {
                    // The "..." button follows the pointer: only the hovered
                    // row builds it.
                    if *hovered {
                        if this.hovered.as_ref() != Some(&target) {
                            this.hovered = Some(target.clone());
                            cx.notify();
                        }
                    } else if this.hovered.as_ref() == Some(&target) {
                        this.hovered = None;
                        cx.notify();
                    }
                }
            }))
            .debug_selector(|| format!("timeline-row-{}", id.ts.0))
            .w_full()
            // The tint spans the whole message (reactions, files, thread summary).
            .when_some(
                mention_tint(
                    mention,
                    selected,
                    parla_tokens(&theme).message_mention_background,
                ),
                |d, c| d.bg(c),
            )
            .child(row)
            .children(more)
            .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                if ev.modifiers().alt {
                    this.mark_unread(id.clone(), cx);
                } else {
                    this.select(Some(id.clone()), cx);
                }
            }))
            .on_mouse_down(
                gpui::MouseButton::Right,
                cx.listener({
                    let target = id_for_pills.clone();
                    move |this, _, _, cx| {
                        this.menu = Some(target.clone());
                        this.select(Some(target.clone()), cx);
                    }
                }),
            );
        if !msg_files.is_empty() {
            let mut cards = div()
                .pl(crate::message_row::body_indent())
                .pr(themes::metrics::SPACE_7)
                .flex()
                .flex_col();
            for (fi, f) in msg_files.iter().enumerate() {
                let expanded = self.snippets_expanded.contains(&f.id);
                let c = match self.snippet_card(f, expanded) {
                    Some(s) => files::Card::Snippet(s),
                    None => files::card_with(
                        f,
                        self.inline_images,
                        self.revealed.contains(&f.id),
                        false,
                    ),
                };
                let card_id = ("file-card", rcx.id.wrapping_mul(64).wrapping_add(fi));
                if let files::Card::Snippet(s) = &c {
                    let (this, toggle_id) = (cx.entity(), f.id.clone());
                    let (dl_this, dl_file) = (cx.entity(), f.clone());
                    let (open_this, open_file, open_msg) =
                        (cx.entity(), f.clone(), id_for_pills.clone());
                    cards = cards.child(snippet::render_snippet(
                        s,
                        &theme,
                        &l,
                        card_id,
                        self.code_font.clone(),
                        snippet::SnippetActions {
                            toggle: Box::new(move |_, cx| {
                                this.update(cx, |t, cx| t.toggle_snippet(toggle_id.clone(), cx))
                            }),
                            download: Box::new(move |_, cx| {
                                dl_this.update(cx, |t, cx| {
                                    let f = dl_file.clone();
                                    t.session.update(cx, |s, cx| s.download(&f, cx))
                                })
                            }),
                            open: Box::new(move |_, cx| {
                                open_this.update(cx, |t, cx| {
                                    t.open_code(open_file.clone(), open_msg.clone(), cx)
                                })
                            }),
                        },
                    ));
                    continue;
                }
                if let Some(ctl) = self.clips.get_mut(&f.id) {
                    self.clips_seen.insert(f.id.clone());
                    ctl.tick();
                    if ctl.take_cue_changed() {
                        // A caption of another length can change the row height.
                        cx.on_next_frame(window, |this, _, cx| {
                            this.list.remeasure();
                            cx.notify();
                        });
                    }
                    // Frames and the position move only while playing (or
                    // just after a seek while paused); otherwise nothing is
                    // requested.
                    if ctl.wants_frame() {
                        window.request_animation_frame();
                    }
                    if let Some(card) =
                        clip::render_open_clip(ctl, &self.clip_seek_focus, &theme, &l, card_id)
                        && let Some(on) = ctl.on()
                    {
                        // Texts and the handler come from the controller, so
                        // a playing frame builds and clones nothing here.
                        let (click, keys) = (on.clone(), on.clone());
                        let sel = f.id.0.clone();
                        cards = cards.child(
                            clip_card_focus(card, colors)
                                .debug_selector(move || format!("file-card-{sel}"))
                                .on_click(move |_, window, cx| {
                                    click(ClipControl::PlayPause, window, cx);
                                })
                                .on_key_down(move |ev: &gpui::KeyDownEvent, window, cx| {
                                    if let Some(k) = clip_key_of(ev)
                                        && keys(ClipControl::Key(k), window, cx)
                                    {
                                        cx.stop_propagation();
                                    }
                                }),
                        );
                        continue;
                    }
                }
                let file = f.clone();
                let msg_id = id_for_pills.clone();
                let is_clip = matches!(c, files::Card::Clip { .. });
                let (dl_this, dl_file) = (cx.entity(), f.clone());
                let mut card = match self.clips.get(&f.id) {
                    Some(ctl) if is_clip => clip::render_clip(&c, ctl.state(), &theme, &l, card_id),
                    _ => {
                        if is_clip {
                            self.clips_seen.insert(f.id.clone());
                        }
                        files::render_card(
                            &c,
                            &theme,
                            &l,
                            card_id,
                            &f.id.0.clone().into(),
                            move |_, cx| {
                                dl_this.update(cx, |t, cx| {
                                    let f = dl_file.clone();
                                    t.session.update(cx, |s, cx| s.download(&f, cx))
                                });
                            },
                        )
                    }
                };
                if is_clip {
                    // Space plays or pauses, Left and Right seek, M mutes.
                    let (kf, km) = (f.clone(), msg_id.clone());
                    card = clip_card_focus(card, colors).on_key_down(cx.listener(
                        move |this, ev: &gpui::KeyDownEvent, _, cx| {
                            if let Some(k) = clip_key_of(ev)
                                && this.clip_key(kf.clone(), km.clone(), k, cx)
                            {
                                cx.stop_propagation();
                            }
                        },
                    ));
                }
                if c != files::Card::Tombstone {
                    let sel = f.id.0.clone();
                    card = card.debug_selector(move || format!("file-card-{sel}"));
                    card = card.on_click(cx.listener(move |this, _, _, cx| {
                        this.file_click(file.clone(), msg_id.clone(), cx)
                    }));
                }
                cards = cards.child(card);
            }
            el = el.child(cards);
        }
        if self.menu.as_ref() == Some(&id_for_pills) {
            let mut strip = div()
                .id(("timeline-actions", ix))
                .debug_selector(|| "message-actions".into())
                .pl(crate::message_row::body_indent())
                .pr(themes::metrics::SPACE_7)
                .pb_1()
                .flex()
                .flex_wrap()
                .gap_3()
                .text_xs()
                .text_color(colors.text_accent);
            for a in actions {
                let target = id_for_pills.clone();
                strip = strip.child(
                    div()
                        .id((
                            "timeline-action",
                            rcx.id.wrapping_mul(16).wrapping_add(a as usize),
                        ))
                        .debug_selector(move || format!("message-action-{a:?}"))
                        .cursor_pointer()
                        .child(a.label(&l))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_action(a, target.clone(), window, cx)
                        })),
                );
            }
            el = el.child(strip);
        }
        if self.remind_menu.as_ref() == Some(&id_for_pills) {
            // "Remind me about this": the presets, each a tab stop.
            let mut strip = div()
                .id(("timeline-remind", ix))
                .debug_selector(|| "message-remind-presets".into())
                .pl(crate::message_row::body_indent())
                .pr(themes::metrics::SPACE_7)
                .pb_1()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .text_xs()
                // Escape closes the presets, the focus back on the timeline.
                .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, w, cx| {
                    if ev.keystroke.key == "escape" {
                        this.close_remind(w, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(
                    div()
                        .text_color(colors.text_muted)
                        .child(MessageAction::RemindMe.label(&l)),
                );
            let zone = self.remind_zone(cx);
            for (i, p) in domain::RemindPreset::ALL.into_iter().enumerate() {
                let target = id_for_pills.clone();
                let chip = div()
                    .id((
                        "timeline-remind-preset",
                        rcx.id.wrapping_mul(8).wrapping_add(i),
                    ))
                    .debug_selector(move || format!("message-remind-preset-{i}"))
                    .px(themes::metrics::SPACE_4)
                    .py(themes::metrics::SPACE_1)
                    .rounded(themes::metrics::RADIUS_FULL)
                    .border_1()
                    .border_color(colors.border)
                    .text_color(colors.text_accent)
                    .cursor_pointer()
                    .focus_visible(crate::controls::focus_style(colors))
                    .child(crate::reminders_view::preset_label(
                        p, &zone, self.clock, cx,
                    ));
                let chip = if i == 0 {
                    chip.track_focus(&self.remind_first)
                } else {
                    chip.tab_stop(true).focusable()
                };
                strip = strip.child(crate::controls::on_activate(
                    chip,
                    cx,
                    move |this, w, cx| {
                        this.remind(target.clone(), p, cx);
                        w.focus(&this.focus, cx);
                    },
                ));
            }
            let me = cx.entity();
            strip = strip.child(
                crate::controls::icon_button_sized(
                    ("timeline-remind-close", ix),
                    crate::icon::IconName::X,
                    t!(l, "timeline-remind-close").into_owned(),
                    crate::controls::IconButtonSize::Small,
                    colors,
                    move |w, cx| me.update(cx, |this, cx| this.close_remind(w, cx)),
                )
                .debug_selector(|| "message-remind-close".into()),
            );
            el = el.child(strip);
        }
        // Read while `m` is still borrowed from the session.
        let summary = self.thread_summary_of(m, cx);
        if has_reactions {
            let pills = self.reaction_pills_of(m, cx);
            el = el.child(pills_row(
                ix,
                &id_for_pills,
                pills,
                cx,
                Self::toggle_reaction,
                Some(Self::open_picker),
            ));
        }
        if let Some(s) = summary {
            let target = id_for_pills.clone();
            let trailing = if thread_open {
                t!(l, "thread-summary-open").into_owned()
            } else if s.latest.is_empty() {
                String::new()
            } else {
                t!(l, "thread-summary-last-reply", time = s.latest.as_str()).into_owned()
            };
            let mut stack = div()
                .debug_selector(|| "thread-summary-avatars".into())
                .flex()
                .items_center();
            for (i, a) in s.authors.iter().enumerate() {
                stack = stack.child(
                    div()
                        .when(i > 0, |d| d.ml(-themes::metrics::SPACE_3))
                        .rounded(themes::metrics::RADIUS_AVATAR)
                        // The stack's ring is the ground it sits on, not a layout border.
                        .shadow(vec![gpui::BoxShadow {
                            color: colors.background,
                            offset: gpui::point(px(0.0), px(0.0)),
                            blur_radius: px(0.0),
                            spread_radius: themes::metrics::SPACE_1,
                            inset: false,
                        }])
                        .child(crate::message_row::avatar_sized(
                            &a.name,
                            &theme,
                            themes::metrics::LAYOUT_AVATAR_THREAD,
                            a.image.as_ref(),
                        )),
                );
            }
            let pill = div()
                .id(("thread-summary", ix))
                .debug_selector(|| format!("thread-open-{}", id_for_pills.ts.0))
                .flex()
                .items_center()
                .gap(themes::metrics::SPACE_4)
                .mt(themes::metrics::SPACE_3)
                .pl(themes::metrics::SPACE_2)
                .pr(themes::metrics::SPACE_5)
                .py(themes::metrics::SPACE_2)
                .rounded(themes::metrics::RADIUS_MD)
                .border_1()
                .border_color(if thread_open {
                    colors.element_background
                } else {
                    gpui::transparent_black()
                })
                .bg(if thread_open {
                    colors.accent_soft
                } else {
                    gpui::transparent_black()
                })
                .text_size(themes::metrics::FONT_SIZE_SM)
                .when(!thread_open, |d| {
                    d.hover(|h| h.border_color(colors.border).bg(colors.surface_background))
                })
                .tab_stop(true)
                .focusable()
                .focus_visible(crate::controls::focus_style(colors))
                .child(stack)
                .child(
                    div()
                        .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                        .text_color(if thread_open {
                            parla_tokens(&theme).accent_soft_text
                        } else {
                            colors.text_accent
                        })
                        .child(s.label.clone()),
                )
                .when(!trailing.is_empty(), |d| {
                    d.child(
                        div()
                            .when(thread_open, |d| {
                                d.debug_selector(|| "thread-summary-open".into())
                            })
                            .text_size(themes::metrics::FONT_SIZE_XS)
                            .text_color(parla_tokens(&theme).text_subtle)
                            .child(trailing),
                    )
                });
            el = el.child(
                div()
                    .pl(crate::message_row::body_indent())
                    .pr(themes::metrics::SPACE_7)
                    .pb_1()
                    .child(crate::controls::on_activate(
                        pill,
                        cx,
                        move |this, _, cx| this.open_thread(target.clone(), cx),
                    )),
            );
        }
        let lead = self.leads.get(ix).copied().unwrap_or_default();
        // The intro needs the first page in: a live message that arrives
        // before it is not the start of history.
        let intro = if ix == 0
            && !self.more_before
            && self
                .session
                .read(cx)
                .window(&self.conversation)
                .is_some_and(|w| w.is_loaded())
        {
            self.render_intro(cx)
        } else {
            None
        };
        if lead.day.is_none() && !lead.new && intro.is_none() {
            return el.into_any_element();
        }
        let mut col = div().w_full().flex().flex_col().children(intro);
        if let Some(day) = lead.day {
            col = col.child(timeline_dividers::render_day(
                timeline_dividers::day_text(day, &self.tz, &l),
                &theme,
            ));
        }
        if lead.new {
            col = col.child(timeline_dividers::render_new(&l, &theme));
        }
        col.child(el).into_any_element()
    }
}

impl<S: TimelineSource + 'static> Focusable for Timeline<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: TimelineSource + 'static> Timeline<S> {
    /// The intro's name, line and the peer for View profile; `None` for
    /// channels. Named like the header when the source has the workspace.
    fn intro_subject(&self, cx: &App) -> Option<(String, String, Option<UserId>)> {
        let s = self.session.read(cx);
        let c = s.conversation(&self.conversation)?;
        if !matches!(
            c.kind,
            domain::ConversationKind::Direct | domain::ConversationKind::Group
        ) {
            return None;
        }
        let l = i18n::gpui::localizer(cx);
        let (name, line) = match s.workspace_state() {
            Some(state) => intro_copy(state, c, &l),
            None => {
                let name = c.name.to_string();
                let line = t!(l, "timeline-intro", name = name.clone()).into_owned();
                (name, line)
            }
        };
        let own = c.peer.is_some() && c.peer.as_ref() == s.self_user();
        let peer = c.peer.clone().filter(|_| !own);
        Some((name, line, peer))
    }

    /// The start of a direct message or group: avatar, name, a line, and View
    /// profile (a direct message) and Share a file (while signed in). Adding
    /// people needs a provider action, which none has yet, so there is no such
    /// button.
    fn render_intro(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (name, line, peer) = self.intro_subject(cx)?;
        let picture = peer
            .as_ref()
            .and_then(|u| self.session.read(cx).member(u))
            .and_then(|m| {
                m.avatar_for(themes::metrics::LAYOUT_AVATAR_LG.as_f32())
                    .cloned()
            });
        let expired = self.session.read(cx).session_expired();
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let l = i18n::gpui::localizer(cx);
        let button = |id: &'static str, label: String| {
            crate::controls::no_focus_on_click(
                crate::controls::button_small(
                    id,
                    label,
                    crate::controls::ButtonKind::Secondary,
                    &colors,
                )
                .debug_selector(move || id.to_string()),
            )
        };
        let mut buttons = div()
            .flex()
            .gap(themes::metrics::SPACE_4)
            .mt(themes::metrics::SPACE_5);
        if let Some(user) = peer {
            buttons = buttons.child(crate::controls::on_activate(
                button(
                    "timeline-intro-profile",
                    t!(l, "timeline-intro-profile").into_owned(),
                ),
                cx,
                move |this, _, cx| {
                    let u = user.clone();
                    this.session.update(cx, |s, cx| s.open_profile(u, cx));
                },
            ));
        }
        if !expired {
            buttons = buttons.child(crate::controls::on_activate(
                button(
                    "timeline-intro-share",
                    t!(l, "timeline-intro-share").into_owned(),
                ),
                cx,
                |this, _, cx| this.attach(cx),
            ));
        }
        Some(
            div()
                .debug_selector(|| "timeline-intro".into())
                .flex()
                .flex_col()
                .items_start()
                .px(themes::metrics::SPACE_7)
                .pt(themes::metrics::SPACE_8)
                .pb(themes::metrics::SPACE_8)
                .child(crate::message_row::avatar_sized(
                    &name,
                    &theme,
                    themes::metrics::LAYOUT_AVATAR_LG,
                    picture.as_ref(),
                ))
                .child(
                    div()
                        .mt(themes::metrics::SPACE_5)
                        .text_size(themes::metrics::FONT_SIZE_2XL)
                        .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                        .text_color(colors.text)
                        .child(name),
                )
                .child(
                    div()
                        .mt(themes::metrics::SPACE_2)
                        .text_size(themes::metrics::FONT_SIZE_SM)
                        .text_color(colors.text_muted)
                        .child(line),
                )
                .child(buttons)
                .into_any_element(),
        )
    }

    /// The file toast and attach refusals, under the list.
    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let error = cx.theme().status().error;
        let l = i18n::gpui::localizer(cx);
        let notice = self.session.read(cx).file_notice();
        // In flow under the list (not an overlay) so it never covers the last row.
        let mut col = div()
            .debug_selector(|| "timeline-footer".into())
            .w_full()
            .flex_none()
            .px_3()
            .text_xs();
        if let Some(n) = self.upload_notice(&l) {
            col = col.child(div().text_color(error).child(n));
        }
        match notice {
            Some(FileNotice::Downloaded(path)) => {
                let shown = path.clone();
                col = col.child(
                    div()
                        .flex()
                        .gap_3()
                        .text_color(colors.text)
                        .child(
                            t!(
                                l,
                                "file-saved",
                                name = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy())
                                    .unwrap_or_default()
                                    .as_ref()
                            )
                            .into_owned(),
                        )
                        .child(
                            div()
                                .id("file-reveal")
                                .text_color(colors.text_accent)
                                .child(t!(l, "file-show-in-folder").into_owned())
                                .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&shown))),
                        )
                        .child(
                            div()
                                .id("file-notice-dismiss")
                                .child(t!(l, "file-dismiss").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session.update(cx, |s, cx| s.dismiss_file_notice(cx))
                                })),
                        ),
                );
            }
            Some(FileNotice::Error(e)) => {
                col = col.child(
                    div()
                        .flex()
                        .gap_3()
                        .text_color(error)
                        .child(l.text(&e).into_owned())
                        .child(
                            div()
                                .id("file-notice-dismiss")
                                .child(t!(l, "file-dismiss").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.session.update(cx, |s, cx| s.dismiss_file_notice(cx))
                                })),
                        ),
                );
            }
            None => {}
        }
        col
    }
}

impl<S: TimelineSource + 'static> Render for Timeline<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation.is_none() {
            self.window_active = window.is_window_active();
            self.activation = Some(cx.observe_window_activation(window, |this, window, cx| {
                this.window_active = window.is_window_active();
                if !this.window_active {
                    for c in this.clips.values_mut() {
                        c.window_inactive();
                    }
                }
                this.report_focus(cx);
            }));
            self.schedule_focus_report(cx);
        }
        // Scrolling back to the bottom re-engages the tail while laying out
        // this frame, so look again once it is done.
        if !self.list.is_following_tail() {
            self.schedule_focus_report(cx);
        }
        self.rendered = 0;
        self.sweep_clips();
        // The board's timeline ground is `bg.app`: the header and tabs keep the
        // surface, so the two planes read apart (Main board).
        let app_bg = cx.theme().colors().background;
        let detached = self.is_detached(cx).then(|| {
            let l = i18n::gpui::localizer(cx);
            t!(l, "timeline-jump-to-latest").into_owned()
        });
        // History arrived and holds nothing: say so, instead of a blank pane
        // (a direct message or group shows its intro). Still loading, the pane
        // stays blank.
        let loaded_empty = self
            .session
            .read(cx)
            .window(&self.conversation)
            .is_some_and(|w| w.is_loaded() && w.is_empty() && !w.has_more_before());
        let empty_intro = if loaded_empty {
            self.render_intro(cx)
        } else {
            None
        };
        let empty = (empty_intro.is_none()
            && self
                .session
                .read(cx)
                .window(&self.conversation)
                .is_some_and(|w| w.is_loaded() && w.is_empty()))
        .then(|| {
            let l = i18n::gpui::localizer(cx);
            (
                t!(l, "timeline-empty").into_owned(),
                cx.theme().colors().text_muted,
            )
        });
        // The first page is still in flight (a skeleton) or failed (Retry);
        // either way the pane says so instead of staying blank.
        let first_page_pending = {
            let src = self.session.read(cx);
            src.window(&self.conversation)
                .is_none_or(|w| !w.is_loaded() && w.is_empty())
        };
        let pending = first_page_pending.then(|| {
            let failed = self.session.read(cx).history_failed(&self.conversation);
            let l = i18n::gpui::localizer(cx);
            if failed {
                let conv = self.conversation.clone();
                crate::load_state::load_error(
                    "timeline-load-error",
                    "timeline-retry",
                    t!(l, "timeline-load-error-title").into_owned(),
                    t!(l, "timeline-load-error-hint").into_owned(),
                    t!(l, "load-retry").into_owned(),
                    cx,
                    move |this: &mut Self, _, cx| {
                        let c = conv.clone();
                        this.session.update(cx, |s, cx| s.retry_history(c, cx));
                    },
                )
            } else {
                crate::load_state::skeleton(
                    "timeline-loading",
                    3,
                    t!(l, "timeline-loading").into_owned(),
                    cx,
                )
                .into_any_element()
            }
        });
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::edit_message))
            .on_action(cx.listener(Self::delete_message))
            .on_action(cx.listener(Self::add_reaction))
            .on_action(cx.listener(Self::open_thread_action))
            .on_action(cx.listener(Self::mark_unread_action))
            .on_action(cx.listener(Self::jump_to_latest_action))
            .on_action(cx.listener(Self::toggle_pin_action))
            .on_action(cx.listener(Self::toggle_saved_action))
            .on_action(cx.listener(Self::copy_link_action))
            .size_full()
            .flex()
            .flex_col()
            .bg(app_bg)
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.drop_paths(paths.paths().to_vec(), cx)
            }))
            .child(
                div()
                    .debug_selector(|| "timeline-list".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .relative()
                    .child(list(self.list.clone(), cx.processor(Self::render_item)).size_full())
                    .when_some(pending, |d, el| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .bg(app_bg)
                                .child(el),
                        )
                    })
                    .when_some(empty_intro, |d, intro| {
                        d.child(div().absolute().bottom_0().left_0().w_full().child(intro))
                    })
                    .when_some(empty, |d, (text, color)| {
                        d.child(
                            div()
                                .debug_selector(|| "timeline-empty".into())
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(color)
                                .child(text),
                        )
                    }),
            )
            .when_some(detached, |d, text| {
                let colors = *cx.theme().colors();
                d.child(
                    div()
                        .id("timeline-jump-to-latest")
                        .debug_selector(|| "timeline-jump-to-latest".into())
                        .flex_none()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .cursor_pointer()
                        .bg(colors.element_selected)
                        .text_color(colors.text)
                        .child(text)
                        .on_click(cx.listener(|this, _, _, cx| this.jump_to_latest(cx))),
                )
            })
            .child(self.render_footer(cx))
            .when_some(self.picker.clone(), |d, p| {
                d.child(div().absolute().bottom_4().right_4().child(p))
            })
    }
}

#[cfg(test)]
mod sync_tests {
    use super::*;

    fn ids(ts: &[&str]) -> Vec<MessageId> {
        ts.iter()
            .map(|t| MessageId {
                conversation: ConversationId("c".into()),
                ts: MessageTs((*t).into()),
            })
            .collect()
    }

    #[test]
    fn a_confirmed_send_updates_its_row_in_place() {
        let c = |t: &str| {
            let cm = domain::ClientMsgId("k".into());
            let mut m = Message::new(
                MessageId {
                    conversation: ConversationId("c".into()),
                    ts: MessageTs(t.into()),
                },
                None,
                "x".into(),
            );
            m.client_msg_id = Some(cm);
            m
        };
        let keys = |m: &Message| vec![row_key(m)];
        // Only row: the ts changes, the key does not, so no reset or splice.
        assert_eq!(
            id_splices(&keys(&c("1.000001")), &keys(&c("2.000001"))),
            IdOps::Same
        );
    }

    #[test]
    fn unchanged_window_is_a_no_op() {
        assert_eq!(
            id_splices(&ids(&["1", "2"]), &ids(&["1", "2"])),
            IdOps::Same
        );
    }

    #[test]
    fn prepend_and_append_keep_scroll() {
        assert_eq!(
            id_splices(&ids(&["2", "3"]), &ids(&["0", "1", "2", "3", "4"])),
            IdOps::Splice(vec![(0..0, 2), (4..4, 1)])
        );
    }

    #[test]
    fn mid_window_delete_splices_the_removed_row() {
        assert_eq!(
            id_splices(&ids(&["1", "2", "3", "4"]), &ids(&["1", "3", "4"])),
            IdOps::Splice(vec![(1..2, 0)])
        );
    }

    #[test]
    fn top_trim_with_append_splices_instead_of_resetting() {
        assert_eq!(
            id_splices(&ids(&["1", "2", "3"]), &ids(&["2", "3", "4"])),
            IdOps::Splice(vec![(0..1, 0), (2..2, 1)])
        );
    }

    #[test]
    fn unrelated_windows_reset() {
        assert_eq!(
            id_splices(&ids(&["1", "2"]), &ids(&["8", "9"])),
            IdOps::Reset
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mentioned_message_is_tinted_unless_selected() {
        let tint = 7_u8;
        assert_eq!(mention_tint(true, false, tint), Some(tint));
        assert_eq!(mention_tint(true, true, tint), None);
        assert_eq!(mention_tint(false, false, tint), None);
    }
}
