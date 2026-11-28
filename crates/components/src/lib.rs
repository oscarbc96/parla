//! Provider-neutral UI components for Parla.

// The catalog is embedded from `i18n/*.ftl`; `t!` is defined here for every module.
i18n::namespace!("components");

pub mod a11y;
pub mod aggregate_views;
pub mod audit_log;
pub mod badge;
pub mod bookmarks_bar;
pub mod call_bar;
pub mod call_card;
pub mod channel_admin;
pub mod channel_details;
pub mod clip;
pub mod code_sign_in;
pub mod code_viewer;
pub mod command_palette;
pub mod completion;
pub mod composer;
pub mod confirm_write;
pub mod controls;
pub mod conversation_menu;
pub mod datetime_picker;
pub mod density;
pub mod dialog;
pub mod dnd_menu;
pub mod drafts_view;
pub mod emoji_picker;
pub mod files;
pub mod files_tab;
pub mod format_toolbar;
pub mod gif_picker;
pub mod header;
pub mod huddle_bar;
pub mod icon;
pub mod keycap;
pub(crate) mod keys;
pub mod links;
pub mod live_settings;
pub mod load_state;
pub mod logo;
pub mod media;
pub mod media_viewer;
pub mod message_actions;
pub mod message_row;
pub mod nav_stack;
pub mod notify_menu;
pub mod onboarding;
pub mod people_directory;
pub mod picker;
pub mod pins_popover;
pub mod platform;
pub mod popover;
pub mod profile_panel;
pub(crate) mod pump;
pub mod quick_switcher;
pub mod rail;
pub mod reactions;
pub mod reminders_view;
pub mod render_cache;
pub mod rich_text;
pub mod ring_banner;
pub mod saved_view;
pub mod scheduled_view;
pub mod search_panel;
pub mod send_later;
pub mod settings_controls;
pub mod shortcuts_sheet;
pub mod sidebar;
pub mod sidebar_sections;
pub mod sign_in;
pub mod slash_menu;
pub mod snippet;
pub mod snippet_highlight;
pub mod status_dialog;
pub mod text_input;
pub mod thread_panel;
pub mod thread_summary;
pub mod threads_view;
pub mod timeline;
pub mod timeline_dividers;
pub mod toast_stack;
pub mod tour;
pub mod upload;
pub mod vtt;
pub mod workspace_picker;

pub use aggregate_views::{
    AggregateEntry, AggregateEvent, AggregateKind, AggregateSource, AggregateTarget, AggregateView,
    AllMentions, AllScheduled, AllUnreads, MentionItem, MentionLine, PAGE, ScheduledItem,
    UnreadItem, WorkspaceTag,
};
pub use audit_log::{AuditError, AuditLogView, AuditNames, ClearAuditLog};
pub use badge::{workspace_badge, workspace_badge_sized};
pub use bookmarks_bar::{BookmarksBar, Chip};
pub use call_bar::{
    CallAvatar, CallBar, CallBarEvent, CallBarLayout, CallBarModel, CallBarPhase, CallPerson,
    DeviceMenu, DeviceMenuView, DeviceRow, MediaState, MuteControl,
};
pub use call_card::{CallAction, CallNode};
pub use channel_admin::{
    BROWSE_DEBOUNCE, BrowseRow, BrowseRowText, ChannelBrowser, ChannelBrowserEvent, ConfirmDialog,
    ConfirmEvent, CreateChannelDialog, CreateChannelEvent, EditChannelDialog, EditChannelEvent,
    EditField, InviteDialog, InviteEvent, InviteRow, NameCheck,
};
pub use channel_details::{
    ChannelDetailsEvent, ChannelDetailsPanel, ChannelDetailsSource, DetailsAbout, DetailsTab,
    MemberList, MemberRow, conversation_people, filter_members, member_rows, recent_people,
};
pub use command_palette::{
    CommandDelegate, CommandItem, CommandPalette, is_palette_action, palette_action_names,
};
pub use completion::{Completion, CompletionKind};
pub use composer::{
    Cancel, Composer, ComposerEvent, ComposerSource, EditLast, NewSnippet, Newline, Send,
    ToggleBroadcast,
};
pub use confirm_write::{
    ApproveWrite, ApproveWriteForSession, ConfirmWriteEvent, ConfirmWriteView, DeclineWrite,
};
pub use conversation_menu::{level_label, level_mark};
pub use datetime_picker::{CustomCheck, check_custom};
pub use dnd_menu::{
    PauseChoice, PauseRow, minutes_until_tomorrow, parse_custom_minutes, pause_rows,
};
pub use drafts_view::{DraftItem, DraftKind, DraftRow, DraftsEvent, DraftsSource, DraftsView};
pub use emoji_picker::{
    Confirm as PickerConfirm, Dismiss as PickerDismiss, EmojiPicked, EmojiPicker, PickerDismissed,
    PickerTab, SelectLeft, SelectRight, fuzzy_score,
};
pub use files::{FileNotice, StagedFile, StagedSnippet, StagedState};
pub use files_tab::{FileOpen, FilesDone, FilesLoad, FilesSource, FilesTab};
pub use header::{
    ConversationTab, HeaderActions, HeaderInfo, NotifyMenuFn, PinsFn, TabFn, ToggleFn,
    conversation_header, dm_presence, header_info, menu_anchor,
};
pub use huddle_bar::{BarView, HuddleAction, HuddleBar, HuddleSource, huddle_action};
pub use icon::{Icon, IconName};
pub use links::{LinkAction, open_link};
pub use message_actions::{MessageAction, actions_for};
pub use message_row::{
    AuthorLink, GROUP_WINDOW_SECS, MessageRow, PREVIEW_CHARS, PreviewLine, author_name,
    format_date_time, format_time, group_flags, preview_line, preview_text, user_name,
};
pub use notify_menu::{
    ExtraAction as ExtraActionFn, MenuExtra, MenuKind, NotifyControls, NotifyMenu, display_binding,
    paused_text,
};
pub use people_directory::{
    CARDS_PER_ROW, PeopleDirectory, PeopleEvent, PeopleFilter, PeopleFilters, PeopleIndex,
    PeopleSource, PersonEntry,
};
pub use picker::{MAX_ROWS, PasteOutcome, Picker, PickerDelegate, PickerEvent};
pub use pins_popover::PinsPopover;
pub use profile_panel::{
    ProfileModel, ProfilePanel, ProfilePanelEvent, ProfileSource, local_time_line, next_presence,
    profile_model,
};
pub use quick_switcher::{
    GoBack, GoForward, QuickSwitch, QuickSwitcher, SwitchDelegate, SwitchItem, SwitchScope,
    SwitchTarget, rank,
};
pub use reactions::{
    MAX_TOOLTIP_NAMES, Pill, glyph, pill_image, pill_label, tooltip_text, tooltip_view,
};
pub use reminders_view::{
    ReminderGroups, ReminderRow, RemindersEvent, RemindersSource, RemindersView, preset_label,
};
pub use render_cache::RenderCache;
pub use rich_text::{
    ImageHook, Node, OpenConversation, OpenUrl, OpenUser, RichColors, RichTextCx, Role, Span,
    TextNode, break_long_tokens, build_nodes, render_rich_text,
};
pub use ring_banner::{RingBannerData, ring_banner};
pub use saved_view::{CollectionEvent, CollectionsSource, Preview, SavedRow, SavedView, preview};
pub use scheduled_view::{DayGroup, ScheduledEvent, ScheduledRow, ScheduledSource, ScheduledView};
pub use search_panel::{
    CONTEXT_AROUND, Search, SearchPanel, SearchPanelEvent, SearchResults, SearchRow, SearchScope,
    SearchSource, SearchTarget, context_around, highlight_spans, resolve_user_ids, results_label,
    search_context_width,
};
pub use send_later::{SendChoice, SendLaterConfig, SendLaterRow, needs_local, preset_choices};
pub use sidebar::{
    DraggedRow, DraggedSection, MemberIndex, NextUnread, Open, SelectNext, SelectPrev, Sidebar,
    SidebarEvent, SidebarOption, SidebarRow, SidebarSection, SidebarSource, build_rows,
    build_rows_indexed, build_rows_pinned, build_rows_sections, build_rows_with,
    conversation_label, header_chevron, next_unread,
};
pub use sidebar_sections::{SECTION_NAME_MAX, SectionOp, apply_section_op, section_of};
pub use status_dialog::{
    ExpiryChoice, ExpiryRow, StatusDialog, StatusDialogEvent, StatusPreset, expiry_at, expiry_rows,
    status_presets,
};
pub use thread_panel::{ClosePanel, ThreadPanel, ThreadPanelEvent, ThreadSource};
pub use thread_summary::{ThreadSummary, relative_time};
pub use threads_view::{ThreadRow, ThreadsView};
pub use timeline::{
    AddReaction, CopyLink, DeleteMessage, EditMessage, MarkUnread, OpenThread, Timeline,
    TimelineEvent, TimelineSource, TogglePin, ToggleSaved,
};
pub use toast_stack::{
    MAX_QUEUED_TOASTS, MAX_VISIBLE_TOASTS, Shown, TOAST_ACTION_MIN, TOAST_ERROR_TIMEOUT,
    TOAST_TIMEOUT, Toast, ToastId, ToastKind, ToastStack, Toasts,
};

/// The clock UI code reads. Defaults to [`domain::SystemNow`]; visual tests
/// install a fixed one so relative times do not drift.
pub struct NowGlobal(pub std::sync::Arc<dyn domain::Now>);

impl gpui::Global for NowGlobal {}

/// Whether time-driven UI (GIF frames, timers) runs or holds still.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Motion {
    #[default]
    Live,
    /// First frame only, nothing scheduled: for deterministic screenshots.
    Frozen,
}

impl gpui::Global for Motion {}

/// The current time from the installed clock.
pub fn now(cx: &gpui::App) -> jiff::Timestamp {
    match cx.try_global::<NowGlobal>() {
        Some(g) => g.0.now(),
        None => domain::Now::now(&domain::SystemNow),
    }
}

/// Seconds since the Unix epoch from the installed clock.
pub fn now_secs(cx: &gpui::App) -> i64 {
    now(cx).as_second()
}

/// The time zone UI code formats times in. Defaults to the system zone; visual
/// tests install a fixed one so baselines do not depend on the machine.
pub struct TimeZoneGlobal(pub jiff::tz::TimeZone);

impl gpui::Global for TimeZoneGlobal {}

/// The installed time zone, or the system's.
pub fn time_zone(cx: &gpui::App) -> jiff::tz::TimeZone {
    cx.try_global::<TimeZoneGlobal>()
        .map_or_else(jiff::tz::TimeZone::system, |g| g.0.clone())
}

/// Installs a time zone.
pub fn set_time_zone(cx: &mut gpui::App, tz: jiff::tz::TimeZone) {
    cx.set_global(TimeZoneGlobal(tz));
}

/// The current motion mode.
pub fn motion(cx: &gpui::App) -> Motion {
    cx.try_global::<Motion>().copied().unwrap_or_default()
}

/// Sets the motion mode.
pub fn set_motion(cx: &mut gpui::App, m: Motion) {
    cx.set_global(m);
}

/// Installs a clock.
pub fn set_now(cx: &mut gpui::App, clock: std::sync::Arc<dyn domain::Now>) {
    cx.set_global(NowGlobal(clock));
}

/// The components catalog, for the app's `i18n::gpui::init`.
pub fn catalog() -> &'static i18n::Catalog {
    &CATALOG
}

/// Registers the default key bindings of every component. Also installs a
/// source-language localizer when the host has not set one up (tests, tools);
/// the app installs its own, with the user's language, before calling this.
pub fn init(cx: &mut gpui::App) {
    if !cx.has_global::<i18n::gpui::I18n>() {
        let source = i18n::LocaleSet::embedded().source.clone();
        i18n::gpui::init(cx, &[catalog()], i18n::LocalePref::Code(source));
    }
    // GPUI Base primitives (popover, dialog, focus trap) and the theme they
    // read. The mapping is refreshed by every `themes::apply`.
    gpui_base::init(cx);
    themes::base_theme::install(cx);
    composer::init(cx);
    text_input::init(cx);
    emoji_picker::init(cx);
    gif_picker::init(cx);
    confirm_write::init(cx);
    sidebar::init(cx);
}
